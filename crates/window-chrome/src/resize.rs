//! Resizing a window that draws its own frame (client-side decorations).
//!
//! Under client-side decorations the window's surface includes an
//! invisible band around the visible frame, which the app tells gpui about
//! with `Window::set_client_inset` and which resizes the window when
//! dragged. These helpers do the hit-testing; [`ResizeHandles`] wires it up.

use gpui::{
    App, Bounds, CursorStyle, Decorations, HitboxBehavior, MouseButton, Pixels, Point, ResizeEdge,
    Size, Tiling, Window, canvas, div, point, prelude::*, px,
};

/// How the window's edges are tiled, if it draws its own frame; `None`
/// under server-side decorations, where the system draws the frame.
#[must_use]
pub fn client_tiling(window: &Window) -> Option<Tiling> {
    match window.window_decorations() {
        Decorations::Client { tiling } => Some(tiling),
        Decorations::Server => None,
    }
}

/// Whether the window draws its own frame and floats free: not tiled on
/// any edge, not maximized. That is when rounded corners, a shadow and a
/// resize band make sense.
#[must_use]
pub fn is_floating(window: &Window) -> bool {
    client_tiling(window).is_some_and(|tiling| !tiling.is_tiled()) && !window.is_maximized()
}

/// The edge or corner at `position` (window coordinates) in a window of
/// `size`, within `inset` of an edge that isn't tiled.
#[must_use]
pub fn resize_edge(
    position: Point<Pixels>,
    size: Size<Pixels>,
    tiling: Tiling,
    inset: Pixels,
) -> Option<ResizeEdge> {
    let (x, y) = (f32::from(position.x), f32::from(position.y));
    let inset = f32::from(inset);
    let top = y < inset && !tiling.top;
    let bottom = y > f32::from(size.height) - inset && !tiling.bottom;
    let left = x < inset && !tiling.left;
    let right = x > f32::from(size.width) - inset && !tiling.right;
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

/// The pointer for resizing from `edge`.
#[must_use]
pub const fn resize_cursor(edge: ResizeEdge) -> CursorStyle {
    match edge {
        ResizeEdge::Top | ResizeEdge::Bottom => CursorStyle::ResizeUpDown,
        ResizeEdge::Left | ResizeEdge::Right => CursorStyle::ResizeLeftRight,
        ResizeEdge::TopLeft | ResizeEdge::BottomRight => CursorStyle::ResizeUpLeftDownRight,
        ResizeEdge::TopRight | ResizeEdge::BottomLeft => CursorStyle::ResizeUpRightDownLeft,
    }
}

/// A full-window layer that shows resize pointers within `inset` of the
/// window's untiled edges and starts a system resize when pressed there.
///
/// Place it first among the children of the window's root, behind the
/// content, and inset the content by the same amount (as the app's
/// `Window::set_client_inset`). Renders nothing under server-side
/// decorations.
#[derive(IntoElement)]
#[must_use]
pub struct ResizeHandles {
    inset: Pixels,
}

impl ResizeHandles {
    pub const fn new(inset: Pixels) -> Self {
        Self { inset }
    }
}

impl RenderOnce for ResizeHandles {
    fn render(self, window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let inset = self.inset;
        let tiling = client_tiling(window);
        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .when_some(tiling, |d, tiling| {
                d.child(
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
                            let size = window.window_bounds().get_bounds().size;
                            if let Some(edge) =
                                resize_edge(window.mouse_position(), size, tiling, inset)
                            {
                                window.set_cursor_style(resize_cursor(edge), &hitbox);
                            }
                        },
                    )
                    .size_full(),
                )
                // The pointer is set while painting, so repaint as it
                // moves near an edge — including just inside, to clear it
                // again — but not for every move over the content.
                .on_mouse_move(move |event, window, _| {
                    let size = window.window_bounds().get_bounds().size;
                    if resize_edge(event.position, size, tiling, px(f32::from(inset) * 2.0))
                        .is_some()
                    {
                        window.refresh();
                    }
                })
                .on_mouse_down(MouseButton::Left, move |event, window, _| {
                    let size = window.window_bounds().get_bounds().size;
                    if let Some(edge) = resize_edge(event.position, size, tiling, inset) {
                        window.start_window_resize(edge);
                    }
                })
            })
    }
}
