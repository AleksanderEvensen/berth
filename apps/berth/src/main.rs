mod chrome;
mod screens;
mod shell;

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
                |window, cx| cx.new(|cx| Shell::new(window, cx)),
            )
            .expect("failed to open the main window");
            cx.activate(true);
        });
}
