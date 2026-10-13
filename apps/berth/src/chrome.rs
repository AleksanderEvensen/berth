//! Berth's window chrome: the client-side frame, caption buttons and the
//! room they take, built from [`gpui_window_chrome`]'s pieces in Berth's
//! style.

use berth_settings::WindowButtons;
use berth_ui::{Appearance, TabStop, Theme};
use gpui::{
    AnyElement, App, Div, Hsla, IntoElement, MouseButton, PathBuilder, Pixels, Point, Window,
    canvas, div, prelude::*, px, transparent_black,
};
use gpui_window_chrome::{
    ButtonPolicy, CaptionButton, CaptionLayout, NativeButtons, NativeRow, ResizeHandles, Side,
    caption_layout, resize, traffic_lights_end,
};

pub use gpui_window_chrome::drag_region;

use crate::preferences;

/// Left edge of our own leading titlebar controls when nothing native
/// occupies the corner.
pub const TITLEBAR_LEADING: f32 = Theme::SPACE_SM + 2.0;

/// Width of the invisible resize band (and shadow) around a CSD window.
const FRAME_INSET: Pixels = px(10.0);
/// Corner radius of a floating CSD window.
const WINDOW_RADIUS: f32 = 10.0;

const CAPTION_GROUP: &str = "caption-button";
/// Width of one Windows caption button, as in Windows 11.
#[cfg(windows)]
const WINDOWS_CAPTION_WIDTH: f32 = 46.0;
/// Side of one of our own Linux buttons.
#[cfg_attr(windows, allow(dead_code, reason = "Windows has its own buttons"))]
const DRAWN_SIZE: f32 = 24.0;
/// Gap between our own Linux buttons.
#[cfg_attr(windows, allow(dead_code, reason = "Windows has its own buttons"))]
const DRAWN_SPACING: f32 = 8.0;

/// Where the macOS traffic lights go, centred in our header.
pub fn traffic_light_position() -> Point<Pixels> {
    gpui_window_chrome::traffic_light_position(px(Theme::HEADER_HEIGHT))
}

/// The corner radius the content should use for its outermost corners.
pub fn content_radius(window: &Window) -> f32 {
    if resize::is_floating(window) {
        WINDOW_RADIUS
    } else {
        0.0
    }
}

fn policy(cx: &App) -> ButtonPolicy {
    match preferences::get(cx).window_buttons {
        WindowButtons::Auto => ButtonPolicy::Auto,
        WindowButtons::All => ButtonPolicy::All,
        WindowButtons::CloseOnly => ButtonPolicy::CloseOnly,
        WindowButtons::Hidden => ButtonPolicy::Hidden,
    }
}

/// Left edge for our leading titlebar controls, clear of the traffic
/// lights or caption buttons in the leading corner.
pub fn titlebar_leading(window: &Window, cx: &App) -> f32 {
    let layout = caption_layout(window, cx, policy(cx));
    traffic_lights_end(window)
        .map(f32::from)
        .or_else(|| side_inset(Side::Left, &layout.left, cx))
        .map_or(TITLEBAR_LEADING, |end| end + Theme::SPACE_MD)
}

/// Horizontal room a centred titlebar strip leaves on each side, clear of
/// the caption buttons.
pub fn caption_clearance(window: &Window, cx: &App) -> f32 {
    let layout = caption_layout(window, cx, policy(cx));
    side_inset(Side::Right, &layout.right, cx).unwrap_or(0.0) + Theme::SPACE_SM
}

/// Width `buttons` take from the window's edge on `side`.
fn side_inset(side: Side, buttons: &[CaptionButton], cx: &App) -> Option<f32> {
    let count = f32::from(u8::try_from(buttons.len()).ok().filter(|n| *n > 0)?);
    if cfg!(windows) {
        return Some(count * windows_caption_width());
    }
    let geometry = RowGeometry::new(native_row(side, buttons, cx).as_ref());
    let width = geometry
        .width
        .unwrap_or_else(|| DRAWN_SIZE.mul_add(count, DRAWN_SPACING * (count - 1.0)));
    Some(geometry.margin + width)
}

const fn windows_caption_width() -> f32 {
    #[cfg(windows)]
    return WINDOWS_CAPTION_WIDTH;
    #[cfg(not(windows))]
    0.0
}

/// The desktop theme's drawings of `buttons`, in Berth's light or dark.
fn native_row(side: Side, buttons: &[CaptionButton], cx: &App) -> Option<NativeRow> {
    let native = NativeButtons::get()?;
    native.set_dark(Theme::of(cx).appearance == Appearance::Dark);
    native.row(side, buttons)
}

/// Where a row of Linux buttons sits: centred vertically in our header,
/// as far from the window's side as from its top.
struct RowGeometry {
    margin: f32,
    spacing: f32,
    /// The native row's width; ours is computed from the button count.
    width: Option<f32>,
}

impl RowGeometry {
    fn new(row: Option<&NativeRow>) -> Self {
        let height = row.map_or(DRAWN_SIZE, |row| f32::from(row.height()));
        Self {
            margin: ((Theme::HEADER_HEIGHT - height) / 2.0).max(0.0),
            spacing: row.map_or(DRAWN_SPACING, |row| f32::from(row.spacing())),
            width: row.map(|row| f32::from(row.width())),
        }
    }
}

/// The caption buttons, absolutely positioned in the window's corners;
/// placed last in the content so they sit above the titlebar.
pub fn caption_buttons(window: &mut Window, cx: &mut App) -> Option<AnyElement> {
    let layout = caption_layout(window, cx, policy(cx));
    if layout.is_empty() {
        return None;
    }
    Some(platform_buttons(layout, window, cx))
}

/// Windows 11's caption buttons: flush with the top-right corner, the
/// header's full height, glyphs from the system icon font. Each is also
/// the native caption area for its button, so hovering maximize opens
/// Snap Layouts.
#[cfg(windows)]
fn platform_buttons(layout: CaptionLayout, window: &mut Window, cx: &mut App) -> AnyElement {
    let theme = Theme::of(cx);
    let fg = if window.is_window_active() {
        theme.text
    } else {
        theme.text_muted
    };
    let font = gpui_window_chrome::caption_font(cx).unwrap_or_default();
    let buttons = layout.right.into_iter().map(|button| {
        let close = button == CaptionButton::Close;
        let (hover, pressed): (Hsla, Hsla) = if close {
            (gpui::rgb(0x00c4_2b1c).into(), gpui::rgb(0x00b2_2a1b).into())
        } else {
            (theme.element_hover, theme.element_active)
        };
        let glyph = div()
            .font_family(font.clone())
            .text_size(px(10.0))
            .line_height(px(10.0))
            .text_color(fg)
            .when(close, |d| {
                d.group_hover(CAPTION_GROUP, |s| s.text_color(gpui::white()))
            })
            .child(button.glyph());
        let face = div()
            .w(px(WINDOWS_CAPTION_WIDTH))
            .h(px(Theme::HEADER_HEIGHT))
            .flex()
            .items_center()
            .justify_center()
            .hover(move |s| s.bg(hover))
            .active(move |s| s.bg(pressed))
            .window_control_area(button.control_area())
            .child(glyph);
        caption_button(button, face)
    });
    div()
        .absolute()
        .top_0()
        .right_0()
        .flex()
        .children(buttons)
        .into_any_element()
}

/// The desktop's buttons on each side, drawn by its theme where it has
/// one and by us otherwise.
#[cfg(not(windows))]
fn platform_buttons(layout: CaptionLayout, window: &mut Window, cx: &mut App) -> AnyElement {
    let left = button_row(Side::Left, layout.left, window, cx);
    let right = button_row(Side::Right, layout.right, window, cx);
    div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .children(left)
        .children(right)
        .into_any_element()
}

#[cfg(not(windows))]
fn button_row(
    side: Side,
    buttons: Vec<CaptionButton>,
    window: &mut Window,
    cx: &mut App,
) -> Option<Div> {
    if buttons.is_empty() {
        return None;
    }
    let row = native_row(side, &buttons, cx);
    let geometry = RowGeometry::new(row.as_ref());
    let margin = px(geometry.margin);
    let children: Vec<AnyElement> = buttons
        .into_iter()
        .map(|button| {
            let face = row
                .as_ref()
                .and_then(|row| row.button(button, CAPTION_GROUP, window, cx))
                .map_or_else(
                    // A drawing the theme couldn't make shouldn't leave an
                    // invisible button.
                    || drawn_face(button, cx),
                    // The focus ring's border; the drawing stays centred.
                    |face| {
                        face.border_1()
                            .border_color(transparent_black())
                            .rounded_full()
                    },
                );
            caption_button(button, face)
        })
        .collect();
    let row = div()
        .absolute()
        .top(margin)
        .flex()
        .items_center()
        .gap(px(geometry.spacing))
        .children(children);
    Some(match side {
        Side::Left => row.left(margin),
        Side::Right => row.right(margin),
    })
}

/// Make `face` a focusable, clickable caption button.
fn caption_button(button: CaptionButton, face: Div) -> AnyElement {
    let id = match button {
        CaptionButton::Minimize => "caption-min",
        CaptionButton::Maximize | CaptionButton::Restore => "caption-max",
        CaptionButton::Close => "caption-close",
    };
    TabStop::new(
        id,
        face.id(id)
            .group(CAPTION_GROUP)
            .occlude()
            .role(gpui::Role::Button)
            .aria_label(button.label())
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(move |_, window, cx| {
                cx.stop_propagation();
                button.perform(window);
            }),
    )
    .into_any_element()
}

/// Our own button, for when the desktop has no theme to draw it: a glyph
/// on a soft circle.
#[cfg(not(windows))]
fn drawn_face(button: CaptionButton, cx: &App) -> Div {
    let theme = Theme::of(cx);
    let (muted, hover_fg) = (theme.text_muted, theme.text);
    let glyph = match button {
        CaptionButton::Minimize => div()
            .w(px(10.0))
            .h(px(1.5))
            .rounded_full()
            .bg(muted)
            .group_hover(CAPTION_GROUP, move |s| s.bg(hover_fg)),
        CaptionButton::Maximize | CaptionButton::Restore => div()
            .size(px(9.5))
            .rounded(px(2.0))
            .border(px(1.5))
            .border_color(muted)
            .group_hover(CAPTION_GROUP, move |s| s.border_color(hover_fg)),
        CaptionButton::Close => div()
            .relative()
            .size(px(8.0))
            .child(
                div()
                    .absolute()
                    .size_full()
                    .child(cross(muted))
                    .group_hover(CAPTION_GROUP, |s| s.opacity(0.0)),
            )
            .child(
                div()
                    .absolute()
                    .size_full()
                    .opacity(0.0)
                    .child(cross(gpui::white()))
                    .group_hover(CAPTION_GROUP, |s| s.opacity(1.0)),
            ),
    };
    let hover = if button == CaptionButton::Close {
        gpui::rgb(0x00e5_484d).into()
    } else {
        theme.element_hover
    };
    div()
        .size(px(DRAWN_SIZE))
        .border_1()
        .border_color(transparent_black())
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .bg(theme.row_hover)
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
        .child(glyph)
}

/// An X filling its bounds.
#[cfg(not(windows))]
fn cross(color: Hsla) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds, (), window, _| {
            let mut path = PathBuilder::stroke(px(1.5));
            path.move_to(bounds.origin);
            path.line_to(bounds.bottom_right());
            path.move_to(bounds.top_right());
            path.line_to(bounds.bottom_left());
            if let Ok(path) = path.build() {
                window.paint_path(path, color);
            }
        },
    )
    .size_full()
}

/// Wrap the app content in the client-side frame: transparent resize
/// band, rounded corners, hairline border and shadow. Under server-side
/// decorations only the background is added.
pub fn window_frame(content: AnyElement, window: &mut Window, cx: &App) -> impl IntoElement {
    let theme = Theme::of(cx);
    let tiling = resize::client_tiling(window);
    window.set_client_inset(if tiling.is_some() {
        FRAME_INSET
    } else {
        px(0.0)
    });
    let radius = px(content_radius(window));
    div()
        .id("window-frame")
        .relative()
        .size_full()
        .bg(transparent_black())
        .child(ResizeHandles::new(FRAME_INSET))
        .when_some(tiling, |d, tiling| {
            d.when(!tiling.top, |d| d.pt(FRAME_INSET))
                .when(!tiling.bottom, |d| d.pb(FRAME_INSET))
                .when(!tiling.left, |d| d.pl(FRAME_INSET))
                .when(!tiling.right, |d| d.pr(FRAME_INSET))
        })
        .child(
            div()
                .relative()
                .size_full()
                .cursor(gpui::CursorStyle::Arrow)
                .overflow_hidden()
                .rounded(radius)
                .bg(theme.bg)
                .when_some(tiling, |d, tiling| {
                    d.border_color(theme.border_strong)
                        .when(!tiling.top, Styled::border_t_1)
                        .when(!tiling.bottom, Styled::border_b_1)
                        .when(!tiling.left, Styled::border_l_1)
                        .when(!tiling.right, Styled::border_r_1)
                        .when(!tiling.is_tiled(), |d| {
                            d.shadow(vec![
                                gpui::BoxShadow::new(
                                    px(0.0),
                                    px(2.0),
                                    gpui::hsla(0.0, 0.0, 0.0, 0.45),
                                )
                                .blur_radius(px(f32::from(FRAME_INSET) / 1.5)),
                            ])
                        })
                })
                .child(content),
        )
}
