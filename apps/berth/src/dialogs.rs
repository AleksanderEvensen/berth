//! Requests for shell-presented modals.

use std::rc::Rc;

use gpui::{App, SharedString, Window};

pub type ConfirmHandler = Rc<dyn Fn(&mut Window, &mut App)>;

/// Ask the shell to confirm a destructive action. Screens emit this; the
/// shell shows the dialog and runs `on_confirm` if the user accepts.
#[derive(Clone)]
pub struct ConfirmRequest {
    pub title: SharedString,
    pub message: SharedString,
    /// Affected objects, listed in the dialog (may be empty).
    pub items: Vec<SharedString>,
    pub confirm_label: SharedString,
    pub on_confirm: ConfirmHandler,
}
