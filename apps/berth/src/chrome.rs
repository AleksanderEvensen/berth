//! Window chrome for client-side decorations: the resizable shadowed frame,
//! titlebar drag regions and Linux caption buttons.
//!
//! Follows Zed's `window_shadow` example and zeron's titlebar handling
//! (`shell.rs` `titlebar_drag_region` / `linux_caption_button`).

use std::cell::Cell;

use berth_ui::Theme;
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
use berth_ui::{icon, icons};
use gpui::{
    AnyElement, App, Bounds, CursorStyle, Decorations, Div, HitboxBehavior, MouseButton, Pixels,
    Point, ResizeEdge, Size, Stateful, Tiling, Window, canvas, div, point, prelude::*, px,
    transparent_black,
};

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
const CAPTION_GROUP: &str = "caption-button";

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
    el.id(id)
        .on_mouse_down(MouseButton::Left, |_, _, _| SHOULD_MOVE.set(true))
        .on_mouse_down_out(|_, _, _| SHOULD_MOVE.set(false))
        .on_mouse_up(MouseButton::Left, |_, _, _| SHOULD_MOVE.set(false))
        .on_mouse_move(|event, window, _| {
            if SHOULD_MOVE.get() && event.pressed_button == Some(MouseButton::Left) {
                SHOULD_MOVE.set(false);
                window.start_window_move();
            }
        })
        .on_click(|event, window, _| {
            if event.click_count() == 2 {
                window.zoom_window();
            }
        })
        .on_mouse_down(MouseButton::Right, |event, window, _| {
            window.show_window_menu(event.position);
        })
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

/// Other platforms always draw native window controls.
#[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
pub const fn caption_buttons(_window: &Window, _cx: &App) -> Option<AnyElement> {
    None
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
        Some(
            div()
                .id(id)
                .group(CAPTION_GROUP)
                .size(px(24.0))
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
        )
    });

    Some(
        div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .children(buttons)
            .into_any_element(),
    )
}
