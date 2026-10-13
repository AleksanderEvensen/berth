//! Caption buttons as the desktop's theme draws them.
//!
//! On Linux and FreeBSD the buttons are drawn by the user's GTK 3 theme,
//! as GTK apps and Chromium show them: Adwaita's and Yaru's icons on
//! circles, the `KWin` buttons Plasma exports to GTK, image themes. GTK is
//! loaded at runtime, so it is never a dependency; without it — and on
//! macOS and Windows, which have no such theme — [`NativeButtons::get`]
//! is `None` and the app draws its own.
//!
//! Each drawing is a plain image. [`NativeRow::button`] stacks a button's
//! states for you; [`NativeRow::image`] and [`NativeRow::pixels`] are there
//! to compose them any other way.

use std::sync::Arc;

use gpui::{Pixels, RenderImage, Size};

#[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
use gpui::{App, Div, SharedString, Window};

#[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
use crate::CaptionButton;

/// The pointer state a button is drawn in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ButtonState {
    Normal,
    Hover,
    Pressed,
}

/// Which end of the titlebar a row of buttons sits at. Themes may style
/// the two differently.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Side {
    Left,
    Right,
}

/// A drawing as a gpui image, with the button's size when drawn.
#[derive(Clone)]
pub struct NativeImage {
    pub image: Arc<RenderImage>,
    /// The button's logical size in this state; draw the image at it.
    pub size: Size<Pixels>,
}

/// A drawing as raw pixels: straight-alpha BGRA rows, `width × height`
/// device pixels.
pub struct NativePixels {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
    /// The button's logical size in this state.
    pub size: Size<Pixels>,
}

/// The desktop theme's caption buttons; see the module docs.
#[derive(Clone, Copy)]
pub struct NativeButtons {
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    gtk: &'static crate::linux::gtk::Gtk,
    #[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
    never: std::convert::Infallible,
}

/// One side's buttons, measured by the theme. Measurements and drawings
/// are of the theme as it was when the row was made, so make rows afresh
/// as the app renders.
#[derive(Clone)]
pub struct NativeRow {
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    inner: linux::Row,
    #[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
    never: std::convert::Infallible,
}

#[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
mod unsupported {
    use super::{
        App, ButtonState, CaptionButton, Div, NativeButtons, NativeImage, NativePixels, NativeRow,
        Pixels, SharedString, Side, Size, Window,
    };

    impl NativeButtons {
        /// `None`: this platform has no theme to draw buttons with.
        #[must_use]
        pub const fn get() -> Option<Self> {
            None
        }

        pub const fn set_dark(&self, _dark: bool) {
            match self.never {}
        }

        #[must_use]
        pub const fn row(&self, _side: Side, _buttons: &[CaptionButton]) -> Option<NativeRow> {
            match self.never {}
        }
    }

    impl NativeRow {
        #[must_use]
        pub const fn size(&self, _button: CaptionButton) -> Option<Size<Pixels>> {
            match self.never {}
        }

        #[must_use]
        pub const fn spacing(&self) -> Pixels {
            match self.never {}
        }

        #[must_use]
        pub const fn width(&self) -> Pixels {
            match self.never {}
        }

        #[must_use]
        pub const fn height(&self) -> Pixels {
            match self.never {}
        }

        pub const fn image(
            &self,
            _button: CaptionButton,
            _state: ButtonState,
            _focused: bool,
            _window: &mut Window,
            _cx: &mut App,
        ) -> Option<NativeImage> {
            match self.never {}
        }

        #[must_use]
        pub const fn pixels(
            &self,
            _button: CaptionButton,
            _state: ButtonState,
            _focused: bool,
            _scale: f32,
        ) -> Option<NativePixels> {
            match self.never {}
        }

        pub const fn button(
            &self,
            _button: CaptionButton,
            _group: impl Into<SharedString>,
            _window: &mut Window,
            _cx: &mut App,
        ) -> Option<Div> {
            match self.never {}
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
mod linux {
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::sync::Arc;

    use gpui::{
        App, Div, ImageSource, Pixels, RenderImage, SharedString, Size, Window, div, img,
        prelude::*, px,
    };

    use super::{ButtonState, NativeButtons, NativeImage, NativePixels, NativeRow, Side};
    use crate::CaptionButton;
    use crate::linux::gtk::{self, Bitmap, ButtonSize, Gtk, Kind, Metrics, State, ThemeKey};

    /// What a [`NativeRow`] holds on Linux.
    #[derive(Clone)]
    pub struct Row {
        gtk: &'static Gtk,
        theme: ThemeKey,
        layout: gtk::Row,
        buttons: Vec<CaptionButton>,
        metrics: Metrics,
    }

    const fn kind(button: CaptionButton) -> Kind {
        match button {
            CaptionButton::Minimize => Kind::Minimize,
            CaptionButton::Maximize => Kind::Maximize,
            CaptionButton::Restore => Kind::Restore,
            CaptionButton::Close => Kind::Close,
        }
    }

    const fn state(state: ButtonState) -> State {
        match state {
            ButtonState::Normal => State::Normal,
            ButtonState::Hover => State::Hover,
            ButtonState::Pressed => State::Pressed,
        }
    }

    #[allow(clippy::missing_const_for_fn, reason = "gpui's `size` isn't const")]
    fn logical(size: ButtonSize) -> Size<Pixels> {
        gpui::size(px(size.width), px(size.height))
    }

    impl NativeButtons {
        /// The desktop theme's buttons, if GTK 3 could be loaded. GTK is
        /// loaded on the first call and belongs to the calling thread — in
        /// a gpui app, call this from the main thread — and is `None` on
        /// every other.
        #[must_use]
        pub fn get() -> Option<Self> {
            Gtk::get().map(|gtk| Self { gtk })
        }

        /// Draw the theme's dark (or light) variant, to match the app;
        /// themes without one are unaffected.
        pub fn set_dark(&self, dark: bool) {
            self.gtk.set_prefer_dark(dark);
        }

        /// `buttons` on `side`, in order from the left, measured by the
        /// theme; `None` if it can't lay them out.
        #[must_use]
        pub fn row(&self, side: Side, buttons: &[CaptionButton]) -> Option<NativeRow> {
            if buttons.is_empty() {
                return None;
            }
            let theme = self.gtk.theme();
            let side = match side {
                Side::Left => gtk::Side::Left,
                Side::Right => gtk::Side::Right,
            };
            let row = gtk::Row::new(side, buttons.iter().map(|button| kind(*button)));
            let metrics = self.gtk.metrics(&theme, &row)?;
            Some(NativeRow {
                inner: Row {
                    gtk: self.gtk,
                    theme,
                    layout: row,
                    buttons: buttons.to_vec(),
                    metrics,
                },
            })
        }
    }

    impl NativeRow {
        /// `button`'s size: the largest it takes in any state, focused or
        /// not. `None` if it isn't in the row.
        #[must_use]
        pub fn size(&self, button: CaptionButton) -> Option<Size<Pixels>> {
            self.inner.metrics.size(kind(button)).map(logical)
        }

        /// The gap the theme leaves between neighbouring buttons.
        #[must_use]
        #[allow(clippy::missing_const_for_fn, reason = "not const off Linux")]
        pub fn spacing(&self) -> Pixels {
            px(self.inner.metrics.spacing)
        }

        /// The row's width: its buttons and the gaps between them.
        #[must_use]
        pub fn width(&self) -> Pixels {
            let buttons = &self.inner.buttons;
            let widths: f32 = buttons
                .iter()
                .filter_map(|button| self.inner.metrics.size(kind(*button)))
                .map(|size| size.width)
                .sum();
            let gaps = buttons.len().saturating_sub(1);
            let gaps = f32::from(u8::try_from(gaps).unwrap_or(0));
            px(self.inner.metrics.spacing.mul_add(gaps, widths))
        }

        /// The tallest button's height.
        #[must_use]
        pub fn height(&self) -> Pixels {
            px(self
                .inner
                .buttons
                .iter()
                .filter_map(|button| self.inner.metrics.size(kind(*button)))
                .map(|size| size.height)
                .fold(0.0, f32::max))
        }

        /// `button` drawn in `state`, for a focused or unfocused window, at
        /// the window's scale; cached until the theme changes.
        pub fn image(
            &self,
            button: CaptionButton,
            state: ButtonState,
            focused: bool,
            window: &mut Window,
            cx: &mut App,
        ) -> Option<NativeImage> {
            let key = ImageKey {
                row: self.inner.layout.clone(),
                button,
                state,
                focused,
                scale: window.scale_factor().to_bits(),
            };
            cached_image(&self.inner, key, window, cx)
        }

        /// `button` drawn in `state`, for a focused or unfocused window, at
        /// `scale` device pixels per logical pixel; drawn afresh each call.
        #[must_use]
        pub fn pixels(
            &self,
            button: CaptionButton,
            state: ButtonState,
            focused: bool,
            scale: f32,
        ) -> Option<NativePixels> {
            self.inner
                .gtk
                .draw(
                    &self.inner.layout,
                    kind(button),
                    self::state(state),
                    focused,
                    scale,
                )
                .map(straight)
        }

        /// `button` at rest, hovered and pressed, stacked so each shows in
        /// its state: hover and press follow the element with group
        /// `group`, which the caller gives the button's clickable element.
        /// Sized as [`Self::size`], each drawing centred at its own size.
        /// `None` if the theme couldn't draw the button at rest.
        pub fn button(
            &self,
            button: CaptionButton,
            group: impl Into<SharedString>,
            window: &mut Window,
            cx: &mut App,
        ) -> Option<Div> {
            let group = group.into();
            let focused = window.is_window_active();
            let mut image = |state: ButtonState| self.image(button, state, focused, window, cx);
            let normal = image(ButtonState::Normal)?;
            let hover = image(ButtonState::Hover).unwrap_or_else(|| normal.clone());
            let pressed = image(ButtonState::Pressed).unwrap_or_else(|| hover.clone());
            let size = self.size(button)?;
            // Centred in a layer filling the button's padding box, so a
            // border the caller adds doesn't move the drawing.
            let layer = |id: &'static str, drawing: NativeImage| {
                div()
                    .id(id)
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        img(ImageSource::Render(drawing.image))
                            .flex_none()
                            .w(drawing.size.width)
                            .h(drawing.size.height),
                    )
            };
            Some(
                div()
                    .relative()
                    .flex_none()
                    .w(size.width)
                    .h(size.height)
                    .child(layer("normal", normal).group_hover(group.clone(), |s| s.opacity(0.0)))
                    .child(
                        layer("hover", hover)
                            .opacity(0.0)
                            .group_hover(group.clone(), |s| s.opacity(1.0))
                            .group_active(group.clone(), |s| s.opacity(0.0)),
                    )
                    .child(
                        layer("pressed", pressed)
                            .opacity(0.0)
                            // No visible change, but gpui only tracks
                            // presses for `group_active` on elements with a
                            // hitbox, which a `group_hover` style gives.
                            .group_hover(group.clone(), |s| s)
                            .group_active(group, |s| s.opacity(1.0)),
                    ),
            )
        }
    }

    /// One drawing, for the theme it was made with.
    #[derive(Debug, Clone, PartialEq, Eq, Hash)]
    struct ImageKey {
        row: gtk::Row,
        button: CaptionButton,
        state: ButtonState,
        focused: bool,
        scale: u32,
    }

    /// Drawings of the current theme; failures are kept too, so each is
    /// attempted once.
    #[derive(Default)]
    struct Images {
        theme: Option<ThemeKey>,
        images: HashMap<ImageKey, Option<NativeImage>>,
    }

    fn cached_image(
        row: &Row,
        key: ImageKey,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<NativeImage> {
        thread_local! {
            static IMAGES: RefCell<Images> = RefCell::default();
        }
        IMAGES.with_borrow_mut(|cache| {
            if cache.theme.as_ref() != Some(&row.theme) {
                for stale in cache.images.drain().filter_map(|(_, image)| image) {
                    cx.drop_image(stale.image, Some(window));
                }
                cache.theme = Some(row.theme.clone());
            }
            if let Some(image) = cache.images.get(&key) {
                return image.clone();
            }
            let image = row
                .gtk
                .draw(
                    &key.row,
                    kind(key.button),
                    state(key.state),
                    key.focused,
                    f32::from_bits(key.scale),
                )
                .map(straight)
                .and_then(|pixels| {
                    let buffer =
                        image::RgbaImage::from_raw(pixels.width, pixels.height, pixels.pixels)?;
                    Some(NativeImage {
                        // The buffer's nominal RGBA order is only a container.
                        image: Arc::new(RenderImage::new(vec![image::Frame::new(buffer)])),
                        size: pixels.size,
                    })
                });
            cache.images.insert(key, image.clone());
            image
        })
    }

    /// GTK draws premultiplied BGRA; gpui, and most consumers, want
    /// straight alpha.
    fn straight(bitmap: Bitmap) -> NativePixels {
        let mut pixels = bitmap.pixels;
        for pixel in pixels.as_chunks_mut::<4>().0 {
            let [b, g, r, alpha] = *pixel;
            let unpremultiply = |channel: u8| {
                if alpha == 0 {
                    return 0;
                }
                let straight = (f32::from(channel) * 255.0 / f32::from(alpha)).round();
                num_traits::cast::<f32, u8>(straight.min(255.0)).unwrap_or(u8::MAX)
            };
            *pixel = [unpremultiply(b), unpremultiply(g), unpremultiply(r), alpha];
        }
        NativePixels {
            width: bitmap.width,
            height: bitmap.height,
            pixels,
            size: logical(bitmap.size),
        }
    }
}
