//! Settings-page building blocks: a large page title, muted section labels
//! over filled groups of hairline-split rows, and the pickers that sit in
//! those rows.

use std::rc::Rc;

use gpui::{
    AnyElement, App, ClickEvent, ElementId, FontWeight, Hsla, IntoElement, RenderOnce,
    SharedString, Window, div, prelude::*, px,
};

use super::{ClickHandler, icon, reveal_when_focused, tab_stop};

/// A click handler more than one element can hold.
type SharedClickHandler = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;
use crate::Theme;

/// Page headline.
pub fn page_title(title: impl Into<SharedString>, theme: &Theme) -> gpui::Div {
    div()
        .px(px(Theme::SPACE_XS))
        .pb(px(Theme::SPACE_SM))
        .text_size(px(20.0))
        .line_height(px(26.0))
        .font_weight(FontWeight::MEDIUM)
        .text_color(theme.text)
        .child(title.into())
}

/// Muted label above a group ("Color scheme").
pub fn section_title(title: impl Into<SharedString>, theme: &Theme) -> gpui::Div {
    div()
        .px(px(Theme::SPACE_XS))
        .pt(px(Theme::SPACE_LG + 4.0))
        .pb(px(Theme::SPACE_SM + 2.0))
        .text_size(px(Theme::TEXT_LG))
        .text_color(theme.text_muted)
        .child(title.into())
}

/// A filled card of [`SettingRow`]s separated by hairlines.
#[derive(IntoElement, Default)]
#[must_use]
pub struct SettingsGroup {
    rows: Vec<AnyElement>,
}

impl SettingsGroup {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn row(mut self, row: impl IntoElement) -> Self {
        self.rows.push(row.into_any_element());
        self
    }
}

impl RenderOnce for SettingsGroup {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::of(cx);
        div()
            .flex()
            .flex_col()
            .px(px(Theme::SPACE_LG))
            .rounded(px(Theme::PANEL_RADIUS + 2.0))
            .bg(theme.surface_card)
            .border_1()
            .border_color(theme.border)
            .children(self.rows.into_iter().enumerate().map(|(ix, row)| {
                div()
                    .when(ix > 0, |d| d.border_t_1().border_color(theme.border))
                    .child(row)
            }))
    }
}

/// Title + optional description on the left, a control on the right; an
/// optional full-width body underneath.
#[derive(IntoElement)]
#[must_use]
pub struct SettingRow {
    title: SharedString,
    description: Option<SharedString>,
    control: Option<AnyElement>,
    body: Option<AnyElement>,
}

impl SettingRow {
    pub fn new(title: impl Into<SharedString>) -> Self {
        Self {
            title: title.into(),
            description: None,
            control: None,
            body: None,
        }
    }

    pub fn description(mut self, description: impl Into<SharedString>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn control(mut self, control: impl IntoElement) -> Self {
        self.control = Some(control.into_any_element());
        self
    }

    pub fn body(mut self, body: impl IntoElement) -> Self {
        self.body = Some(body.into_any_element());
        self
    }
}

impl RenderOnce for SettingRow {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::of(cx);
        div()
            .py(px(Theme::SPACE_MD + 2.0))
            .flex()
            .flex_col()
            .gap(px(Theme::SPACE_MD))
            .child(
                div()
                    .min_h(px(28.0))
                    .flex()
                    .items_center()
                    .gap(px(Theme::SPACE_LG))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(2.0))
                            .child(
                                div()
                                    .text_size(px(Theme::TEXT_MD + 0.5))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(theme.text)
                                    .child(self.title),
                            )
                            .when_some(self.description, |d, description| {
                                d.child(
                                    div()
                                        .text_size(px(Theme::TEXT_SM))
                                        .text_color(theme.text_muted)
                                        .child(description),
                                )
                            }),
                    )
                    .when_some(self.control, |d, control| {
                        d.child(div().flex_none().child(control))
                    }),
            )
            .children(self.body)
    }
}

/// An on/off switch. On: accent track, knob at the right.
#[derive(IntoElement)]
#[must_use]
pub struct Switch {
    id: ElementId,
    on: bool,
    on_toggle: Option<ClickHandler>,
}

impl Switch {
    pub fn new(id: impl Into<ElementId>, on: bool) -> Self {
        Self {
            id: id.into(),
            on,
            on_toggle: None,
        }
    }

    pub fn on_toggle(mut self, f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_toggle = Some(Box::new(f));
        self
    }
}

impl RenderOnce for Switch {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        const WIDTH: f32 = 34.0;
        const HEIGHT: f32 = 20.0;
        const KNOB: f32 = 14.0;
        let focus = tab_stop(self.id.clone(), window, cx);
        let theme = Theme::of(cx);
        let inset = (HEIGHT - KNOB) / 2.0 - 1.0;
        let hover_border = theme.text_faint;
        let ring = theme.text;
        div()
            .id(self.id)
            .track_focus(&focus)
            .map(|d| reveal_when_focused(d, &focus, window))
            .focus_visible(move |s| s.border_color(ring))
            .flex_none()
            .w(px(WIDTH))
            .h(px(HEIGHT))
            .flex()
            .items_center()
            .px(px(inset))
            .rounded_full()
            .border_1()
            .cursor_pointer()
            .map(|d| {
                if self.on {
                    d.bg(theme.accent).border_color(theme.accent).justify_end()
                } else {
                    d.bg(theme.element_hover)
                        .border_color(theme.border_strong)
                        .hover(move |s| s.border_color(hover_border))
                }
            })
            .child(
                div()
                    .size(px(KNOB))
                    .rounded_full()
                    // `on_solid` contrasts with every accent in both appearances.
                    .bg(if self.on {
                        theme.on_solid
                    } else {
                        theme.text_muted
                    })
                    .shadow_sm(),
            )
            .when_some(self.on_toggle, |d, f| {
                d.on_click(move |e, w, cx| {
                    cx.stop_propagation();
                    f(e, w, cx);
                })
            })
    }
}

/// One choice in a row of visual options: a preview frame with a caption
/// below. The selected card gets an accent edge and accent caption.
#[derive(IntoElement)]
#[must_use]
pub struct OptionCard {
    id: ElementId,
    preview: AnyElement,
    icon: Option<&'static str>,
    label: SharedString,
    selected: bool,
    on_click: Option<SharedClickHandler>,
}

impl OptionCard {
    pub fn new(
        id: impl Into<ElementId>,
        label: impl Into<SharedString>,
        preview: impl IntoElement,
    ) -> Self {
        Self {
            id: id.into(),
            preview: preview.into_any_element(),
            icon: None,
            label: label.into(),
            selected: false,
            on_click: None,
        }
    }

    pub const fn icon(mut self, icon: &'static str) -> Self {
        self.icon = Some(icon);
        self
    }

    pub const fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    pub fn on_click(mut self, f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Rc::new(f));
        self
    }
}

impl RenderOnce for OptionCard {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let focus = tab_stop(self.id.clone(), window, cx);
        let theme = Theme::of(cx);
        let ring = theme.text;
        let caption = if self.selected {
            theme.accent
        } else {
            theme.text_muted
        };
        let hover_border = theme.border_strong;
        div()
            .id(self.id)
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(Theme::SPACE_SM + 2.0))
            .cursor_pointer()
            .child(
                // The frame is the focus target: Enter/Space click it.
                div()
                    .id("frame")
                    .track_focus(&focus)
                    .map(|d| reveal_when_focused(d, &focus, window))
                    .h(px(OPTION_PREVIEW_HEIGHT))
                    .rounded(px(Theme::PANEL_RADIUS))
                    .border(px(if self.selected { 1.5 } else { 1.0 }))
                    .p(px(3.0))
                    .map(|d| {
                        if self.selected {
                            d.border_color(theme.accent)
                        } else {
                            d.border_color(theme.border)
                                .hover(move |s| s.border_color(hover_border))
                        }
                    })
                    .focus_visible(move |s| s.border_color(ring))
                    .child(self.preview)
                    .when_some(self.on_click.clone(), |d, f| {
                        d.on_click(move |e, w, cx| {
                            cx.stop_propagation();
                            f(e, w, cx);
                        })
                    }),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap(px(6.0))
                    .text_size(px(Theme::TEXT_MD + 0.5))
                    .text_color(caption)
                    .when(self.selected, |d| d.font_weight(FontWeight::MEDIUM))
                    .when_some(self.icon, |d, path| {
                        d.child(icon(path).size(px(15.0)).text_color(caption))
                    })
                    .child(self.label),
            )
            .when_some(self.on_click, |d, f| {
                d.on_click(move |e, w, cx| f(e, w, cx))
            })
    }
}

/// Height of an [`OptionCard`]'s preview frame.
pub const OPTION_PREVIEW_HEIGHT: f32 = 112.0;
/// Radius a preview must round itself to inside an [`OptionCard`] frame:
/// gpui clips to rectangles, not to the frame's radius.
pub const OPTION_PREVIEW_RADIUS: f32 = Theme::PANEL_RADIUS - 3.0;

/// A filled colour swatch; the selected one carries a ring.
#[derive(IntoElement)]
#[must_use]
pub struct ColorSwatch {
    id: ElementId,
    color: Hsla,
    selected: bool,
    on_click: Option<ClickHandler>,
}

impl ColorSwatch {
    pub fn new(id: impl Into<ElementId>, color: Hsla) -> Self {
        Self {
            id: id.into(),
            color,
            selected: false,
            on_click: None,
        }
    }

    pub const fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    pub fn on_click(mut self, f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Box::new(f));
        self
    }
}

impl RenderOnce for ColorSwatch {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let focus = tab_stop(self.id.clone(), window, cx);
        let theme = Theme::of(cx);
        let hover_ring = theme.border_strong;
        let focus_ring = theme.text_muted;
        let selected_ring = theme.accent;
        div()
            .id(self.id)
            .track_focus(&focus)
            .map(|d| reveal_when_focused(d, &focus, window))
            .flex_none()
            .size(px(30.0))
            .p(px(3.0))
            .rounded(px(Theme::ROW_RADIUS + 1.0))
            .border(px(1.5))
            .cursor_pointer()
            .map(|d| {
                if self.selected {
                    d.border_color(theme.text)
                        .focus_visible(move |s| s.border_color(selected_ring))
                } else {
                    d.border_color(gpui::transparent_black())
                        .hover(move |s| s.border_color(hover_ring))
                        .focus_visible(move |s| s.border_color(focus_ring))
                }
            })
            .child(
                div()
                    .size_full()
                    .rounded(px(Theme::CONTROL_RADIUS - 1.0))
                    .bg(self.color),
            )
            .when_some(self.on_click, |d, f| {
                d.on_click(move |e, w, cx| f(e, w, cx))
            })
    }
}
