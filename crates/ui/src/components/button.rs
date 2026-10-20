use gpui::{
    App, ClickEvent, ElementId, Hsla, IntoElement, RenderOnce, SharedString, Window, div,
    prelude::*, px,
};

use super::{ClickHandler, icon, reveal_when_focused, tab_stop};
use crate::{Theme, icons};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ButtonTone {
    #[default]
    Default,
    /// Tints red on hover — destructive row actions.
    Danger,
}

/// A square, ghost icon button: transparent at rest, white-ink wash on hover.
/// Behaviour (role, keyboard activation, disabled state) is
/// [`gpui_base::Button`]'s.
#[derive(IntoElement)]
#[must_use]
pub struct IconButton {
    id: ElementId,
    icon: &'static str,
    label: SharedString,
    size: f32,
    icon_size: f32,
    tone: ButtonTone,
    color: Option<Hsla>,
    active: bool,
    disabled: bool,
    tab_stop: bool,
    on_click: Option<ClickHandler>,
}

impl IconButton {
    /// `label` names the button for screen readers, which can't see the icon.
    pub fn new(
        id: impl Into<ElementId>,
        icon: &'static str,
        label: impl Into<SharedString>,
    ) -> Self {
        Self {
            id: id.into(),
            icon,
            label: label.into(),
            size: 26.0,
            icon_size: 15.0,
            tone: ButtonTone::Default,
            color: None,
            active: false,
            disabled: false,
            tab_stop: true,
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

    /// Whether Tab stops here (default). Row actions opt out: rows can be
    /// long, and their actions have list shortcuts instead.
    pub const fn tab_stop(mut self, tab_stop: bool) -> Self {
        self.tab_stop = tab_stop;
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
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let focus = (self.tab_stop && !self.disabled && self.on_click.is_some())
            .then(|| tab_stop(self.id.clone(), window, cx));
        let theme = Theme::of(cx);
        let ring = theme.accent;
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

        gpui_base::Button::new(self.id)
            .accessibility_label(self.label)
            .disabled(self.disabled)
            // Without a tab stop it takes no focus at all, not even on
            // click, so pressing a row action leaves focus on the list.
            .focusable(focus.is_some())
            .group(group.clone())
            .flex_none()
            .size(px(self.size))
            .rounded(px(Theme::CONTROL_RADIUS))
            .when_some(focus, |b, focus| {
                reveal_when_focused(b, &focus, window)
                    .track_focus(&focus)
                    .border_1()
                    .border_color(gpui::transparent_black())
                    .focus_visible(move |s| s.border_color(ring))
            })
            .when(self.active, |b| b.bg(theme.element_hover))
            .when(!self.disabled, |b| {
                b.cursor_pointer()
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
            .when_some(on_click, |b, f| {
                b.on_click(move |e, w, cx| {
                    cx.stop_propagation();
                    f(e, w, cx);
                })
            })
            // Keep presses out of a surrounding drag region.
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
    }
}

/// A text button. `Solid` is the high-contrast primary plate, `Ghost` is a
/// bordered secondary. Behaviour is [`gpui_base::Button`]'s.
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
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let focus = tab_stop(self.id.clone(), window, cx);
        let theme = Theme::of(cx);
        let ring = theme.accent;
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
        gpui_base::Button::new(self.id)
            .accessibility_label(self.label.clone())
            .track_focus(&focus)
            .map(|b| reveal_when_focused(b, &focus, window))
            .focus_visible(move |s| s.border_color(ring))
            .h(px(30.0))
            .px(px(12.0))
            .flex_none()
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
            .when_some(on_click, |b, f| b.on_click(move |e, w, cx| f(e, w, cx)))
    }
}

/// A square checkbox. Checked: accent plate with a check glyph.
#[derive(IntoElement)]
#[must_use]
pub struct Checkbox {
    id: ElementId,
    checked: bool,
    disabled: bool,
    on_toggle: Option<ClickHandler>,
}

impl Checkbox {
    pub fn new(id: impl Into<ElementId>, checked: bool) -> Self {
        Self {
            id: id.into(),
            checked,
            disabled: false,
            on_toggle: None,
        }
    }

    pub const fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn on_toggle(mut self, f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_toggle = Some(Box::new(f));
        self
    }
}

impl RenderOnce for Checkbox {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::of(cx);
        let hover_border = theme.text_faint;
        div()
            .id(self.id)
            .flex_none()
            .size(px(16.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(4.0))
            .border_1()
            .map(|d| {
                if self.checked {
                    d.bg(theme.accent).border_color(theme.accent)
                } else {
                    d.bg(theme.row_hover).border_color(theme.border_strong)
                }
            })
            .when(self.disabled, |d| d.opacity(0.4))
            .when(!self.disabled, |d| {
                d.cursor_pointer()
                    .hover(move |s| s.border_color(hover_border))
            })
            .when(self.checked, |d| {
                d.child(icon(icons::CHECK).size(px(12.0)).text_color(theme.on_solid))
            })
            .when_some(self.on_toggle.filter(|_| !self.disabled), |d, f| {
                d.on_click(move |e, w, cx| {
                    cx.stop_propagation();
                    f(e, w, cx);
                })
            })
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use gpui::{
        Context, ElementId, KeyDownEvent, KeyUpEvent, Keystroke, Modifiers, Render, Role,
        TestAppContext, VisualTestContext,
    };
    use gpui_base::test_support::{ElementSnapshot, snapshots};

    use super::*;
    use crate::ScrollPane;

    #[derive(Default)]
    struct Clicks {
        save: Rc<Cell<usize>>,
        row: Rc<Cell<usize>>,
    }

    /// A text button below the fold, an icon button, and a row action that
    /// is no tab stop, in a scrolled pane.
    struct Buttons(Clicks);

    impl Render for Buttons {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let (save, row) = (self.0.save.clone(), self.0.row.clone());
            ScrollPane::new(
                "pane",
                div()
                    .child(
                        IconButton::new("row-delete", icons::TRASH, "Delete")
                            .tab_stop(false)
                            .on_click(move |_, _, _| row.set(row.get().wrapping_add(1))),
                    )
                    .child(div().h(px(4000.0)))
                    .child(IconButton::new("copy", icons::COPY, "Copy").on_click(|_, _, _| {}))
                    .child(
                        Button::new("save", "Save")
                            .on_click(move |_, _, _| save.set(save.get().wrapping_add(1))),
                    ),
            )
        }
    }

    fn open(cx: &mut TestAppContext) -> (Clicks, &mut VisualTestContext) {
        cx.update(|cx| {
            gpui_base::init(cx);
            crate::install_theme(Theme::dark(), cx);
        });
        let clicks = Clicks::default();
        let (save, row) = (clicks.save.clone(), clicks.row.clone());
        let (_, cx) = cx.add_window_view(|_, _| Buttons(Clicks { save, row }));
        cx.update(|window, _| window.activate_window());
        cx.run_until_parked();
        (clicks, cx)
    }

    /// The last painted facts of the element keyed `id`.
    fn snapshot(id: &str, cx: &mut VisualTestContext) -> ElementSnapshot {
        let id = ElementId::from(SharedString::from(id.to_string()));
        cx.update(|window, _| {
            snapshots(window)
                .into_iter()
                .find(|s| s.path().last() == Some(&id))
        })
        .unwrap_or_else(|| panic!("{id:?} wasn't painted"))
    }

    /// Down and up: a focused button clicks on the key's release.
    fn press(key: &str, cx: &mut VisualTestContext) {
        let keystroke = Keystroke::parse(key).unwrap_or_else(|e| panic!("{e}"));
        cx.simulate_event(KeyDownEvent {
            keystroke: keystroke.clone(),
            is_held: false,
            prefer_character_input: false,
        });
        cx.simulate_event(KeyUpEvent { keystroke });
    }

    #[gpui::test]
    fn buttons_report_their_role_and_label(cx: &mut TestAppContext) {
        let (_, cx) = open(cx);
        for (id, label) in [("save", "Save"), ("copy", "Copy"), ("row-delete", "Delete")] {
            let button = snapshot(id, cx);
            assert_eq!(button.role(), Some(Role::Button), "{id}");
            assert_eq!(button.label(), Some(label), "{id}");
        }
    }

    #[gpui::test]
    fn tab_reveals_a_button_and_enter_presses_it(cx: &mut TestAppContext) {
        let (clicks, cx) = open(cx);
        let viewport = cx.update(|window, _| window.viewport_size());
        assert!(snapshot("save", cx).bounds().top() > viewport.height);

        // The row action is no tab stop: from the start, Tab passes it.
        cx.simulate_keystrokes("tab");
        cx.update(|window, cx| {
            window.focus_next(cx);
            window.focus_next(cx);
        });
        cx.run_until_parked();
        // The pane scrolls it into view the frame after it is focused.
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        let save = snapshot("save", cx);
        assert_eq!(save.focused(), Some(true));
        assert!(save.bounds().bottom() <= viewport.height, "{save:?}");

        press("enter", cx);
        assert_eq!(clicks.save.get(), 1);
        press("space", cx);
        assert_eq!(clicks.save.get(), 2);
    }

    #[gpui::test]
    fn pressing_a_row_action_leaves_focus_alone(cx: &mut TestAppContext) {
        let (clicks, cx) = open(cx);
        let row = snapshot("row-delete", cx);
        cx.simulate_click(row.bounds().center(), Modifiers::default());
        assert_eq!(clicks.row.get(), 1);
        assert_eq!(cx.update(|window, cx| window.focused(cx)), None);
    }
}
