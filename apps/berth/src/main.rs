mod chrome;
mod dialogs;
mod format;
mod keymap;
mod live;
mod preferences;
mod screens;
mod selection;
mod shell;
mod store;
mod virtual_rows;

use std::sync::{Arc, PoisonError};
use std::time::Duration;

use berth_docker::{BollardEngine, DiscoveryEnv, discover, fixtures::FixtureEngine};
use berth_settings::EngineChoice;
use preferences::EngineTarget;
use store::{ConnectFailure, Connector, Pin};

use gpui::{
    App, AppContext as _, Bounds, TitlebarOptions, WindowBounds, WindowDecorations, WindowOptions,
    px, size,
};

use keymap::Quit;
use shell::Shell;

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    gpui_platform::application()
        .with_assets(berth_ui::Assets)
        .run(|cx: &mut App| {
            gpui_base::init(cx);
            berth_ui::init(cx);
            preferences::init(cx);

            keymap::bind(cx);
            cx.on_action(|_: &Quit, cx| cx.quit());

            let connector = connector(preferences::engine_target(cx));
            let bounds = Bounds::centered(None, size(px(1240.0), px(800.0)), cx);
            if let Err(error) = cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    window_min_size: Some(size(px(860.0), px(520.0))),
                    titlebar: Some(TitlebarOptions {
                        title: Some("Berth".into()),
                        appears_transparent: true,
                        traffic_light_position: Some(chrome::TRAFFIC_LIGHT_POSITION),
                    }),
                    // We draw our own unified titlebar and, under CSD, our own
                    // caption buttons (see chrome.rs). The compositor may still
                    // negotiate server decorations; chrome re-resolves per frame.
                    window_decorations: cfg!(any(target_os = "linux", target_os = "freebsd"))
                        .then_some(WindowDecorations::Client),
                    app_owns_titlebar_drag: true,
                    app_id: Some("no.ahse.berth".into()),
                    ..Default::default()
                },
                |window, cx| cx.new(|cx| Shell::new(connector, window, cx)),
            ) {
                log::error!("failed to open the main window: {error}");
                cx.quit();
                return;
            }
            cx.activate(true);
        });
}

/// How the store obtains an engine. `BERTH_FIXTURES=1` runs on in-memory
/// fixture data (`BERTH_FIXTURE_LATENCY_MS` simulates a slow engine,
/// `BERTH_FIXTURE_LOG_LINES` sets per-container log history, e.g. 1000000);
/// otherwise each connect resolves `target` — a host from settings, or
/// discovery (`DOCKER_HOST`, Docker context, default sockets) — and opens a
/// live client.
fn connector(target: EngineTarget) -> Connector {
    if std::env::var_os("BERTH_FIXTURES").is_some() {
        let latency = std::env::var("BERTH_FIXTURE_LATENCY_MS")
            .ok()
            .and_then(|ms| ms.parse().ok())
            .unwrap_or(600);
        let log_lines = std::env::var("BERTH_FIXTURE_LOG_LINES")
            .ok()
            .and_then(|n| n.parse().ok());
        return Arc::new(move || {
            let engine = FixtureEngine::default().with_latency(Duration::from_millis(latency));
            Ok(Arc::new(match log_lines {
                Some(lines) => engine.with_log_history_lines(lines),
                None => engine,
            }))
        });
    }
    Arc::new(move || {
        let choice = target
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let mut env = DiscoveryEnv::from_process();
        match &choice {
            EngineChoice::Auto => {}
            // An explicit context wins over DOCKER_HOST, like `docker --context`.
            EngineChoice::Context { name } => {
                env.docker_host = None;
                env.docker_context = Some(name.clone());
            }
            EngineChoice::Host { host } => env.docker_host = Some(host.clone()),
        }
        let pinned = !matches!(choice, EngineChoice::Auto);
        let discovered = discover(&env).map_err(|error| ConnectFailure {
            error: error.into(),
            pin: pinned.then_some(Pin::Settings),
        })?;
        log::info!(
            "connecting to {} ({:?})",
            discovered.endpoint,
            discovered.source
        );
        let engine =
            BollardEngine::connect(&discovered.endpoint).map_err(|error| ConnectFailure {
                error: error.into(),
                pin: Pin::of(&choice, &env, &discovered.source),
            })?;
        Ok(Arc::new(engine))
    })
}
