use std::rc::Rc;

use gpui::{
    App, ElementId, FontWeight, IntoElement, RenderOnce, SharedString, Window, div, prelude::*, px,
};

use super::{reveal_when_focused, tab_stop};
use crate::Theme;

type SelectHandler = Rc<dyn Fn(&usize, &mut Window, &mut App) + 'static>;

/// A pill-shaped segmented control (Info · Stats · Logs · Terminal · Files).
#[derive(IntoElement)]
#[must_use]
pub struct SegmentedTabs {
    id: ElementId,
    labels: Vec<SharedString>,
    selected: usize,
    on_select: Option<SelectHandler>,
}

impl SegmentedTabs {
    pub fn new(
        id: impl Into<ElementId>,
        labels: impl IntoIterator<Item = impl Into<SharedString>>,
    ) -> Self {
        Self {
            id: id.into(),
            labels: labels.into_iter().map(Into::into).collect(),
            selected: 0,
            on_select: None,
        }
    }

    pub const fn selected(mut self, index: usize) -> Self {
        self.selected = index;
        self
    }

    pub fn on_select(mut self, f: impl Fn(&usize, &mut Window, &mut App) + 'static) -> Self {
        self.on_select = Some(Rc::new(f));
        self
    }
}

impl RenderOnce for SegmentedTabs {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let focus: Vec<_> = (0..self.labels.len())
            .map(|ix| {
                let key = ElementId::NamedInteger(
                    format!("{}-segment", self.id).into(),
                    u64::try_from(ix).unwrap_or_default(),
                );
                tab_stop(key, window, cx)
            })
            .collect();
        let theme = Theme::of(cx);
        let text_hover = theme.text;
        let ring = theme.border_strong;
        div()
            .id(self.id)
            .flex()
            .flex_none()
            .items_center()
            .gap(px(2.0))
            .p(px(3.0))
            .rounded(px(Theme::PANEL_RADIUS))
            .bg(theme.row_hover)
            .border_1()
            .border_color(theme.border)
            .children(self.labels.into_iter().enumerate().map(|(ix, label)| {
                let selected = ix == self.selected;
                let on_select = self.on_select.clone();
                div()
                    .id(ix)
                    .when_some(focus.get(ix), |d, focus| {
                        reveal_when_focused(d, focus, window)
                            .track_focus(focus)
                            .border_1()
                            .border_color(gpui::transparent_black())
                            .focus_visible(move |s| s.border_color(ring))
                    })
                    .h(px(26.0))
                    .px(px(Theme::SPACE_MD))
                    .flex()
                    .items_center()
                    .rounded(px(Theme::CONTROL_RADIUS + 1.0))
                    .text_size(px(Theme::TEXT_MD))
                    .font_weight(FontWeight::MEDIUM)
                    .cursor_pointer()
                    .map(|d| {
                        if selected {
                            d.bg(theme.surface_raised)
                                .text_color(theme.text)
                                .shadow_sm()
                        } else {
                            d.text_color(theme.text_muted)
                                .hover(move |s| s.text_color(text_hover))
                        }
                    })
                    .child(label)
                    .when_some(on_select, |d, f| d.on_click(move |_, w, cx| f(&ix, w, cx)))
            }))
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
    }
}
