//! Images, volumes, disk usage and clean-up.
//!
//! Lists are cheap and loaded first; `/system/df` (sizes, usage counts) is
//! slow and merged in afterwards. Engine events about images/volumes — and
//! containers appearing or disappearing, which changes usage — trigger a
//! coalesced refresh rather than per-object inspects.

use std::time::Duration;

use berth_docker::{Container, Image, ImagePrune, MountKind, PruneReport, Volume};
use gpui::Context;

use super::{Op, Store, StoreEvent};
use crate::format;

/// Coalesces bursts of events (e.g. `compose down`) into one refresh.
const REFRESH_DEBOUNCE: Duration = Duration::from_millis(300);

/// Something the clean-up page can prune.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PruneTarget {
    StoppedContainers,
    DanglingImages,
    UnusedImages,
    UnusedVolumes,
    UnusedNetworks,
    BuildCache,
}

impl PruneTarget {
    pub const ALL: [Self; 6] = [
        Self::StoppedContainers,
        Self::DanglingImages,
        Self::UnusedImages,
        Self::UnusedVolumes,
        Self::UnusedNetworks,
        Self::BuildCache,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::StoppedContainers => "Stopped containers",
            Self::DanglingImages => "Dangling images",
            Self::UnusedImages => "Unused images",
            Self::UnusedVolumes => "Unused volumes",
            Self::UnusedNetworks => "Unused networks",
            Self::BuildCache => "Build cache",
        }
    }
}

/// The storage half of [`Store`]'s state.
#[derive(Default)]
pub struct Storage {
    pub(super) images: Vec<Image>,
    pub(super) volumes: Vec<Volume>,
    pub(super) build_cache_reclaimable: Option<u64>,
    /// Sizes and usage counts from `/system/df` have arrived.
    pub(super) usage_known: bool,
    pub(super) refresh_queued: bool,
    pub(super) pruning: bool,
}

impl Store {
    pub fn images(&self) -> &[Image] {
        &self.storage.images
    }

    pub fn volumes(&self) -> &[Volume] {
        &self.storage.volumes
    }

    pub const fn usage_known(&self) -> bool {
        self.storage.usage_known
    }

    pub const fn pruning(&self) -> bool {
        self.storage.pruning
    }

    /// Containers (running or not) created from `image`.
    pub fn image_users(&self, image: &Image) -> Vec<&Container> {
        self.containers
            .iter()
            .filter(|c| c.image == image.id || image.repo_tags.contains(&c.image))
            .collect()
    }

    pub fn image_in_use(&self, image: &Image) -> bool {
        image
            .containers
            .map_or_else(|| !self.image_users(image).is_empty(), |n| n > 0)
    }

    /// Containers mounting the named volume.
    pub fn volume_users(&self, name: &str) -> Vec<&Container> {
        self.containers
            .iter()
            .filter(|c| {
                c.mounts
                    .iter()
                    .any(|m| m.kind == MountKind::Volume && m.source == name)
            })
            .collect()
    }

    pub fn volume_in_use(&self, volume: &Volume) -> bool {
        volume
            .ref_count
            .map_or_else(|| !self.volume_users(&volume.name).is_empty(), |n| n > 0)
    }

    /// Reclaimable bytes for a clean-up category, when known.
    pub fn reclaimable(&self, target: PruneTarget) -> Option<u64> {
        let images = |dangling: bool| -> u64 {
            self.storage
                .images
                .iter()
                .filter(|i| i.is_dangling() == dangling && !self.image_in_use(i))
                .map(|i| i.size)
                .sum()
        };
        match target {
            PruneTarget::DanglingImages => Some(images(true)),
            // Pruning "all unused" also takes the dangling ones.
            PruneTarget::UnusedImages => Some(images(false).saturating_add(images(true))),
            PruneTarget::UnusedVolumes => self.storage.usage_known.then(|| {
                self.storage
                    .volumes
                    .iter()
                    .filter(|v| !self.volume_in_use(v))
                    .filter_map(|v| v.size)
                    .sum()
            }),
            PruneTarget::BuildCache => self.storage.build_cache_reclaimable,
            PruneTarget::StoppedContainers | PruneTarget::UnusedNetworks => None,
        }
    }

    /// How many objects a clean-up category would remove, when known.
    pub fn prune_count(&self, target: PruneTarget) -> Option<usize> {
        match target {
            PruneTarget::StoppedContainers => Some(
                self.containers
                    .iter()
                    .filter(|c| !c.state.is_active())
                    .count(),
            ),
            PruneTarget::DanglingImages => Some(
                self.storage
                    .images
                    .iter()
                    .filter(|i| i.is_dangling() && !self.image_in_use(i))
                    .count(),
            ),
            PruneTarget::UnusedImages => Some(
                self.storage
                    .images
                    .iter()
                    .filter(|i| !self.image_in_use(i))
                    .count(),
            ),
            PruneTarget::UnusedVolumes => Some(
                self.storage
                    .volumes
                    .iter()
                    .filter(|v| !self.volume_in_use(v))
                    .count(),
            ),
            PruneTarget::UnusedNetworks | PruneTarget::BuildCache => None,
        }
    }

    /// Reload image/volume lists, then disk usage. Coalesces bursts.
    pub fn refresh_storage(&mut self, cx: &mut Context<Self>) {
        if self.storage.refresh_queued {
            return;
        }
        let Some(engine) = self.engine.clone() else {
            return;
        };
        self.storage.refresh_queued = true;
        let generation = self.generation;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(REFRESH_DEBOUNCE).await;
            let (images, volumes) = futures::join!(engine.list_images(), engine.list_volumes());
            let current = this.update(cx, |store, cx| {
                if !store.is_current(generation) {
                    return false;
                }
                store.storage.refresh_queued = false;
                match (images, volumes) {
                    (Ok(images), Ok(volumes)) => store.merge_lists(images, volumes),
                    (Err(err), _) | (_, Err(err)) => {
                        log::warn!("storage refresh failed: {err}");
                    }
                }
                cx.notify();
                true
            });
            if !matches!(current, Ok(true)) {
                return;
            }
            match engine.disk_usage().await {
                Ok(usage) => {
                    this.update(cx, |store, cx| {
                        if !store.is_current(generation) {
                            return;
                        }
                        store.storage.images = usage.images;
                        store.storage.volumes = usage.volumes;
                        store.storage.build_cache_reclaimable =
                            Some(usage.build_cache_reclaimable_bytes);
                        store.storage.usage_known = true;
                        cx.notify();
                    })
                    .ok();
                }
                Err(err) => log::warn!("disk usage failed: {err}"),
            }
        })
        .detach();
    }

    /// Replace lists while keeping the last known sizes/usage until disk
    /// usage arrives, so rows don't flicker to "unknown".
    fn merge_lists(&mut self, mut images: Vec<Image>, mut volumes: Vec<Volume>) {
        for image in &mut images {
            if image.containers.is_none()
                && let Some(old) = self.storage.images.iter().find(|i| i.id == image.id)
            {
                image.containers = old.containers;
            }
        }
        for volume in &mut volumes {
            if let Some(old) = self.storage.volumes.iter().find(|v| v.name == volume.name) {
                volume.size = volume.size.or(old.size);
                volume.ref_count = volume.ref_count.or(old.ref_count);
            }
        }
        self.storage.images = images;
        self.storage.volumes = volumes;
    }

    /// Remove images by id. In-use images need `force`.
    pub fn remove_images(&mut self, ids: Vec<String>, force: bool, cx: &mut Context<Self>) {
        let Some(engine) = self.engine.clone() else {
            return Self::not_connected("remove images", cx);
        };
        for id in ids {
            if self.pending.contains_key(&id) {
                continue;
            }
            self.pending.insert(id.clone(), Op::Remove);
            let call = engine.remove_image(&id, force);
            let generation = self.generation;
            cx.spawn(async move |this, cx| {
                let result = call.await;
                this.update(cx, |store, cx| {
                    if !store.is_current(generation) {
                        return;
                    }
                    store.pending.remove(&id);
                    match result {
                        Ok(()) => store.storage.images.retain(|i| i.id != id),
                        Err(err) => {
                            let name = store
                                .storage
                                .images
                                .iter()
                                .find(|i| i.id == id)
                                .map_or_else(|| id.clone(), Image::display_name);
                            cx.emit(StoreEvent::Error {
                                title: format!("Couldn't remove {name}"),
                                detail: err.to_string(),
                            });
                        }
                    }
                    store.refresh_storage(cx);
                    cx.notify();
                })
                .ok();
            })
            .detach();
        }
        cx.notify();
    }

    /// Remove volumes by name. In-use volumes fail with the engine's message.
    pub fn remove_volumes(&mut self, names: Vec<String>, cx: &mut Context<Self>) {
        let Some(engine) = self.engine.clone() else {
            return Self::not_connected("remove volumes", cx);
        };
        for name in names {
            if self.pending.contains_key(&name) {
                continue;
            }
            self.pending.insert(name.clone(), Op::Remove);
            let call = engine.remove_volume(&name, false);
            let generation = self.generation;
            cx.spawn(async move |this, cx| {
                let result = call.await;
                this.update(cx, |store, cx| {
                    if !store.is_current(generation) {
                        return;
                    }
                    store.pending.remove(&name);
                    match result {
                        Ok(()) => store.storage.volumes.retain(|v| v.name != name),
                        Err(err) => cx.emit(StoreEvent::Error {
                            title: format!("Couldn't remove {name}"),
                            detail: err.to_string(),
                        }),
                    }
                    store.refresh_storage(cx);
                    cx.notify();
                })
                .ok();
            })
            .detach();
        }
        cx.notify();
    }

    /// Run the selected prunes one after another and report the total.
    pub fn prune(&mut self, targets: Vec<PruneTarget>, cx: &mut Context<Self>) {
        if self.storage.pruning || targets.is_empty() {
            return;
        }
        let Some(engine) = self.engine.clone() else {
            return Self::not_connected("clean up", cx);
        };
        self.storage.pruning = true;
        cx.notify();
        let generation = self.generation;
        cx.spawn(async move |this, cx| {
            let mut deleted = 0usize;
            let mut reclaimed = 0u64;
            let mut failures = Vec::new();
            for target in targets {
                let result: Result<PruneReport, _> = match target {
                    PruneTarget::StoppedContainers => engine.prune_containers().await,
                    PruneTarget::DanglingImages => engine.prune_images(ImagePrune::Dangling).await,
                    PruneTarget::UnusedImages => engine.prune_images(ImagePrune::AllUnused).await,
                    PruneTarget::UnusedVolumes => engine.prune_volumes().await,
                    PruneTarget::UnusedNetworks => engine.prune_networks().await,
                    PruneTarget::BuildCache => engine.prune_build_cache().await,
                };
                match result {
                    Ok(report) => {
                        deleted = deleted.saturating_add(report.deleted.len());
                        reclaimed = reclaimed.saturating_add(report.reclaimed_bytes);
                    }
                    Err(err) => failures.push(format!("{}: {err}", target.label())),
                }
            }
            this.update(cx, |store, cx| {
                if !store.is_current(generation) {
                    return;
                }
                store.storage.pruning = false;
                if failures.is_empty() {
                    cx.emit(StoreEvent::Info {
                        title: format!("Reclaimed {}", format::bytes(reclaimed)),
                        detail: format!("Removed {deleted} items."),
                    });
                } else {
                    cx.emit(StoreEvent::Error {
                        title: "Clean up partly failed".into(),
                        detail: failures.join("\n"),
                    });
                }
                store.refresh_storage(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn not_connected(what: &str, cx: &mut Context<Self>) {
        cx.emit(StoreEvent::Error {
            title: format!("Couldn't {what}"),
            detail: "Not connected to a container engine.".into(),
        });
    }
}
