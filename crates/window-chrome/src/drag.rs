//! Titlebar drag regions.

use std::cell::Cell;

use gpui::{Div, MouseButton, Pixels, Point, Stateful, prelude::*};

use crate::actions;

/// How far the pointer must travel with the button held before the window
/// moves; gpui's own drag threshold.
const DRAG_THRESHOLD: f32 = 2.;

thread_local! {
    /// Where a press on a drag region landed; the first move past
    /// [`DRAG_THRESHOLD`] with the button held hands the drag to the
    /// system. Lets a double-click arrive, jitter and all, instead of
    /// being swallowed by a move session.
    static PRESSED_AT: Cell<Option<Point<Pixels>>> = const { Cell::new(None) };
}

fn past_threshold(press: Point<Pixels>, now: Point<Pixels>) -> bool {
    let dx = f32::from(now.x) - f32::from(press.x);
    let dy = f32::from(now.y) - f32::from(press.y);
    dx.hypot(dy) > DRAG_THRESHOLD
}

/// Make `el` behave like a titlebar.
///
/// Dragging moves the window, a double-click does the platform's titlebar
/// double-click and a right click opens the window menu. Children that
/// handle the mouse themselves (buttons, tabs) should stop propagation of
/// their presses.
#[must_use]
pub fn drag_region(id: &'static str, el: Div) -> Stateful<Div> {
    let el = el
        .id(id)
        .on_mouse_down(MouseButton::Left, |event, _, _| {
            PRESSED_AT.set(Some(event.position));
        })
        .on_mouse_down_out(|_, _, _| PRESSED_AT.set(None))
        .on_mouse_up(MouseButton::Left, |_, _, _| PRESSED_AT.set(None))
        .on_mouse_move(|event, window, _| {
            if event.pressed_button == Some(MouseButton::Left)
                && PRESSED_AT
                    .get()
                    .is_some_and(|press| past_threshold(press, event.position))
            {
                PRESSED_AT.set(None);
                actions::start_move(window);
            }
        })
        .on_click(|event, window, _| {
            if event.click_count() == 2 {
                actions::titlebar_double_click(window);
            }
        });
    // Windows opens the window menu on release, as its own titlebar does.
    #[cfg(windows)]
    let el = el.on_mouse_up(MouseButton::Right, |event, window, _| {
        actions::show_window_menu(window, event.position);
    });
    #[cfg(not(windows))]
    let el = el.on_mouse_down(MouseButton::Right, |event, window, _| {
        actions::show_window_menu(window, event.position);
    });
    el
}

#[cfg(test)]
mod tests {
    use gpui::point;
    use gpui::px;

    use super::past_threshold;

    #[test]
    fn jitter_between_double_click_presses_does_not_drag() {
        let press = point(px(100.), px(10.));
        assert!(!past_threshold(press, press));
        assert!(!past_threshold(press, point(px(101.), px(11.))));
        assert!(!past_threshold(press, point(px(98.), px(10.))));
    }

    #[test]
    fn real_drag_moves() {
        let press = point(px(100.), px(10.));
        assert!(past_threshold(press, point(px(103.), px(10.))));
        assert!(past_threshold(press, point(px(98.), px(8.))));
    }
}
