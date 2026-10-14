//! Networks and their connected containers.
//!
//! Endpoints exist only while containers run, so besides network events,
//! container starts and stops refresh the list too. The list call is cheap
//! (no `/system/df`), but bursts are still coalesced into one refresh. A
//! change that lands while a list is in flight may be missing from it, so it
//! schedules a follow-up refresh instead of being dropped.

use std::time::Duration;

use berth_docker::{EngineKind, Network};
use gpui::Context;

use super::{Connection, Op, Store, StoreEvent};

const REFRESH_DEBOUNCE: Duration = Duration::from_millis(300);

/// Refresh state shared with storage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum Refresh {
    #[default]
    Idle,
    /// Waiting out the debounce; later changes are still picked up.
    Queued,
    /// Talking to the engine. `stale`: something changed meanwhile, so the
    /// result may be outdated.
    Listing { stale: bool },
}

#[derive(Default)]
pub struct Networks {
    pub(super) list: Vec<Network>,
    /// The first list has arrived.
    pub(super) loaded: bool,
    refresh: Refresh,
}

impl Networks {
    /// Forget an in-flight refresh; its engine is gone.
    pub(super) const fn cancel_refresh(&mut self) {
        self.refresh = Refresh::Idle;
    }
}

impl Store {
    pub fn networks(&self) -> &[Network] {
        &self.networks.list
    }

    pub const fn networks_loaded(&self) -> bool {
        self.networks.loaded
    }

    /// Built-in names differ between Docker and Podman.
    const fn engine_kind(&self) -> EngineKind {
        match &self.connection {
            Connection::Connected(info) => info.kind,
            Connection::Connecting | Connection::Disconnected { .. } => EngineKind::Docker,
        }
    }

    pub fn network_builtin(&self, network: &Network) -> bool {
        network.is_builtin(self.engine_kind())
    }

    /// Has local endpoints, or is a Swarm network that services on other
    /// nodes may use without any local endpoint.
    pub fn network_in_use(network: &Network) -> bool {
        !network.endpoints.is_empty() || network.is_swarm()
    }

    /// What `network prune` removes.
    pub fn unused_networks(&self) -> impl Iterator<Item = &Network> {
        let kind = self.engine_kind();
        self.networks
            .list
            .iter()
            .filter(move |n| n.is_prunable(kind))
    }

    /// Reload the network list. Coalesces bursts.
    pub fn refresh_networks(&mut self, cx: &mut Context<Self>) {
        match self.networks.refresh {
            Refresh::Idle => {}
            Refresh::Queued => return,
            Refresh::Listing { .. } => {
                self.networks.refresh = Refresh::Listing { stale: true };
                return;
            }
        }
        let Some(engine) = self.engine.clone() else {
            return;
        };
        self.networks.refresh = Refresh::Queued;
        let generation = self.generation;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(REFRESH_DEBOUNCE).await;
            let current = this.update(cx, |store, _| {
                let current = store.is_current(generation);
                if current {
                    store.networks.refresh = Refresh::Listing { stale: false };
                }
                current
            });
            if !matches!(current, Ok(true)) {
                return;
            }
            let result = engine.list_networks().await;
            this.update(cx, |store, cx| {
                if !store.is_current(generation) {
                    return;
                }
                let networks = &mut store.networks;
                let stale = networks.refresh == Refresh::Listing { stale: true };
                networks.refresh = Refresh::Idle;
                match result {
                    // An outdated list could resurrect a removed network or
                    // hide a new endpoint; keep the current one until the
                    // follow-up arrives (unless there is nothing yet).
                    Ok(_) if stale && networks.loaded => {}
                    Ok(list) => {
                        networks.list = list;
                        networks.loaded = true;
                    }
                    Err(err) => log::warn!("network refresh failed: {err}"),
                }
                if stale {
                    store.refresh_networks(cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Remove networks by ID. Built-in and in-use networks fail with the
    /// engine's message.
    pub fn remove_networks(&mut self, ids: Vec<String>, cx: &mut Context<Self>) {
        let Some(engine) = self.engine.clone() else {
            return Self::not_connected("remove networks", cx);
        };
        for id in ids {
            if self.pending.contains_key(&id) {
                continue;
            }
            self.pending.insert(id.clone(), Op::Remove);
            let call = engine.remove_network(&id);
            let generation = self.generation;
            cx.spawn(async move |this, cx| {
                let result = call.await;
                this.update(cx, |store, cx| {
                    if !store.is_current(generation) {
                        return;
                    }
                    store.pending.remove(&id);
                    match result {
                        Ok(()) => store.networks.list.retain(|n| n.id != id),
                        Err(err) => {
                            let name = store
                                .networks
                                .list
                                .iter()
                                .find(|n| n.id == id)
                                .map_or_else(|| id.clone(), |n| n.name.clone());
                            cx.emit(StoreEvent::Error {
                                title: format!("Couldn't remove {name}"),
                                detail: err.to_string(),
                            });
                        }
                    }
                    store.refresh_networks(cx);
                    cx.notify();
                })
                .ok();
            })
            .detach();
        }
        cx.notify();
    }
}
