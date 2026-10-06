mod chrome;
mod dialogs;
mod screens;
mod shell;
mod store;

use std::sync::Arc;
use std::time::Duration;

use berth_docker::{Engine, fixtures::FixtureEngine};

use gpui::{
    App, AppContext as _, Bounds, KeyBinding, TitlebarOptions, WindowBounds, WindowDecorations,
    WindowOptions, px, size,
};

use shell::{Quit, Shell, ToggleSidebar};

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
                KeyBinding::new("ctrl-q", Quit, None),
                KeyBinding::new("cmd-q", Quit, None),
                KeyBinding::new("enter", dialogs::Confirm, Some(dialogs::CONFIRM_CONTEXT)),
                KeyBinding::new("escape", dialogs::Cancel, Some(dialogs::CONFIRM_CONTEXT)),
            ]);
            cx.on_action(|_: &Quit, cx| cx.quit());

            let bounds = Bounds::centered(None, size(px(1240.0), px(800.0)), cx);
            cx.open_window(
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
                |window, cx| cx.new(|cx| Shell::new(engine(), window, cx)),
            )
            .expect("failed to open the main window");
            cx.activate(true);
        });
}

/// The engine to drive the UI with. Fixture data until the live client and
/// discovery are merged (M1); `BERTH_FIXTURE_LATENCY_MS` simulates a slow engine.
fn engine() -> Arc<dyn Engine> {
    let latency = std::env::var("BERTH_FIXTURE_LATENCY_MS")
        .ok()
        .and_then(|ms| ms.parse().ok())
        .unwrap_or(600);
    Arc::new(FixtureEngine::default().with_latency(Duration::from_millis(latency)))
}
