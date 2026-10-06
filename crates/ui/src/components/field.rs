use gpui::{App, Entity, Focusable as _, IntoElement, RenderOnce, Window, div, prelude::*, px};
use gpui_base::input::InputState;

use super::{ClickHandler, IconButton, icon};
use crate::{Theme, icons};

/// A compact search field: magnifier, a `gpui-base` text input, and a clear
/// button while it has a value.
#[derive(IntoElement)]
pub struct SearchField {
    state: Entity<InputState>,
    on_clear: Option<ClickHandler>,
}

impl SearchField {
    pub fn new(state: &Entity<InputState>) -> Self {
        Self {
            state: state.clone(),
            on_clear: None,
        }
    }

    pub fn on_clear(
        mut self,
        f: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_clear = Some(Box::new(f));
        self
    }
}

impl RenderOnce for SearchField {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::of(cx);
        let state = self.state.read(cx);
        let focused = state.focus_handle(cx).is_focused(window);
        let has_value = !state.value().is_empty();
        div()
            .h(px(30.0))
            .flex()
            .items_center()
            .gap(px(Theme::SPACE_SM))
            .pl(px(Theme::SPACE_SM + 2.0))
            .pr(px(Theme::SPACE_XS))
            .rounded(px(Theme::ROW_RADIUS))
            .bg(theme.row_hover)
            .border_1()
            .border_color(if focused {
                theme.accent.opacity(0.55)
            } else {
                theme.border
            })
            .text_size(px(Theme::TEXT_MD))
            .text_color(theme.text)
            .child(
                icon(icons::SEARCH)
                    .size(px(14.0))
                    .text_color(theme.text_faint),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(gpui_base::Input::new(&self.state)),
            )
            .when_some(self.on_clear.filter(|_| has_value), |d, f| {
                d.child(
                    IconButton::new("search-clear", icons::CLOSE)
                        .size(22.0, 12.0)
                        .on_click(f),
                )
            })
    }
}
