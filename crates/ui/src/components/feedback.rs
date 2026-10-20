//! Feedback surfaces: spinner, modal dialog frame, toasts.

use std::time::Duration;

use gpui::{
    Animation, AnimationExt as _, AnyElement, App, Div, ElementId, FontWeight, Hsla, IntoElement,
    RenderOnce, SharedString, Stateful, Transformation, Window, div, percentage, prelude::*, px,
};

use super::{ClickHandler, IconButton, icon};
use crate::{Theme, icons};

/// An indeterminate spinner (rotating arc).
#[derive(IntoElement)]
#[must_use]
pub struct Spinner {
    id: ElementId,
    size: f32,
    color: Option<Hsla>,
}

impl Spinner {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            size: 14.0,
            color: None,
        }
    }

    pub const fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }

    pub const fn color(mut self, color: Hsla) -> Self {
        self.color = Some(color);
        self
    }
}

impl RenderOnce for Spinner {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let color = self.color.unwrap_or_else(|| Theme::of(cx).text_muted);
        icon(icons::SPINNER)
            .size(px(self.size))
            .text_color(color)
            .with_animation(
                self.id,
                Animation::new(Duration::from_millis(800)).repeat(),
                |svg, t| svg.with_transformation(Transformation::rotate(percentage(t))),
            )
    }
}

/// Full-window scrim behind a modal; swallows pointer events behind it.
#[must_use]
pub fn modal_backdrop(theme: &Theme) -> Stateful<Div> {
    div()
        .id("modal-backdrop")
        .size_full()
        .bg(theme.scrim)
        .occlude()
}

/// Key context of a dialog's button row, where Enter presses the focused
/// button rather than confirming the dialog.
pub const DIALOG_BUTTONS_CONTEXT: &str = "DialogButtons";

/// A modal dialog card: title, message, optional body and a right-aligned
/// button row.
#[derive(IntoElement)]
#[must_use]
pub struct Dialog {
    title: SharedString,
    message: Option<SharedString>,
    body: Option<AnyElement>,
    buttons: Vec<AnyElement>,
    width: f32,
}

impl Dialog {
    pub fn new(title: impl Into<SharedString>) -> Self {
        Self {
            title: title.into(),
            message: None,
            body: None,
            buttons: Vec::new(),
            width: 420.0,
        }
    }

    pub fn message(mut self, message: impl Into<SharedString>) -> Self {
        self.message = Some(message.into());
        self
    }

    pub fn body(mut self, body: impl IntoElement) -> Self {
        self.body = Some(body.into_any_element());
        self
    }

    pub fn button(mut self, button: impl IntoElement) -> Self {
        self.buttons.push(button.into_any_element());
        self
    }
}

impl RenderOnce for Dialog {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::of(cx);
        div()
            .w(px(self.width))
            .flex()
            .flex_col()
            .gap(px(Theme::SPACE_MD))
            .p(px(Theme::SPACE_LG + 4.0))
            .rounded(px(14.0))
            .bg(theme.surface_dialog)
            .border_1()
            .border_color(theme.border_strong)
            .shadow_2xl()
            .child(
                div()
                    .text_size(px(Theme::TEXT_LG + 1.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.text)
                    .child(self.title),
            )
            .when_some(self.message, |d, m| {
                d.child(
                    div()
                        .text_size(px(Theme::TEXT_MD))
                        .line_height(px(19.0))
                        .text_color(theme.text_muted)
                        .child(m),
                )
            })
            .children(self.body)
            .child(
                div()
                    .key_context(DIALOG_BUTTONS_CONTEXT)
                    .pt(px(Theme::SPACE_XS))
                    .flex()
                    .justify_end()
                    .gap(px(Theme::SPACE_SM))
                    .children(self.buttons),
            )
    }
}

/// A short list of affected objects inside a dialog ("+N more" beyond `max`).
#[must_use]
pub fn item_list(items: &[SharedString], max: usize, theme: &Theme) -> impl IntoElement {
    let more = items.len().saturating_sub(max);
    div()
        .flex()
        .flex_col()
        .gap(px(2.0))
        .py(px(Theme::SPACE_SM))
        .px(px(Theme::SPACE_MD))
        .rounded(px(Theme::ROW_RADIUS))
        .bg(theme.row_hover)
        .border_1()
        .border_color(theme.border)
        .text_size(px(Theme::TEXT_SM))
        .font_family(theme.font_mono.clone())
        .text_color(theme.text)
        .children(
            items
                .iter()
                .take(max)
                .map(|item| div().truncate().child(item.clone())),
        )
        .when(more > 0, |d| {
            d.child(
                div()
                    .text_color(theme.text_faint)
                    .child(format!("+{more} more")),
            )
        })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastKind {
    Error,
    Info,
}

/// A floating notification pill, announced to screen readers as an alert
/// ([`gpui_base::Toast`]).
#[derive(IntoElement)]
#[must_use]
pub struct Toast {
    id: ElementId,
    kind: ToastKind,
    title: SharedString,
    detail: Option<SharedString>,
    on_dismiss: Option<ClickHandler>,
}

impl Toast {
    pub const WIDTH: f32 = 440.0;
    /// Height of a toast whose detail fits one line.
    pub const ONE_LINE_HEIGHT: f32 = 62.0;

    pub fn new(id: impl Into<ElementId>, kind: ToastKind, title: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            kind,
            title: title.into(),
            detail: None,
            on_dismiss: None,
        }
    }

    pub fn detail(mut self, detail: impl Into<SharedString>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn on_dismiss(
        mut self,
        f: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_dismiss = Some(Box::new(f));
        self
    }
}

impl RenderOnce for Toast {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::of(cx);
        let accent = match self.kind {
            ToastKind::Error => theme.danger,
            ToastKind::Info => theme.accent,
        };
        let on_dismiss = self.on_dismiss;
        gpui_base::Toast::new(self.id)
            .w(px(Self::WIDTH))
            .flex()
            .items_start()
            .gap(px(Theme::SPACE_SM + 2.0))
            .pl(px(Theme::SPACE_MD))
            .pr(px(Theme::SPACE_SM))
            .py(px(Theme::SPACE_SM + 2.0))
            .rounded(px(Theme::PANEL_RADIUS + 2.0))
            .bg(theme.surface_overlay)
            .border_1()
            .border_color(theme.border_strong)
            .shadow_lg()
            .occlude()
            .child(
                div()
                    .pt(px(1.0))
                    .child(icon(icons::DANGER).size(px(16.0)).text_color(accent)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .child(
                        div()
                            .text_size(px(Theme::TEXT_MD))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme.text)
                            .child(self.title),
                    )
                    .when_some(self.detail, |d, detail| {
                        d.child(
                            div()
                                .text_size(px(Theme::TEXT_SM))
                                .line_height(px(17.0))
                                .text_color(theme.text_muted)
                                .child(detail),
                        )
                    }),
            )
            .when_some(on_dismiss, |d, f| {
                d.child(
                    IconButton::new("dismiss", icons::CLOSE, "Dismiss")
                        .size(22.0, 13.0)
                        .on_click(f),
                )
            })
    }
}
