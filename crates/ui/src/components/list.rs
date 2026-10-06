use gpui::{
    AnyElement, App, ClickEvent, ElementId, FontWeight, Hsla, IntoElement, RenderOnce,
    SharedString, Window, div, prelude::*, px,
};

use super::{ClickHandler, icon};
use crate::{Theme, icons};

/// Title + subtitle header of a list column ("Containers · 3 running"), with
/// trailing actions.
#[derive(IntoElement)]
pub struct ListHeader {
    title: SharedString,
    subtitle: Option<SharedString>,
    leading_inset: f32,
    actions: Vec<AnyElement>,
}

impl ListHeader {
    pub fn new(title: impl Into<SharedString>) -> Self {
        Self {
            title: title.into(),
            subtitle: None,
            leading_inset: 0.0,
            actions: Vec::new(),
        }
    }

    pub fn subtitle(mut self, subtitle: impl Into<SharedString>) -> Self {
        self.subtitle = Some(subtitle.into());
        self
    }

    /// Extra left padding, e.g. to clear a floating sidebar toggle.
    pub fn leading_inset(mut self, inset: f32) -> Self {
        self.leading_inset = inset;
        self
    }

    pub fn action(mut self, action: impl IntoElement) -> Self {
        self.actions.push(action.into_any_element());
        self
    }
}

impl RenderOnce for ListHeader {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::of(cx);
        div()
            .h(px(Theme::HEADER_HEIGHT))
            .flex_none()
            .flex()
            .items_center()
            .pl(px(Theme::SPACE_LG + self.leading_inset))
            .pr(px(Theme::SPACE_SM + 2.0))
            .gap(px(Theme::SPACE_XS))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .text_size(px(Theme::TEXT_LG))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.text)
                            .truncate()
                            .child(self.title),
                    )
                    .when_some(self.subtitle, |d, s| {
                        d.child(
                            div()
                                .text_size(px(Theme::TEXT_XS))
                                .text_color(theme.text_faint)
                                .truncate()
                                .child(s),
                        )
                    }),
            )
            .children(self.actions)
    }
}

/// Small muted heading inside a list ("Running", "Stopped").
#[derive(IntoElement)]
pub struct SectionLabel {
    label: SharedString,
    count: Option<usize>,
}

impl SectionLabel {
    pub fn new(label: impl Into<SharedString>) -> Self {
        Self {
            label: label.into(),
            count: None,
        }
    }

    pub fn count(mut self, count: usize) -> Self {
        self.count = Some(count);
        self
    }
}

impl RenderOnce for SectionLabel {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::of(cx);
        div()
            .flex()
            .items_center()
            .gap(px(6.0))
            .px(px(Theme::SPACE_SM + 2.0))
            .pt(px(Theme::SPACE_MD + 2.0))
            .pb(px(Theme::SPACE_XS + 2.0))
            .text_size(px(Theme::TEXT_XS))
            .font_weight(FontWeight::MEDIUM)
            .text_color(theme.text_faint)
            .child(self.label)
            .when_some(self.count, |d, n| {
                d.child(
                    div()
                        .text_color(theme.text_faint.opacity(0.7))
                        .child(n.to_string()),
                )
            })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Running,
    Paused,
    Transitioning,
    Stopped,
    Error,
}

impl Status {
    pub fn color(self, theme: &Theme) -> Hsla {
        match self {
            Status::Running => theme.success,
            Status::Paused | Status::Transitioning => theme.warning,
            Status::Stopped => theme.text_faint.opacity(0.6),
            Status::Error => theme.danger,
        }
    }
}

/// A status dot. With `ring`, it gets a border in `ring` colour so it can sit
/// on top of a badge corner.
pub fn status_dot(status: Status, size: f32, ring: Option<Hsla>, theme: &Theme) -> gpui::Div {
    div()
        .flex_none()
        .size(px(size))
        .rounded_full()
        .bg(status.color(theme))
        .when_some(ring, |d, ring| d.border_2().border_color(ring))
}

/// Leading tile for resource rows: a tinted rounded square with either a
/// monogram or an icon, and an optional status dot on its corner.
#[derive(IntoElement)]
pub struct Badge {
    key: SharedString,
    content: BadgeContent,
    size: f32,
    status: Option<Status>,
    dimmed: bool,
}

enum BadgeContent {
    Monogram(SharedString),
    Icon(&'static str),
}

impl Badge {
    /// Monogram badge; the tint is derived from `key`.
    pub fn monogram(key: impl Into<SharedString>) -> Self {
        let key = key.into();
        let letter = key
            .chars()
            .find(|c| c.is_alphanumeric())
            .map(|c| c.to_uppercase().to_string())
            .unwrap_or_else(|| "?".into());
        Self {
            key,
            content: BadgeContent::Monogram(letter.into()),
            size: 28.0,
            status: None,
            dimmed: false,
        }
    }

    pub fn icon(key: impl Into<SharedString>, icon: &'static str) -> Self {
        Self {
            key: key.into(),
            content: BadgeContent::Icon(icon),
            size: 28.0,
            status: None,
            dimmed: false,
        }
    }

    pub fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }

    pub fn status(mut self, status: Status) -> Self {
        self.status = Some(status);
        self
    }

    pub fn dimmed(mut self, dimmed: bool) -> Self {
        self.dimmed = dimmed;
        self
    }
}

impl RenderOnce for Badge {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::of(cx);
        let tint = theme.tint_for(&self.key);
        let tint = if self.dimmed {
            tint.grayscale().opacity(0.8)
        } else {
            tint
        };
        let size = self.size;
        div()
            .relative()
            .flex_none()
            .size(px(size))
            .child(
                div()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(size * 0.3))
                    .bg(tint.opacity(0.14))
                    .border_1()
                    .border_color(tint.opacity(0.18))
                    .map(|d| match self.content {
                        BadgeContent::Monogram(letter) => d
                            .text_size(px(size * 0.43))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(tint)
                            .child(letter),
                        BadgeContent::Icon(path) => {
                            d.child(icon(path).size(px(size * 0.55)).text_color(tint))
                        }
                    }),
            )
            .when_some(self.status, |d, status| {
                d.child(
                    status_dot(status, 10.0, Some(theme.surface), theme)
                        .absolute()
                        .right(px(-3.0))
                        .bottom(px(-3.0)),
                )
            })
    }
}

/// A two-line resource row (container, image, volume, network).
///
/// Actions are revealed on hover and always shown on the selected row.
#[derive(IntoElement)]
pub struct ResourceRow {
    id: ElementId,
    leading: Option<AnyElement>,
    title: SharedString,
    subtitle: Option<SharedString>,
    meta: Option<SharedString>,
    selected: bool,
    dimmed: bool,
    depth: usize,
    disclosure: Option<(bool, ClickHandler)>,
    reserve_disclosure: bool,
    actions: Vec<AnyElement>,
    on_click: Option<ClickHandler>,
}

impl ResourceRow {
    pub fn new(id: impl Into<ElementId>, title: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            leading: None,
            title: title.into(),
            subtitle: None,
            meta: None,
            selected: false,
            dimmed: false,
            depth: 0,
            disclosure: None,
            reserve_disclosure: false,
            actions: Vec::new(),
            on_click: None,
        }
    }

    pub fn leading(mut self, leading: impl IntoElement) -> Self {
        self.leading = Some(leading.into_any_element());
        self
    }

    pub fn subtitle(mut self, subtitle: impl Into<SharedString>) -> Self {
        self.subtitle = Some(subtitle.into());
        self
    }

    /// Short trailing text (e.g. "2/3"), shown before the actions.
    pub fn meta(mut self, meta: impl Into<SharedString>) -> Self {
        self.meta = Some(meta.into());
        self
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// Stopped / unused resources read one step quieter.
    pub fn dimmed(mut self, dimmed: bool) -> Self {
        self.dimmed = dimmed;
        self
    }

    pub fn depth(mut self, depth: usize) -> Self {
        self.depth = depth;
        self
    }

    /// Reserve the chevron slot so leaf rows align with expandable siblings.
    pub fn reserve_disclosure(mut self) -> Self {
        self.reserve_disclosure = true;
        self
    }

    pub fn disclosure(
        mut self,
        expanded: bool,
        on_toggle: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.disclosure = Some((expanded, Box::new(on_toggle)));
        self
    }

    pub fn action(mut self, action: impl IntoElement) -> Self {
        self.actions.push(action.into_any_element());
        self
    }

    pub fn on_click(mut self, f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Box::new(f));
        self
    }
}

impl RenderOnce for ResourceRow {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::of(cx);
        let group: SharedString = "resource-row".into();
        let (title_color, subtitle_color) = if self.dimmed {
            (theme.text_muted, theme.text_faint)
        } else {
            (theme.text, theme.text_muted)
        };
        let row_hover = theme.row_hover;
        let disclosure_slot = if let Some((expanded, on_toggle)) = self.disclosure {
            Some(
                div()
                    .id("disclosure")
                    .flex_none()
                    .size(px(16.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(4.0))
                    .cursor_pointer()
                    .hover(|s| s.bg(theme.element_hover))
                    .child(
                        icon(if expanded {
                            icons::CHEVRON_DOWN
                        } else {
                            icons::CHEVRON_RIGHT
                        })
                        .size(px(12.0))
                        .text_color(theme.text_faint),
                    )
                    .on_click(move |e, w, cx| {
                        cx.stop_propagation();
                        on_toggle(e, w, cx)
                    })
                    .into_any_element(),
            )
        } else if self.reserve_disclosure {
            Some(div().flex_none().w(px(16.0)).into_any_element())
        } else {
            None
        };
        let on_click = self.on_click;

        div()
            .id(self.id)
            .group(group.clone())
            .h(px(Theme::RESOURCE_ROW_HEIGHT))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(Theme::SPACE_SM + 2.0))
            .pl(px(Theme::SPACE_XS + self.depth as f32 * 22.0))
            .pr(px(Theme::SPACE_XS + 2.0))
            .rounded(px(Theme::ROW_RADIUS))
            .cursor_pointer()
            .map(|d| {
                if self.selected {
                    d.bg(theme.row_selected)
                } else {
                    d.hover(move |s| s.bg(row_hover))
                }
            })
            .children(disclosure_slot)
            .children(self.leading)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(1.0))
                    .child(
                        div()
                            .text_size(px(Theme::TEXT_MD))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(title_color)
                            .truncate()
                            .child(self.title),
                    )
                    .when_some(self.subtitle, |d, s| {
                        d.child(
                            div()
                                .text_size(px(Theme::TEXT_XS + 0.5))
                                .text_color(subtitle_color)
                                .truncate()
                                .child(s),
                        )
                    }),
            )
            .when_some(self.meta, |d, meta| {
                d.child(
                    div()
                        .flex_none()
                        .text_size(px(Theme::TEXT_XS))
                        .text_color(theme.text_faint)
                        .child(meta),
                )
            })
            .when(!self.actions.is_empty(), |d| {
                d.child(
                    div()
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap(px(2.0))
                        .when(!self.selected, |d| {
                            d.opacity(0.0).group_hover(group, |s| s.opacity(1.0))
                        })
                        .children(self.actions),
                )
            })
            .when_some(on_click, |d, f| d.on_click(move |e, w, cx| f(e, w, cx)))
    }
}
