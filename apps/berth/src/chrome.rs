//! Window chrome for client-side decorations: the resizable shadowed frame,
//! titlebar drag regions and Linux and Windows caption buttons.
//!
//! Follows Zed's `window_shadow` example.

use std::cell::Cell;

#[cfg(any(target_os = "linux", target_os = "freebsd", windows))]
use berth_ui::TabStop;
use berth_ui::Theme;
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
use berth_ui::{icon, icons};
use gpui::{
    AnyElement, App, Bounds, CursorStyle, Decorations, Div, HitboxBehavior, MouseButton, Pixels,
    Point, ResizeEdge, Size, Stateful, Tiling, Window, canvas, div, point, prelude::*, px,
    transparent_black,
};

#[cfg(any(target_os = "linux", target_os = "freebsd", windows))]
const CAPTION_GROUP: &str = "caption-button";

/// Width of one Windows caption button, as in Windows 11.
#[cfg(windows)]
const CAPTION_WIDTH: f32 = 46.0;
/// Horizontal room a centred titlebar strip leaves on each side, clear of
/// the caption buttons.
#[cfg(windows)]
pub const CAPTION_CLEARANCE: f32 = 3.0 * CAPTION_WIDTH + Theme::SPACE_SM;
#[cfg(not(windows))]
pub const CAPTION_CLEARANCE: f32 = 120.0;

/// Width of the invisible resize band (and shadow) around a CSD window.
const FRAME_INSET: Pixels = px(10.0);
/// Corner radius of a floating CSD window.
pub const WINDOW_RADIUS: f32 = 10.0;

/// Origin of the macOS close button, vertically centred in our header.
pub const TRAFFIC_LIGHT_POSITION: Point<Pixels> =
    point(px(14.0), px((Theme::HEADER_HEIGHT - 14.0) / 2.0));
/// Fallback right edge of the macOS zoom button, should `AppKit` not hand
/// us the button: three 14pt buttons, 9pt apart (macOS 26+).
#[cfg(target_os = "macos")]
const TRAFFIC_LIGHTS_END: f32 = 14.0 + 3.0 * 14.0 + 2.0 * 9.0;
/// Left edge of our own leading titlebar controls when nothing native
/// occupies the corner.
pub const TITLEBAR_LEADING: f32 = Theme::SPACE_SM + 2.0;

thread_local! {
    /// Armed by a press on a drag region; the first move with the button
    /// held hands the drag to the compositor. Lets double-click reach
    /// `zoom_window` instead of being swallowed by a move session.
    static SHOULD_MOVE: Cell<bool> = const { Cell::new(false) };
}

/// The corner radius the content should use for its outermost corners.
pub fn content_radius(window: &Window) -> f32 {
    match window.window_decorations() {
        Decorations::Client { tiling } if !tiling.is_tiled() && !window.is_maximized() => {
            WINDOW_RADIUS
        }
        _ => 0.0,
    }
}

/// Left edge for our leading titlebar controls, clear of the native macOS
/// traffic lights (which hide in fullscreen).
#[cfg_attr(
    not(target_os = "macos"),
    allow(clippy::missing_const_for_fn, reason = "the macOS branch is not const")
)]
pub fn titlebar_leading(window: &Window) -> f32 {
    #[cfg(target_os = "macos")]
    if !window.is_fullscreen() {
        return traffic_lights_end(window).unwrap_or(TRAFFIC_LIGHTS_END) + Theme::SPACE_MD;
    }
    #[cfg(not(target_os = "macos"))]
    let _ = window;
    TITLEBAR_LEADING
}

/// Right edge of the native zoom button in window coordinates, measured
/// from `AppKit` so button size and spacing are right on every macOS release.
#[cfg(target_os = "macos")]
fn traffic_lights_end(window: &Window) -> Option<f32> {
    use num_traits::ToPrimitive as _;
    use objc2_app_kit::{NSView, NSWindowButton};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    let RawWindowHandle::AppKit(handle) = HasWindowHandle::window_handle(window).ok()?.as_raw()
    else {
        return None;
    };
    // SAFETY: GPUI's AppKit handle points at the window's live content
    // `NSView`, borrowed for as long as `window`; we render on the main thread.
    let view = unsafe { handle.ns_view.cast::<NSView>().as_ref() };
    let zoom = view
        .window()?
        .standardWindowButton(NSWindowButton::ZoomButton)?;
    let frame = zoom.convertRect_toView(zoom.bounds(), None);
    (frame.origin.x + frame.size.width).to_f32()
}

/// Make `el` drag the window; double-click toggles maximize.
pub fn drag_region(id: &'static str, el: Div) -> Stateful<Div> {
    let el = el
        .id(id)
        .on_mouse_down(MouseButton::Left, |_, _, _| SHOULD_MOVE.set(true))
        .on_mouse_down_out(|_, _, _| SHOULD_MOVE.set(false))
        .on_mouse_up(MouseButton::Left, |_, _, _| SHOULD_MOVE.set(false))
        .on_mouse_move(|event, window, _| {
            if SHOULD_MOVE.get() && event.pressed_button == Some(MouseButton::Left) {
                SHOULD_MOVE.set(false);
                #[cfg(windows)]
                win32::start_move(window);
                #[cfg(not(windows))]
                window.start_window_move();
            }
        })
        .on_click(|event, window, _| {
            if event.click_count() == 2 {
                #[cfg(target_os = "macos")]
                window.titlebar_double_click();
                #[cfg(windows)]
                win32::toggle_maximize(window);
                #[cfg(not(any(target_os = "macos", windows)))]
                window.zoom_window();
            }
        });
    // Windows opens the window menu on release, as its own titlebar does.
    #[cfg(windows)]
    let el = el.on_mouse_up(MouseButton::Right, |_, window, _| {
        win32::show_window_menu(window);
    });
    #[cfg(not(windows))]
    let el = el.on_mouse_down(MouseButton::Right, |event, window, _| {
        window.show_window_menu(event.position);
    });
    el
}

/// Window commands gpui's Windows backend lacks: it has no window move or
/// window menu, and its zoom only maximizes. Each is posted to the window
/// procedure, so the modal loop it may start (moving, the menu) runs after
/// gpui's event dispatch has returned rather than inside it.
#[cfg(windows)]
mod win32 {
    use gpui::Window;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows::Win32::Foundation::{HWND, LPARAM, POINT, WPARAM};
    use windows::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetCursorPos, HTCAPTION, PostMessageW, SC_MAXIMIZE, SC_MOVE, SC_RESTORE, WM_SYSCOMMAND,
    };

    /// Opens the window menu at the screen point in `lparam`. Undocumented,
    /// but what `DefWindowProc` itself uses for a right-click on the caption.
    const WM_POPUPSYSTEMMENU: u32 = 0x0313;

    fn hwnd(window: &Window) -> Option<HWND> {
        let RawWindowHandle::Win32(handle) = HasWindowHandle::window_handle(window).ok()?.as_raw()
        else {
            return None;
        };
        Some(HWND(std::ptr::with_exposed_provenance_mut(
            handle.hwnd.get().cast_unsigned(),
        )))
    }

    fn post(window: &Window, message: u32, wparam: u32, lparam: isize) {
        let (Some(hwnd), Ok(wparam)) = (hwnd(window), usize::try_from(wparam)) else {
            return;
        };
        // SAFETY: `hwnd` is the live window `window` wraps; posting only
        // queues the message for its window procedure.
        if let Err(error) =
            unsafe { PostMessageW(Some(hwnd), message, WPARAM(wparam), LPARAM(lparam)) }
        {
            log::warn!("posting window message {message:#x} failed: {error}");
        }
    }

    /// Hand the drag to the system move loop, which also snaps.
    pub fn start_move(window: &Window) {
        // gpui captures the mouse on press; the move loop needs it released.
        // SAFETY: plain Win32 call on the UI thread.
        unsafe { ReleaseCapture() }.ok();
        post(window, WM_SYSCOMMAND, SC_MOVE | HTCAPTION, 0);
    }

    pub fn toggle_maximize(window: &Window) {
        let command = if window.is_maximized() {
            SC_RESTORE
        } else {
            SC_MAXIMIZE
        };
        post(window, WM_SYSCOMMAND, command, 0);
    }

    /// The window menu (restore, move, size, …) at the cursor.
    pub fn show_window_menu(window: &Window) {
        let mut cursor = POINT::default();
        // SAFETY: `cursor` is a valid out-pointer for the call.
        if unsafe { GetCursorPos(&raw mut cursor) }.is_err() {
            return;
        }
        // MAKELPARAM: x in the low word, y in the high word.
        let [x0, x1, ..] = cursor.x.to_le_bytes();
        let [y0, y1, ..] = cursor.y.to_le_bytes();
        let Ok(lparam) = isize::try_from(u32::from_le_bytes([x0, x1, y0, y1])) else {
            return;
        };
        post(window, WM_POPUPSYSTEMMENU, 0, lparam);
    }
}

/// Wrap the app content in a CSD frame: transparent resize band, rounded
/// corners, hairline border and shadow. A no-op under server decorations.
pub fn window_frame(content: AnyElement, window: &mut Window, cx: &App) -> impl IntoElement {
    let theme = Theme::of(cx);
    let decorations = window.window_decorations();
    let radius = px(content_radius(window));
    match decorations {
        Decorations::Server => window.set_client_inset(px(0.0)),
        Decorations::Client { .. } => window.set_client_inset(FRAME_INSET),
    }

    div()
        .id("window-frame")
        .size_full()
        .bg(transparent_black())
        .map(|d| match decorations {
            Decorations::Server => d,
            Decorations::Client { tiling } => d
                .child(
                    canvas(
                        |_bounds, window, _cx| {
                            window.insert_hitbox(
                                Bounds::new(
                                    point(px(0.0), px(0.0)),
                                    window.window_bounds().get_bounds().size,
                                ),
                                HitboxBehavior::Normal,
                            )
                        },
                        move |_bounds, hitbox, window, _cx| {
                            let mouse = window.mouse_position();
                            let size = window.window_bounds().get_bounds().size;
                            if let Some(edge) = resize_edge(mouse, size, tiling) {
                                window.set_cursor_style(cursor_for(edge), &hitbox);
                            }
                        },
                    )
                    .size_full()
                    .absolute(),
                )
                .when(!tiling.top, |d| d.pt(FRAME_INSET))
                .when(!tiling.bottom, |d| d.pb(FRAME_INSET))
                .when(!tiling.left, |d| d.pl(FRAME_INSET))
                .when(!tiling.right, |d| d.pr(FRAME_INSET))
                .on_mouse_move(|_, window, _| window.refresh())
                .on_mouse_down(MouseButton::Left, move |e, window, _| {
                    let size = window.window_bounds().get_bounds().size;
                    if let Some(edge) = resize_edge(e.position, size, tiling) {
                        window.start_window_resize(edge);
                    }
                }),
        })
        .child(
            div()
                .size_full()
                .cursor(CursorStyle::Arrow)
                .overflow_hidden()
                .rounded(radius)
                .bg(theme.bg)
                .map(|d| match decorations {
                    Decorations::Server => d,
                    Decorations::Client { tiling } => d
                        .border_color(theme.border_strong)
                        .when(!tiling.top, Styled::border_t_1)
                        .when(!tiling.bottom, Styled::border_b_1)
                        .when(!tiling.left, Styled::border_l_1)
                        .when(!tiling.right, Styled::border_r_1)
                        .when(!tiling.is_tiled(), |d| {
                            d.shadow(vec![
                                gpui::BoxShadow::new(
                                    px(0.0),
                                    px(2.0),
                                    gpui::hsla(0.0, 0.0, 0.0, 0.45),
                                )
                                .blur_radius(px(f32::from(FRAME_INSET) / 1.5)),
                            ])
                        }),
                })
                .on_mouse_move(|_, _, cx| cx.stop_propagation())
                .child(content),
        )
}

fn resize_edge(pos: Point<Pixels>, size: Size<Pixels>, tiling: Tiling) -> Option<ResizeEdge> {
    let inset = FRAME_INSET;
    let top = pos.y < inset && !tiling.top;
    let bottom = pos.y > px(f32::from(size.height) - f32::from(inset)) && !tiling.bottom;
    let left = pos.x < inset && !tiling.left;
    let right = pos.x > px(f32::from(size.width) - f32::from(inset)) && !tiling.right;
    Some(match (top, bottom, left, right) {
        (true, _, true, _) => ResizeEdge::TopLeft,
        (true, _, _, true) => ResizeEdge::TopRight,
        (_, true, true, _) => ResizeEdge::BottomLeft,
        (_, true, _, true) => ResizeEdge::BottomRight,
        (true, ..) => ResizeEdge::Top,
        (_, true, ..) => ResizeEdge::Bottom,
        (_, _, true, _) => ResizeEdge::Left,
        (_, _, _, true) => ResizeEdge::Right,
        _ => return None,
    })
}

const fn cursor_for(edge: ResizeEdge) -> CursorStyle {
    match edge {
        ResizeEdge::Top | ResizeEdge::Bottom => CursorStyle::ResizeUpDown,
        ResizeEdge::Left | ResizeEdge::Right => CursorStyle::ResizeLeftRight,
        ResizeEdge::TopLeft | ResizeEdge::BottomRight => CursorStyle::ResizeUpLeftDownRight,
        ResizeEdge::TopRight | ResizeEdge::BottomLeft => CursorStyle::ResizeUpRightDownLeft,
    }
}

/// macOS draws its own traffic lights.
#[cfg(not(any(target_os = "linux", target_os = "freebsd", windows)))]
pub const fn caption_buttons(_window: &Window, _cx: &App) -> Option<AnyElement> {
    None
}

/// Minimize / maximize (restore) / close in the Windows 11 style: flush with
/// the top-right corner, the header's full height. Each is also the native
/// caption area for its button, so hovering maximize opens Snap Layouts.
#[cfg(windows)]
pub fn caption_buttons(window: &Window, cx: &App) -> Option<AnyElement> {
    use gpui::WindowControlArea;

    if window.is_fullscreen() {
        return None;
    }
    let theme = Theme::of(cx);
    let fg = if window.is_window_active() {
        theme.text
    } else {
        theme.text_muted
    };
    let maximized = window.is_maximized();
    let buttons = [
        WindowControlArea::Min,
        WindowControlArea::Max,
        WindowControlArea::Close,
    ]
    .map(|area| {
        let (id, label) = match area {
            WindowControlArea::Min => ("caption-min", "Minimize"),
            WindowControlArea::Max if maximized => ("caption-max", "Restore"),
            WindowControlArea::Max => ("caption-max", "Maximize"),
            _ => ("caption-close", "Close"),
        };
        let close = area == WindowControlArea::Close;
        let (hover, pressed) = if close {
            (gpui::rgb(0x00c4_2b1c).into(), gpui::rgb(0x00b2_2a1b).into())
        } else {
            (theme.element_hover, theme.element_active)
        };
        TabStop::new(
            id,
            div()
                .id(id)
                .group(CAPTION_GROUP)
                .w(px(CAPTION_WIDTH))
                .h(px(Theme::HEADER_HEIGHT))
                .border_1()
                .border_color(transparent_black())
                .flex()
                .items_center()
                .justify_center()
                .hover(move |s| s.bg(hover))
                .active(move |s| s.bg(pressed))
                .occlude()
                .window_control_area(area)
                .role(gpui::Role::Button)
                .aria_label(label)
                .child(caption_glyph(area, maximized, fg, cx))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(move |_, window, cx| {
                    cx.stop_propagation();
                    match area {
                        WindowControlArea::Min => window.minimize_window(),
                        WindowControlArea::Max => win32::toggle_maximize(window),
                        _ => window.remove_window(),
                    }
                }),
        )
    });

    Some(
        div()
            .absolute()
            .top_0()
            .right_0()
            .flex()
            .children(buttons)
            .into_any_element(),
    )
}

/// A Windows caption glyph from the system icon font, at the 10px Windows
/// itself uses; maximize shows the restore glyph when maximized.
#[cfg(windows)]
fn caption_glyph(
    area: gpui::WindowControlArea,
    maximized: bool,
    fg: gpui::Hsla,
    cx: &App,
) -> AnyElement {
    use gpui::WindowControlArea;

    let glyph = match area {
        WindowControlArea::Min => "\u{E921}",
        WindowControlArea::Max if maximized => "\u{E923}",
        WindowControlArea::Max => "\u{E922}",
        _ => "\u{E8BB}",
    };
    div()
        .font_family(caption_font(cx))
        .text_size(px(10.0))
        .line_height(px(10.0))
        .text_color(fg)
        .when(area == WindowControlArea::Close, |d| {
            d.group_hover(CAPTION_GROUP, |s| s.text_color(gpui::white()))
        })
        .child(glyph)
        .into_any_element()
}

/// The font Windows draws its caption glyphs with: Segoe Fluent Icons on
/// Windows 11, Segoe MDL2 Assets (same code points) on Windows 10.
#[cfg(windows)]
fn caption_font(cx: &App) -> gpui::SharedString {
    const FLUENT: &str = "Segoe Fluent Icons";
    thread_local! {
        static FONT: std::cell::OnceCell<gpui::SharedString> = const { std::cell::OnceCell::new() };
    }
    FONT.with(|font| {
        font.get_or_init(|| {
            let fluent = cx
                .text_system()
                .all_font_names()
                .iter()
                .any(|name| name == FLUENT);
            if fluent { FLUENT } else { "Segoe MDL2 Assets" }.into()
        })
        .clone()
    })
}

/// Minimize / maximize / close, drawn by us when the compositor leaves
/// decorations to the client. Follows the desktop's button layout.
///
/// TODO: only the right-hand side is rendered; a left-side layout
/// (e.g. GNOME `close,minimize:`) needs clearance around the sidebar toggle.
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
pub fn caption_buttons(window: &Window, cx: &App) -> Option<AnyElement> {
    if !matches!(window.window_decorations(), Decorations::Client { .. }) {
        return None;
    }
    let theme = Theme::of(cx);
    let controls = window.window_controls();
    let layout = cx
        .button_layout()
        .unwrap_or_else(gpui::WindowButtonLayout::linux_default);

    let hover_fg = theme.text;
    let buttons = layout.right.into_iter().flatten().filter_map(|button| {
        use gpui::WindowButton::{Close, Maximize, Minimize};
        let (id, glyph, close): (&'static str, AnyElement, bool) = match button {
            Minimize if controls.minimize => (
                "caption-min",
                div()
                    .w(px(10.0))
                    .h(px(1.5))
                    .rounded_full()
                    .bg(theme.text_muted)
                    .group_hover(CAPTION_GROUP, move |s| s.bg(hover_fg))
                    .into_any_element(),
                false,
            ),
            Maximize if controls.maximize => (
                "caption-max",
                div()
                    .size(px(9.5))
                    .rounded(px(2.0))
                    .border(px(1.5))
                    .border_color(theme.text_muted)
                    .group_hover(CAPTION_GROUP, move |s| s.border_color(hover_fg))
                    .into_any_element(),
                false,
            ),
            Close => (
                "caption-close",
                icon(icons::CLOSE)
                    .size(px(15.0))
                    .text_color(theme.text_muted)
                    .group_hover(CAPTION_GROUP, |s| s.text_color(gpui::white()))
                    .into_any_element(),
                true,
            ),
            _ => return None,
        };
        let hover_background = if close {
            gpui::rgb(0x00e5_484d).into()
        } else {
            theme.element_hover
        };
        Some(TabStop::new(
            id,
            div()
                .id(id)
                .group(CAPTION_GROUP)
                .size(px(24.0))
                .border_1()
                .border_color(transparent_black())
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .bg(theme.row_hover)
                .cursor_pointer()
                .hover(move |s| s.bg(hover_background))
                .occlude()
                .child(glyph)
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(move |_, window, cx| {
                    cx.stop_propagation();
                    match button {
                        Minimize => window.minimize_window(),
                        Maximize => window.zoom_window(),
                        Close => window.remove_window(),
                    }
                }),
        ))
    });

    Some(
        div()
            .absolute()
            .top(px((Theme::HEADER_HEIGHT - 24.0) / 2.0))
            .right(px(Theme::SPACE_MD))
            .flex()
            .items_center()
            .gap(px(8.0))
            .children(buttons)
            .into_any_element(),
    )
}
