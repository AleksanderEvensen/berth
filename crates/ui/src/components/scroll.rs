//! Keyboard reach for scrolled content: a [`ScrollPane`] scrolls a
//! keyboard-focused control into view ([`reveal_when_focused`]) and is the
//! target of the page keys ([`scroll_active_pane`]). Panes that scroll
//! otherwise (a virtualized list) join in with [`active_pane`].

use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    AnyElement, App, Bounds, ElementId, FocusHandle, Global, IntoElement, ParentElement, Pixels,
    RenderOnce, ScrollHandle, WeakFocusHandle, Window, canvas, div, point, prelude::*, px,
};

/// Space kept between a revealed control and the pane's edge.
const REVEAL_MARGIN: f32 = 12.0;
/// How far the line keys scroll.
const LINE: f32 = 40.0;

/// Per-frame bookkeeping shared by panes and focused controls.
#[derive(Default)]
struct PaneFrame {
    /// The keyboard-focused control prepainted in the pane being laid out.
    focused: Option<(WeakFocusHandle, Bounds<Pixels>)>,
    /// The last pane prepainted: the main content pane, for the page keys.
    /// Without a handle, the pane scrolls itself in its callback.
    active: Option<(Option<ScrollHandle>, Option<OnScroll>)>,
}

/// Told after the keys scroll a pane registered with [`active_pane`].
pub type OnScroll = Rc<dyn Fn(ScrollStep, &mut Window, &mut App)>;

impl Global for PaneFrame {}

/// While `focus` has keyboard focus, report `element`'s bounds so the
/// enclosing [`ScrollPane`] can scroll it into view.
pub fn reveal_when_focused<E: ParentElement>(
    element: E,
    focus: &FocusHandle,
    window: &Window,
) -> E {
    if !focus.is_focused(window) || !window.last_input_was_keyboard() {
        return element;
    }
    let focus = focus.downgrade();
    element.child(
        canvas(
            move |bounds, _, cx| {
                cx.default_global::<PaneFrame>().focused = Some((focus, bounds));
            },
            |_, (), _, _| {},
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full(),
    )
}

/// An empty element that makes a pane the target of the page keys.
///
/// While it is laid out, the keys scroll `handle` and then tell
/// `on_scroll`; with no handle (a `gpui::list`), `on_scroll` does the
/// scrolling. Add it after the scrolled content.
pub fn active_pane(handle: Option<ScrollHandle>, on_scroll: OnScroll) -> impl IntoElement {
    canvas(
        move |_, _, cx| {
            cx.default_global::<PaneFrame>().active = Some((handle, Some(on_scroll)));
        },
        |_, (), _, _| {},
    )
    .absolute()
    .size(px(0.0))
}

/// Where a keyboard scroll moves the active pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollStep {
    LineUp,
    LineDown,
    PageUp,
    PageDown,
    Top,
    Bottom,
}

/// Scroll the pane last laid out (the main content pane) by `step`.
pub fn scroll_active_pane(step: ScrollStep, window: &mut Window, cx: &mut App) {
    let Some((handle, on_scroll)) = cx.default_global::<PaneFrame>().active.clone() else {
        return;
    };
    if let Some(handle) = handle {
        scroll_handle(&handle, step, window);
    }
    if let Some(on_scroll) = on_scroll {
        on_scroll(step, window, cx);
    }
}

fn scroll_handle(handle: &ScrollHandle, step: ScrollStep, window: &mut Window) {
    let page = f32::from(handle.bounds().size.height) - LINE;
    let max = -f32::from(handle.max_offset().y);
    let y = f32::from(handle.offset().y);
    let y = match step {
        ScrollStep::LineUp => y + LINE,
        ScrollStep::LineDown => y - LINE,
        ScrollStep::PageUp => y + page,
        ScrollStep::PageDown => y - page,
        ScrollStep::Top => 0.0,
        ScrollStep::Bottom => max,
    };
    handle.set_offset(point(px(0.0), px(y.clamp(max.min(0.0), 0.0))));
    window.refresh();
}

#[derive(Clone, Default)]
struct PaneState {
    handle: ScrollHandle,
    /// The focused control last revealed, so it is revealed once rather
    /// than pinned while the user scrolls away from it.
    revealed: Rc<Cell<Option<WeakFocusHandle>>>,
}

/// A vertically scrolling container whose content follows keyboard focus.
#[derive(IntoElement)]
#[must_use]
pub struct ScrollPane {
    id: ElementId,
    content: AnyElement,
}

impl ScrollPane {
    /// `id` keys the scroll position; unique among siblings.
    pub fn new(id: impl Into<ElementId>, content: impl IntoElement) -> Self {
        Self {
            id: id.into(),
            content: content.into_any_element(),
        }
    }
}

impl RenderOnce for ScrollPane {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window
            .use_keyed_state(self.id.clone(), cx, |_, _| PaneState::default())
            .read(cx)
            .clone();
        let handle = state.handle.clone();
        div()
            .id(self.id)
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&handle)
            // Content prepaints between these two hooks, so a focused
            // control reported in between lies inside this pane.
            .child(
                canvas(
                    |_, _, cx| cx.default_global::<PaneFrame>().focused = None,
                    |_, (), _, _| {},
                )
                .absolute()
                .size(px(0.0)),
            )
            .child(self.content)
            .child(
                canvas(
                    move |_, window, cx| {
                        let frame = cx.default_global::<PaneFrame>();
                        frame.active = Some((Some(state.handle.clone()), None));
                        let focused = frame.focused.take();
                        reveal(&state, focused, window);
                    },
                    |_, (), _, _| {},
                )
                .absolute()
                .size(px(0.0)),
            )
    }
}

fn reveal(state: &PaneState, focused: Option<(WeakFocusHandle, Bounds<Pixels>)>, window: &Window) {
    let Some((focus, bounds)) = focused else {
        state.revealed.set(None);
        return;
    };
    let previous = state.revealed.replace(Some(focus.clone()));
    if previous.is_some_and(|p| p == focus) {
        return;
    }
    let view = state.handle.bounds();
    let y = f32::from(state.handle.offset().y);
    let above = f32::from(view.top()) - f32::from(bounds.top());
    let below = f32::from(bounds.bottom()) - f32::from(view.bottom());
    let y = if above > 0.0 {
        y + above + REVEAL_MARGIN
    } else if below > 0.0 {
        y - below - REVEAL_MARGIN
    } else {
        return;
    };
    let max = -f32::from(state.handle.max_offset().y);
    state
        .handle
        .set_offset(point(px(0.0), px(y.clamp(max.min(0.0), 0.0))));
    // This runs mid-draw, where a refresh is ignored; redraw next frame.
    window.on_next_frame(|window, _| window.refresh());
}
