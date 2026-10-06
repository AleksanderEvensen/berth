mod chrome;
mod dialogs;
mod format;
mod screens;
mod selection;
mod shell;
mod store;
mod virtual_rows;

use std::sync::Arc;
use std::time::Duration;

use berth_docker::{BollardEngine, DiscoveryEnv, discover, fixtures::FixtureEngine};
use store::Connector;

use gpui::{
    App, AppContext as _, Bounds, KeyBinding, TitlebarOptions, WindowBounds, WindowDecorations,
    WindowOptions, px, size,
};

use shell::{FocusSearch, Quit, Shell, ToggleSidebar};

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    gpui_platform::application()
        .with_assets(berth_ui::Assets)
        .run(|cx: &mut App| {
            gpui_base::init(cx);
            berth_ui::init(cx);

            cx.bind_keys([
                KeyBinding::new("ctrl-b", ToggleSidebar, None),
                KeyBinding::new("cmd-b", ToggleSidebar, None),
                KeyBinding::new("ctrl-f", FocusSearch, None),
                KeyBinding::new("cmd-f", FocusSearch, None),
                KeyBinding::new("ctrl-q", Quit, None),
                KeyBinding::new("cmd-q", Quit, None),
                KeyBinding::new("enter", dialogs::Confirm, Some(dialogs::CONFIRM_CONTEXT)),
                KeyBinding::new("escape", dialogs::Cancel, Some(dialogs::CONFIRM_CONTEXT)),
            ]);
            cx.on_action(|_: &Quit, cx| cx.quit());

            let bounds = Bounds::centered(None, size(px(1240.0), px(800.0)), cx);
            if let Err(error) = cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    window_min_size: Some(size(px(860.0), px(520.0))),
                    titlebar: Some(TitlebarOptions {
                        title: Some("Berth".into()),
                        appears_transparent: true,
                        traffic_light_position: Some(gpui::point(px(14.0), px(19.0))),
                    }),
                    // We draw our own unified titlebar and, under CSD, our own
                    // caption buttons (see chrome.rs). The compositor may still
                    // negotiate server decorations; chrome re-resolves per frame.
                    window_decorations: cfg!(any(target_os = "linux", target_os = "freebsd"))
                        .then_some(WindowDecorations::Client),
                    app_id: Some("berth".into()),
                    ..Default::default()
                },
                |window, cx| cx.new(|cx| Shell::new(connector(), window, cx)),
            ) {
                log::error!("failed to open the main window: {error}");
                cx.quit();
                return;
            }
            cx.activate(true);
        });
}

/// How the store obtains an engine. `BERTH_FIXTURES=1` runs on in-memory
/// fixture data (`BERTH_FIXTURE_LATENCY_MS` simulates a slow engine);
/// otherwise each connect discovers the endpoint (`DOCKER_HOST`, Docker
/// context, default sockets) and opens a live client.
fn connector() -> Connector {
    if std::env::var_os("BERTH_FIXTURES").is_some() {
        let latency = std::env::var("BERTH_FIXTURE_LATENCY_MS")
            .ok()
            .and_then(|ms| ms.parse().ok())
            .unwrap_or(600);
        return Arc::new(move || {
            Ok(Arc::new(
                FixtureEngine::default().with_latency(Duration::from_millis(latency)),
            ))
        });
    }
    Arc::new(|| {
        let discovered = discover(&DiscoveryEnv::from_process()).map_err(|e| e.to_string())?;
        log::info!(
            "connecting to {} ({:?})",
            discovered.endpoint,
            discovered.source
        );
        let engine = BollardEngine::connect(&discovered.endpoint).map_err(|e| e.to_string())?;
        Ok(Arc::new(engine))
    })
}
