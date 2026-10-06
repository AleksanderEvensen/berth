use gpui::{
    App, ClickEvent, ElementId, Hsla, IntoElement, RenderOnce, SharedString, Window, div,
    prelude::*, px,
};

use super::{ClickHandler, icon};
use crate::Theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ButtonTone {
    #[default]
    Default,
    /// Tints red on hover — destructive row actions.
    Danger,
}

/// A square, ghost icon button: transparent at rest, white-ink wash on hover.
#[derive(IntoElement)]
#[must_use]
pub struct IconButton {
    id: ElementId,
    icon: &'static str,
    size: f32,
    icon_size: f32,
    tone: ButtonTone,
    color: Option<Hsla>,
    active: bool,
    disabled: bool,
    on_click: Option<ClickHandler>,
}

impl IconButton {
    pub fn new(id: impl Into<ElementId>, icon: &'static str) -> Self {
        Self {
            id: id.into(),
            icon,
            size: 26.0,
            icon_size: 15.0,
            tone: ButtonTone::Default,
            color: None,
            active: false,
            disabled: false,
            on_click: None,
        }
    }

    pub const fn size(mut self, size: f32, icon_size: f32) -> Self {
        self.size = size;
        self.icon_size = icon_size;
        self
    }

    pub const fn tone(mut self, tone: ButtonTone) -> Self {
        self.tone = tone;
        self
    }

    /// Resting icon colour (defaults to `text_muted`).
    pub const fn color(mut self, color: Hsla) -> Self {
        self.color = Some(color);
        self
    }

    pub const fn active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }

    pub const fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// The click does not bubble: an icon button inside a clickable row must
    /// not also select the row.
    pub fn on_click(mut self, f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Box::new(f));
        self
    }
}

impl RenderOnce for IconButton {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::of(cx);
        let rest = if self.disabled {
            theme.text_faint.opacity(0.5)
        } else {
            self.color.unwrap_or(theme.text_muted)
        };
        let (hover_foreground, hover_background) = match self.tone {
            ButtonTone::Default => (theme.text, theme.element_hover),
            ButtonTone::Danger => (theme.danger, theme.danger_wash),
        };
        let group: SharedString = "icon-button".into();
        let on_click = self.on_click;

        div()
            .id(self.id)
            .group(group.clone())
            .flex_none()
            .size(px(self.size))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(Theme::CONTROL_RADIUS))
            .when(self.active, |d| d.bg(theme.element_hover))
            .when(!self.disabled, |d| {
                d.cursor_pointer()
                    .hover(move |s| s.bg(hover_background))
                    .active(move |s| s.opacity(0.8))
            })
            .child(
                icon(self.icon)
                    .size(px(self.icon_size))
                    .text_color(if self.active { theme.text } else { rest })
                    .when(!self.disabled, |s| {
                        s.group_hover(group, move |s| s.text_color(hover_foreground))
                    }),
            )
            .when_some(on_click.filter(|_| !self.disabled), |d, f| {
                d.on_click(move |e, w, cx| {
                    cx.stop_propagation();
                    f(e, w, cx);
                })
            })
            // Keep presses out of a surrounding drag region.
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
    }
}

/// A text button. `Solid` is the high-contrast primary plate (zeron's
/// `solid`/`on_solid`), `Ghost` is a bordered secondary.
#[derive(IntoElement)]
#[must_use]
pub struct Button {
    id: ElementId,
    label: SharedString,
    icon: Option<&'static str>,
    solid: bool,
    tone: ButtonTone,
    on_click: Option<ClickHandler>,
}

impl Button {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            icon: None,
            solid: false,
            tone: ButtonTone::Default,
            on_click: None,
        }
    }

    pub const fn icon(mut self, icon: &'static str) -> Self {
        self.icon = Some(icon);
        self
    }

    pub const fn solid(mut self) -> Self {
        self.solid = true;
        self
    }

    pub const fn tone(mut self, tone: ButtonTone) -> Self {
        self.tone = tone;
        self
    }

    pub fn on_click(mut self, f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Box::new(f));
        self
    }
}

impl RenderOnce for Button {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::of(cx);
        let (bg, fg, border, hover_background) = match (self.solid, self.tone) {
            (true, ButtonTone::Danger) => (
                theme.danger_strong,
                gpui::white(),
                theme.danger_strong,
                theme.danger_strong.opacity(0.88),
            ),
            (true, ButtonTone::Default) => (
                theme.solid,
                theme.on_solid,
                theme.solid,
                theme.solid.opacity(0.88),
            ),
            (false, ButtonTone::Default) => (
                theme.row_hover,
                theme.text,
                theme.border,
                theme.element_hover,
            ),
            (false, ButtonTone::Danger) => (
                theme.danger_wash,
                theme.danger,
                theme.danger.opacity(0.25),
                theme.danger_wash.opacity(0.28),
            ),
        };
        let on_click = self.on_click;
        div()
            .id(self.id)
            .h(px(30.0))
            .px(px(12.0))
            .flex()
            .flex_none()
            .items_center()
            .gap(px(6.0))
            .rounded(px(Theme::CONTROL_RADIUS + 1.0))
            .border_1()
            .border_color(border)
            .bg(bg)
            .text_color(fg)
            .text_size(px(Theme::TEXT_MD))
            .font_weight(gpui::FontWeight::MEDIUM)
            .cursor_pointer()
            .hover(move |s| s.bg(hover_background))
            .when_some(self.icon, |d, path| {
                d.child(icon(path).size(px(14.0)).text_color(fg))
            })
            .child(self.label)
            .when_some(on_click, |d, f| d.on_click(move |e, w, cx| f(e, w, cx)))
    }
}
