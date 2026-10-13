//! Titlebar drag regions.

use std::cell::Cell;

use gpui::{Div, MouseButton, Stateful, prelude::*};

use crate::actions;

thread_local! {
    /// Armed by a press on a drag region; the first move with the button
    /// held hands the drag to the system. Lets a double-click arrive
    /// instead of being swallowed by a move session.
    static SHOULD_MOVE: Cell<bool> = const { Cell::new(false) };
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
        .on_mouse_down(MouseButton::Left, |_, _, _| SHOULD_MOVE.set(true))
        .on_mouse_down_out(|_, _, _| SHOULD_MOVE.set(false))
        .on_mouse_up(MouseButton::Left, |_, _, _| SHOULD_MOVE.set(false))
        .on_mouse_move(|event, window, _| {
            if SHOULD_MOVE.get() && event.pressed_button == Some(MouseButton::Left) {
                SHOULD_MOVE.set(false);
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
