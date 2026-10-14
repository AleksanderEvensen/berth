//! Title buttons drawn by the desktop's GTK 3 theme, as Chromium (and so
//! Electron) does for its Linux frame.
//!
//! GTK is loaded at runtime, so it is never a hard dependency: without it,
//! or without a display it can open, [`Gtk::get`] is `None` and callers
//! draw their own buttons. When present, the buttons are real `GtkButton`s
//! in a `GtkHeaderBar`, built the way GTK builds a window's own title
//! buttons, in an offscreen window. Whatever the theme does — Adwaita's and
//! Yaru's icons on circles, the `KWin` buttons Plasma exports to GTK, image
//! themes — comes out as GTK apps show it.
//!
//! GTK is single-threaded: everything here runs on the thread that first
//! called [`Gtk::get`], which in a gpui app is the main thread.

use std::cell::{OnceCell, RefCell};
use std::collections::HashMap;
use std::ffi::{CStr, c_char, c_int, c_uint, c_ulong, c_void};
use std::ptr::null_mut;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread::{self, ThreadId};

use image::{ImageBuffer, Rgba, imageops};
use libloading::Library;

use crate::CaptionButton;
use crate::native::{ButtonState, Side};

type Ptr = *mut c_void;
/// A `draw` signal handler; `GCallback` is any function pointer.
type DrawHandler = unsafe extern "C" fn(Ptr, Ptr, Ptr) -> c_int;
/// A `GObject::notify` signal handler.
type NotifyHandler = unsafe extern "C" fn(Ptr, Ptr, Ptr);
/// A `GtkIconTheme::changed` signal handler.
type ChangedHandler = unsafe extern "C" fn(Ptr, Ptr);

/// Bumped whenever a GTK setting or the icon theme changes, so drawings
/// are redone for changes the theme names don't show (fonts, DPI, a
/// reloaded icon theme).
static GENERATION: AtomicU64 = AtomicU64::new(0);

const GTK_ORIENTATION_HORIZONTAL: c_int = 0;
const GTK_ALIGN_CENTER: c_int = 3;
const GTK_ICON_SIZE_MENU: c_int = 1;
/// The logical size of `GTK_ICON_SIZE_MENU`, which title buttons use.
const ICON_SIZE: c_int = 16;
const GTK_ICON_LOOKUP_GENERIC_FALLBACK: c_uint = 1 << 3;
const GTK_ICON_LOOKUP_FORCE_SIZE: c_uint = 1 << 4;
const GTK_STATE_FLAG_ACTIVE: c_uint = 1 << 0;
const GTK_STATE_FLAG_PRELIGHT: c_uint = 1 << 1;
const GTK_STATE_FLAG_BACKDROP: c_uint = 1 << 6;
const CAIRO_FORMAT_ARGB32: c_int = 0;
const CAIRO_STATUS_SUCCESS: c_int = 0;

/// Every drawing is made at this scale and filtered down to the one asked
/// for. GTK caches theme images at the first scale they are drawn at, so a
/// single scale keeps them sharp across monitor changes, and it gives
/// fractional scales a properly filtered image.
const RENDER_SCALE: c_int = 4;

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct Allocation {
    x: c_int,
    y: c_int,
    width: c_int,
    height: c_int,
}

const fn class(button: CaptionButton) -> &'static CStr {
    match button {
        CaptionButton::Minimize => c"minimize",
        CaptionButton::Maximize | CaptionButton::Restore => c"maximize",
        CaptionButton::Close => c"close",
    }
}

const fn icon(button: CaptionButton) -> &'static CStr {
    match button {
        CaptionButton::Minimize => c"window-minimize-symbolic",
        CaptionButton::Maximize => c"window-maximize-symbolic",
        CaptionButton::Restore => c"window-restore-symbolic",
        CaptionButton::Close => c"window-close-symbolic",
    }
}

const fn flags(state: ButtonState) -> c_uint {
    match state {
        ButtonState::Normal => 0,
        ButtonState::Hover => GTK_STATE_FLAG_PRELIGHT,
        ButtonState::Pressed => GTK_STATE_FLAG_PRELIGHT | GTK_STATE_FLAG_ACTIVE,
    }
}

const fn side_class(side: Side) -> &'static CStr {
    match side {
        Side::Left => c"left",
        Side::Right => c"right",
    }
}

/// The theme settings drawings depend on; they are only valid for the
/// theme they were made with.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ThemeKey {
    pub theme: String,
    pub icon_theme: String,
    pub prefer_dark: bool,
    /// Changes with any GTK setting or the icon theme.
    pub generation: u64,
}

/// A drawn button: premultiplied BGRA rows, `width × height` device pixels.
pub struct Bitmap {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
    /// The button's size when it was drawn, in logical pixels.
    pub size: ButtonSize,
}

/// A row of buttons as the desktop lays it out: themes may style the
/// buttons by side, order or neighbours, so GTK draws them in this row.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Row {
    side: Side,
    /// Maximize stands for restore too.
    buttons: Vec<CaptionButton>,
}

impl Row {
    #[must_use]
    pub fn new(side: Side, buttons: impl IntoIterator<Item = CaptionButton>) -> Self {
        let buttons = buttons
            .into_iter()
            .map(|kind| {
                if kind == CaptionButton::Restore {
                    CaptionButton::Maximize
                } else {
                    kind
                }
            })
            .collect();
        Self { side, buttons }
    }
}

/// One button's size, in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ButtonSize {
    pub width: f32,
    pub height: f32,
}

/// GTK's layout of a row's buttons, in logical pixels.
///
/// Themes may size each button differently, and differently per state:
/// each size is the largest the button takes, focused or not, at rest,
/// hovered or pressed.
#[derive(Debug, Clone, PartialEq)]
pub struct Metrics {
    sizes: Vec<(CaptionButton, ButtonSize)>,
    /// Gap GTK leaves between neighbouring title buttons.
    pub spacing: f32,
}

impl Metrics {
    /// The size of `kind`, if it is in the row measured.
    #[must_use]
    pub fn size(&self, kind: CaptionButton) -> Option<ButtonSize> {
        self.sizes
            .iter()
            .find_map(|(measured, size)| (*measured == kind).then_some(*size))
    }
}

/// Measurements for the current theme, by row.
#[derive(Default)]
struct MetricsCache {
    theme: Option<ThemeKey>,
    rows: HashMap<Row, Option<Metrics>>,
}

macro_rules! gtk_api {
    ($($field:ident: fn($($arg:ty),*) $(-> $ret:ty)? = $symbol:literal;)*) => {
        /// The GTK, `GLib` and cairo entry points we use, all resolved
        /// through `libgtk-3`'s dependency tree.
        struct Api {
            $($field: unsafe extern "C" fn($($arg),*) $(-> $ret)?,)*
            g_object_get: unsafe extern "C" fn(Ptr, *const c_char, ...),
            g_object_set: unsafe extern "C" fn(Ptr, *const c_char, ...),
        }

        impl Api {
            /// # Safety
            /// `lib` must be GTK 3, so each symbol has the signature above.
            unsafe fn load(lib: &Library) -> Result<Self, libloading::Error> {
                // SAFETY: per this function's contract.
                unsafe {
                    Ok(Self {
                        $($field: *lib.get(concat!($symbol, "\0").as_bytes())?,)*
                        g_object_get: *lib.get(b"g_object_get\0")?,
                        g_object_set: *lib.get(b"g_object_set\0")?,
                    })
                }
            }
        }
    };
}

gtk_api! {
    disable_setlocale: fn() = "gtk_disable_setlocale";
    init_check: fn(*mut c_int, *mut *mut *mut c_char) -> c_int = "gtk_init_check";
    events_pending: fn() -> c_int = "gtk_events_pending";
    main_iteration_do: fn(c_int) -> c_int = "gtk_main_iteration_do";
    settings_get_default: fn() -> Ptr = "gtk_settings_get_default";
    offscreen_window_new: fn() -> Ptr = "gtk_offscreen_window_new";
    header_bar_new: fn() -> Ptr = "gtk_header_bar_new";
    header_bar_pack_end: fn(Ptr, Ptr) = "gtk_header_bar_pack_end";
    box_new: fn(c_int, c_int) -> Ptr = "gtk_box_new";
    box_reorder_child: fn(Ptr, Ptr, c_int) = "gtk_box_reorder_child";
    widget_show: fn(Ptr) = "gtk_widget_show";
    widget_hide: fn(Ptr) = "gtk_widget_hide";
    image_clear: fn(Ptr) = "gtk_image_clear";
    container_add: fn(Ptr, Ptr) = "gtk_container_add";
    container_check_resize: fn(Ptr) = "gtk_container_check_resize";
    widget_queue_resize: fn(Ptr) = "gtk_widget_queue_resize";
    button_new: fn() -> Ptr = "gtk_button_new";
    image_new_from_icon_name: fn(*const c_char, c_int) -> Ptr = "gtk_image_new_from_icon_name";
    image_set_from_surface: fn(Ptr, Ptr) = "gtk_image_set_from_surface";
    icon_theme_get_default: fn() -> Ptr = "gtk_icon_theme_get_default";
    icon_theme_lookup_icon_for_scale: fn(Ptr, *const c_char, c_int, c_int, c_uint) -> Ptr
        = "gtk_icon_theme_lookup_icon_for_scale";
    icon_info_load_symbolic_for_context: fn(Ptr, Ptr, *mut c_int, *mut Ptr) -> Ptr
        = "gtk_icon_info_load_symbolic_for_context";
    gdk_cairo_surface_create_from_pixbuf: fn(Ptr, c_int, Ptr) -> Ptr
        = "gdk_cairo_surface_create_from_pixbuf";
    widget_get_style_context: fn(Ptr) -> Ptr = "gtk_widget_get_style_context";
    style_context_add_class: fn(Ptr, *const c_char) = "gtk_style_context_add_class";
    style_context_remove_class: fn(Ptr, *const c_char) = "gtk_style_context_remove_class";
    widget_set_valign: fn(Ptr, c_int) = "gtk_widget_set_valign";
    widget_set_can_focus: fn(Ptr, c_int) = "gtk_widget_set_can_focus";
    widget_show_all: fn(Ptr) = "gtk_widget_show_all";
    widget_get_allocation: fn(Ptr, *mut Allocation) = "gtk_widget_get_allocation";
    widget_set_state_flags: fn(Ptr, c_uint, c_int) = "gtk_widget_set_state_flags";
    widget_unset_state_flags: fn(Ptr, c_uint) = "gtk_widget_unset_state_flags";
    widget_draw: fn(Ptr, Ptr) = "gtk_widget_draw";
    g_free: fn(Ptr) = "g_free";
    g_error_free: fn(Ptr) = "g_error_free";
    g_object_unref: fn(Ptr) = "g_object_unref";
    connect_draw: fn(Ptr, *const c_char, DrawHandler, Ptr, Ptr, c_int) -> c_ulong
        = "g_signal_connect_data";
    connect_notify: fn(Ptr, *const c_char, NotifyHandler, Ptr, Ptr, c_int) -> c_ulong
        = "g_signal_connect_data";
    connect_changed: fn(Ptr, *const c_char, ChangedHandler, Ptr, Ptr, c_int) -> c_ulong
        = "g_signal_connect_data";
    cairo_image_surface_create: fn(c_int, c_int, c_int) -> Ptr = "cairo_image_surface_create";
    cairo_surface_set_device_scale: fn(Ptr, f64, f64) = "cairo_surface_set_device_scale";
    cairo_surface_status: fn(Ptr) -> c_int = "cairo_surface_status";
    cairo_surface_flush: fn(Ptr) = "cairo_surface_flush";
    cairo_surface_destroy: fn(Ptr) = "cairo_surface_destroy";
    cairo_image_surface_get_data: fn(Ptr) -> *mut u8 = "cairo_image_surface_get_data";
    cairo_image_surface_get_stride: fn(Ptr) -> c_int = "cairo_image_surface_get_stride";
    cairo_create: fn(Ptr) -> Ptr = "cairo_create";
    cairo_destroy: fn(Ptr) = "cairo_destroy";
}

/// `draw` handler that stops the offscreen window painting itself: only
/// our own draws, at [`RENDER_SCALE`], may populate GTK's image caches.
const unsafe extern "C" fn skip_draw(_widget: Ptr, _cr: Ptr, _data: Ptr) -> c_int {
    1
}

unsafe extern "C" fn settings_changed(_settings: Ptr, _property: Ptr, _data: Ptr) {
    GENERATION.fetch_add(1, Ordering::Relaxed);
}

unsafe extern "C" fn icon_theme_changed(_theme: Ptr, _data: Ptr) {
    GENERATION.fetch_add(1, Ordering::Relaxed);
}

/// One title button widget and its icon.
#[derive(Clone, Copy)]
struct Button {
    widget: Ptr,
    image: Ptr,
}

/// A loaded, initialised GTK with an offscreen header bar of title buttons.
pub struct Gtk {
    api: Api,
    settings: Ptr,
    window: Ptr,
    /// The box holding the buttons, as `GtkHeaderBar`'s own.
    row: Ptr,
    /// The header bar's spacing, which the box uses between buttons.
    spacing: c_int,
    minimize: Button,
    /// Serves restore too; only the icon differs.
    maximize: Button,
    close: Button,
    /// The row the widgets are currently set up as.
    configured: RefCell<Option<Row>>,
    metrics: RefCell<MetricsCache>,
    /// Keeps the symbols above valid; never unloaded (GTK can't be).
    _lib: Library,
}

impl Gtk {
    /// GTK, loaded and initialised on first use. GTK belongs to one thread
    /// for the life of the process — the first to call this, in a gpui app
    /// the main thread — and is `None` on every other.
    #[must_use]
    pub fn get() -> Option<&'static Self> {
        static OWNER: OnceLock<ThreadId> = OnceLock::new();
        let current = thread::current().id();
        if *OWNER.get_or_init(|| current) != current {
            return None;
        }
        thread_local! {
            static GTK: OnceCell<Option<&'static Gtk>> = const { OnceCell::new() };
        }
        GTK.with(|gtk| {
            *gtk.get_or_init(|| {
                Self::load()
                    .inspect_err(|error| log::info!("native title buttons unavailable: {error}"))
                    .ok()
                    // Leaked on purpose: GTK can't be torn down, and its
                    // widgets must outlive every caller on this thread.
                    .map(|gtk| &*Box::leak(Box::new(gtk)))
            })
        })
    }

    fn load() -> Result<Self, String> {
        // SAFETY: loading GTK runs only its library constructors, which
        // have no preconditions.
        let lib = unsafe { Library::new("libgtk-3.so.0") }.map_err(|e| e.to_string())?;
        // SAFETY: `lib` is GTK 3.
        let api = unsafe { Api::load(&lib) }.map_err(|e| e.to_string())?;
        // SAFETY: plain GTK calls on the thread that will own GTK. The
        // locale is the host app's business, so GTK must not set it.
        unsafe {
            (api.disable_setlocale)();
            if (api.init_check)(null_mut(), null_mut()) == 0 {
                return Err("GTK could not open a display".into());
            }
        }
        // SAFETY: GTK is initialised; the widgets built here belong to
        // `window`, which is never destroyed.
        unsafe {
            let settings = (api.settings_get_default)();
            if settings.is_null() {
                return Err("GTK has no settings".into());
            }
            // Draw end states, not the first frame of a theme transition.
            (api.g_object_set)(
                settings,
                c"gtk-enable-animations".as_ptr(),
                c_int::from(false),
                null_mut::<c_void>(),
            );
            (api.connect_notify)(
                settings,
                c"notify".as_ptr(),
                settings_changed,
                null_mut(),
                null_mut(),
                0,
            );
            (api.connect_changed)(
                (api.icon_theme_get_default)(),
                c"changed".as_ptr(),
                icon_theme_changed,
                null_mut(),
                null_mut(),
                0,
            );
            let window = (api.offscreen_window_new)();
            (api.connect_draw)(
                window,
                c"draw".as_ptr(),
                skip_draw,
                null_mut(),
                null_mut(),
                0,
            );
            // The compact titlebar GTK gives windows without a header bar
            // of their own: its buttons are titlebar-sized.
            let bar = (api.header_bar_new)();
            let bar_style = (api.widget_get_style_context)(bar);
            (api.style_context_add_class)(bar_style, c"titlebar".as_ptr());
            (api.style_context_add_class)(bar_style, c"default-decoration".as_ptr());
            // As GtkHeaderBar's own title-button box, spaced like it.
            let mut spacing: c_int = 0;
            (api.g_object_get)(
                bar,
                c"spacing".as_ptr(),
                &raw mut spacing,
                null_mut::<c_void>(),
            );
            let row = (api.box_new)(GTK_ORIENTATION_HORIZONTAL, spacing);
            (api.widget_set_valign)(row, GTK_ALIGN_CENTER);
            (api.style_context_add_class)((api.widget_get_style_context)(row), c"right".as_ptr());
            let [minimize, maximize, close] = [
                CaptionButton::Minimize,
                CaptionButton::Maximize,
                CaptionButton::Close,
            ]
            .map(|kind| {
                let widget = (api.button_new)();
                (api.widget_set_valign)(widget, GTK_ALIGN_CENTER);
                (api.widget_set_can_focus)(widget, 0);
                let style = (api.widget_get_style_context)(widget);
                (api.style_context_add_class)(style, c"titlebutton".as_ptr());
                (api.style_context_add_class)(style, class(kind).as_ptr());
                let image = (api.image_new_from_icon_name)(icon(kind).as_ptr(), GTK_ICON_SIZE_MENU);
                (api.container_add)(widget, image);
                (api.container_add)(row, widget);
                Button { widget, image }
            });
            (api.header_bar_pack_end)(bar, row);
            (api.container_add)(window, bar);
            (api.widget_show_all)(window);
            let gtk = Self {
                api,
                settings,
                window,
                row,
                spacing,
                minimize,
                maximize,
                close,
                configured: RefCell::new(None),
                metrics: RefCell::default(),
                _lib: lib,
            };
            gtk.pump();
            log::info!(
                "drawing title buttons with the GTK theme {:?}",
                gtk.string_setting(c"gtk-theme-name")
            );
            Ok(gtk)
        }
    }

    /// Run GTK's pending work (settings and theme changes) and lay the
    /// buttons out.
    pub fn pump(&self) {
        // SAFETY: GTK is initialised on this thread; nothing here blocks.
        unsafe {
            let mut budget = 64_u8;
            while budget > 0 && (self.api.events_pending)() != 0 {
                (self.api.main_iteration_do)(0);
                budget = budget.saturating_sub(1);
            }
            // An offscreen window only lays out on its frame clock, which
            // doesn't tick here. Its styles are recomputed on demand, but
            // the resize a style change queues comes from that clock too,
            // so each button is told to measure itself again, then laid out.
            for button in [self.minimize, self.maximize, self.close] {
                (self.api.widget_queue_resize)(button.widget);
            }
            (self.api.container_check_resize)(self.window);
        }
    }

    /// The current theme, after picking up pending changes.
    #[must_use]
    pub fn theme(&self) -> ThemeKey {
        self.pump();
        ThemeKey {
            theme: self.string_setting(c"gtk-theme-name"),
            icon_theme: self.string_setting(c"gtk-icon-theme-name"),
            prefer_dark: self.bool_setting(c"gtk-application-prefer-dark-theme"),
            generation: GENERATION.load(Ordering::Relaxed),
        }
    }

    /// Ask the theme for its dark (or light) variant, to match the app.
    pub fn set_prefer_dark(&self, dark: bool) {
        if self.bool_setting(c"gtk-application-prefer-dark-theme") == dark {
            return;
        }
        // SAFETY: a boolean property of the live default settings.
        unsafe {
            (self.api.g_object_set)(
                self.settings,
                c"gtk-application-prefer-dark-theme".as_ptr(),
                c_int::from(dark),
                null_mut::<c_void>(),
            );
        }
        self.pump();
    }

    fn string_setting(&self, name: &CStr) -> String {
        let mut value: *mut c_char = null_mut();
        // SAFETY: `name` is a string property of GtkSettings; GLib hands
        // back a copy, which we free.
        unsafe {
            (self.api.g_object_get)(
                self.settings,
                name.as_ptr(),
                &raw mut value,
                null_mut::<c_void>(),
            );
            if value.is_null() {
                return String::new();
            }
            let string = CStr::from_ptr(value).to_string_lossy().into_owned();
            (self.api.g_free)(value.cast());
            string
        }
    }

    fn bool_setting(&self, name: &CStr) -> bool {
        let mut value: c_int = 0;
        // SAFETY: `name` is a boolean (gboolean) property of GtkSettings.
        unsafe {
            (self.api.g_object_get)(
                self.settings,
                name.as_ptr(),
                &raw mut value,
                null_mut::<c_void>(),
            );
        }
        value != 0
    }

    const fn button(&self, kind: CaptionButton) -> Button {
        match kind {
            CaptionButton::Minimize => self.minimize,
            CaptionButton::Maximize | CaptionButton::Restore => self.maximize,
            CaptionButton::Close => self.close,
        }
    }

    fn allocation(&self, widget: Ptr) -> Allocation {
        let mut allocation = Allocation::default();
        // SAFETY: `widget` is one of our live buttons.
        unsafe { (self.api.widget_get_allocation)(widget, &raw mut allocation) };
        allocation
    }

    /// Set the widgets up as `row`: its side's box class, and only its
    /// buttons, in its order.
    fn configure(&self, row: &Row) {
        if self.configured.borrow().as_ref() == Some(row) {
            return;
        }
        let style = {
            // SAFETY: our live box.
            unsafe { (self.api.widget_get_style_context)(self.row) }
        };
        // SAFETY: our live box and buttons; hidden buttons stay in the box.
        unsafe {
            for side in [Side::Left, Side::Right] {
                (self.api.style_context_remove_class)(style, side_class(side).as_ptr());
            }
            (self.api.style_context_add_class)(style, side_class(row.side).as_ptr());
            for kind in [
                CaptionButton::Minimize,
                CaptionButton::Maximize,
                CaptionButton::Close,
            ] {
                (self.api.widget_hide)(self.button(kind).widget);
            }
            for (position, kind) in row.buttons.iter().enumerate() {
                let widget = self.button(*kind).widget;
                (self.api.box_reorder_child)(
                    self.row,
                    widget,
                    c_int::try_from(position).unwrap_or(c_int::MAX),
                );
                (self.api.widget_show)(widget);
            }
        }
        *self.configured.borrow_mut() = Some(row.clone());
        self.pump();
    }

    /// The sizes and spacing of `row`'s buttons as GTK lays them out in
    /// `theme` (from [`Self::theme`]); measured once per theme and row.
    #[must_use]
    pub fn metrics(&self, theme: &ThemeKey, row: &Row) -> Option<Metrics> {
        {
            let mut cache = self.metrics.borrow_mut();
            if cache.theme.as_ref() != Some(theme) {
                cache.theme = Some(theme.clone());
                cache.rows.clear();
            }
            if let Some(metrics) = cache.rows.get(row) {
                return metrics.clone();
            }
        }
        // Measuring pumps GTK, which can emit signals; nothing is borrowed.
        let metrics = self.measure(row);
        self.metrics
            .borrow_mut()
            .rows
            .insert(row.clone(), metrics.clone());
        metrics
    }

    fn measure(&self, row: &Row) -> Option<Metrics> {
        self.configure(row);
        let mut kinds = row.buttons.clone();
        if kinds.contains(&CaptionButton::Maximize) {
            kinds.push(CaptionButton::Restore);
        }
        let mut sizes = Vec::with_capacity(kinds.len());
        for kind in kinds {
            let mut largest: Option<ButtonSize> = None;
            for active in [true, false] {
                for state in [
                    ButtonState::Normal,
                    ButtonState::Hover,
                    ButtonState::Pressed,
                ] {
                    let size = self.in_state(kind, state, active, || {
                        logical_size(self.allocation(self.button(kind).widget))
                    })?;
                    largest = Some(largest.map_or(size, |largest| ButtonSize {
                        width: largest.width.max(size.width),
                        height: largest.height.max(size.height),
                    }));
                }
            }
            sizes.push((kind, largest?));
        }
        let spacing = match row.buttons.as_slice() {
            [first, second, ..] => {
                let (first, second) = (
                    self.allocation(self.button(*first).widget),
                    self.allocation(self.button(*second).widget),
                );
                second
                    .x
                    .checked_sub(first.x)
                    .and_then(|step| step.checked_sub(first.width))
                    .unwrap_or(0)
            }
            _ => self.spacing,
        };
        Some(Metrics {
            sizes,
            spacing: f32::from(u16::try_from(spacing.max(0)).ok()?),
        })
    }

    /// Run `f` with `kind`'s button in `state`, in a focused (`active`) or
    /// unfocused window, laid out; restore with the window maximized.
    fn in_state<T>(
        &self,
        kind: CaptionButton,
        state: ButtonState,
        active: bool,
        f: impl FnOnce() -> T,
    ) -> T {
        let button = self.button(kind).widget;
        // SAFETY: our live window and buttons; the window's class and the
        // button's pointer state are reset before returning, so states
        // don't leak into each other.
        unsafe {
            let window_style = (self.api.widget_get_style_context)(self.window);
            if kind == CaptionButton::Restore {
                (self.api.style_context_add_class)(window_style, c"maximized".as_ptr());
            }
            // Backdrop is inherited from the window, so the button's own
            // flags are only ever added to and removed, never replaced.
            if active {
                (self.api.widget_unset_state_flags)(self.window, GTK_STATE_FLAG_BACKDROP);
            } else {
                (self.api.widget_set_state_flags)(self.window, GTK_STATE_FLAG_BACKDROP, 0);
            }
            (self.api.widget_set_state_flags)(button, flags(state), 0);
            self.pump();
            let value = f();
            (self.api.widget_unset_state_flags)(
                button,
                GTK_STATE_FLAG_PRELIGHT | GTK_STATE_FLAG_ACTIVE,
            );
            if kind == CaptionButton::Restore {
                (self.api.style_context_remove_class)(window_style, c"maximized".as_ptr());
            }
            self.pump();
            value
        }
    }

    /// Draw `kind`, as it sits in `row`, in `state`, for a focused
    /// (`active`) or unfocused window, at `scale` device pixels per
    /// logical pixel.
    #[must_use]
    pub fn draw(
        &self,
        row: &Row,
        kind: CaptionButton,
        state: ButtonState,
        active: bool,
        scale: f32,
    ) -> Option<Bitmap> {
        self.configure(row);
        let button = self.button(kind);
        self.in_state(kind, state, active, || {
            // SAFETY: `button.image` is our live image.
            if !unsafe { self.load_icon(kind, button.image) } {
                return None;
            }
            self.pump();
            self.render(button.widget)
        })
        .and_then(|bitmap| bitmap.downscale(scale))
    }

    /// Give `image` the theme's symbolic icon for `kind` at
    /// [`RENDER_SCALE`], coloured for the image's current state. `GtkImage`
    /// would load it for the offscreen window's scale, which is 1. `false`
    /// if the theme has no such icon or it fails to load.
    ///
    /// # Safety
    /// `image` must be one of our live images.
    unsafe fn load_icon(&self, kind: CaptionButton, image: Ptr) -> bool {
        // SAFETY: `image` is live. A failed load leaves no icon rather than
        // the last one drawn, which could be another button's.
        unsafe { (self.api.image_clear)(image) };
        // SAFETY: GTK is initialised; every object created here is
        // released here, and the image keeps its own reference to the
        // surface it is given.
        unsafe {
            let theme = (self.api.icon_theme_get_default)();
            let info = (self.api.icon_theme_lookup_icon_for_scale)(
                theme,
                icon(kind).as_ptr(),
                ICON_SIZE,
                RENDER_SCALE,
                GTK_ICON_LOOKUP_GENERIC_FALLBACK | GTK_ICON_LOOKUP_FORCE_SIZE,
            );
            if info.is_null() {
                return false;
            }
            let mut error: Ptr = null_mut();
            let pixbuf = (self.api.icon_info_load_symbolic_for_context)(
                info,
                (self.api.widget_get_style_context)(image),
                null_mut(),
                &raw mut error,
            );
            (self.api.g_object_unref)(info);
            if !error.is_null() {
                (self.api.g_error_free)(error);
            }
            if pixbuf.is_null() {
                return false;
            }
            let surface =
                (self.api.gdk_cairo_surface_create_from_pixbuf)(pixbuf, RENDER_SCALE, null_mut());
            (self.api.g_object_unref)(pixbuf);
            (self.api.image_set_from_surface)(image, surface);
            (self.api.cairo_surface_destroy)(surface);
        }
        true
    }

    /// `widget` drawn at [`RENDER_SCALE`].
    fn render(&self, widget: Ptr) -> Option<Bitmap> {
        let allocation = self.allocation(widget);
        let width = allocation.width.checked_mul(RENDER_SCALE)?;
        let height = allocation.height.checked_mul(RENDER_SCALE)?;
        if width <= 0 || height <= 0 {
            return None;
        }
        let scale = f64::from(RENDER_SCALE);
        // SAFETY: a fresh image surface we own and destroy here; GTK draws
        // the widget at the context's origin. Its pixel buffer is `stride ×
        // height` bytes while the surface lives.
        unsafe {
            let surface = (self.api.cairo_image_surface_create)(CAIRO_FORMAT_ARGB32, width, height);
            if (self.api.cairo_surface_status)(surface) != CAIRO_STATUS_SUCCESS {
                (self.api.cairo_surface_destroy)(surface);
                return None;
            }
            (self.api.cairo_surface_set_device_scale)(surface, scale, scale);
            let cr = (self.api.cairo_create)(surface);
            (self.api.widget_draw)(widget, cr);
            (self.api.cairo_destroy)(cr);
            (self.api.cairo_surface_flush)(surface);
            let bitmap = Bitmap::read(
                (self.api.cairo_image_surface_get_data)(surface),
                (self.api.cairo_image_surface_get_stride)(surface),
                width,
                height,
                logical_size(allocation)?,
            );
            (self.api.cairo_surface_destroy)(surface);
            bitmap
        }
    }
}

/// An allocation's size in logical pixels, if it has been laid out.
fn logical_size(allocation: Allocation) -> Option<ButtonSize> {
    let logical = |px: c_int| u16::try_from(px).ok().map(f32::from).filter(|px| *px > 1.0);
    Some(ButtonSize {
        width: logical(allocation.width)?,
        height: logical(allocation.height)?,
    })
}

impl Bitmap {
    /// Copy cairo's ARGB32 rows — premultiplied and native-endian, so BGRA
    /// in memory on little-endian — into a tight buffer.
    ///
    /// # Safety
    /// `data` must point at `stride × height` readable bytes.
    unsafe fn read(
        data: *const u8,
        stride: c_int,
        width: c_int,
        height: c_int,
        size: ButtonSize,
    ) -> Option<Self> {
        if data.is_null() || cfg!(target_endian = "big") {
            return None;
        }
        let stride = usize::try_from(stride).ok()?;
        let width = u32::try_from(width).ok()?;
        let height = u32::try_from(height).ok()?;
        let row = usize::try_from(width).ok()?.checked_mul(4)?;
        let rows = usize::try_from(height).ok()?;
        // SAFETY: per this function's contract.
        let source = unsafe { std::slice::from_raw_parts(data, stride.checked_mul(rows)?) };
        let mut pixels = Vec::with_capacity(row.checked_mul(rows)?);
        for line in source.chunks_exact(stride) {
            pixels.extend_from_slice(line.get(..row)?);
        }
        Some(Self {
            width,
            height,
            pixels,
            size,
        })
    }

    /// Filter a [`RENDER_SCALE`] drawing down to `scale`. Filtering
    /// premultiplied pixels is what keeps edges from darkening, and it
    /// treats each channel alike, so BGRA order doesn't matter.
    fn downscale(self, scale: f32) -> Option<Self> {
        let factor = scale / f32::from(u16::try_from(RENDER_SCALE).ok()?);
        let target = |px: u32| {
            num_traits::cast::<f32, u32>((f32::from(u16::try_from(px).ok()?) * factor).round())
                .filter(|px| *px > 0)
        };
        let (width, height) = (target(self.width)?, target(self.height)?);
        if (width, height) == (self.width, self.height) {
            return Some(self);
        }
        let source: ImageBuffer<Rgba<u8>, _> =
            ImageBuffer::from_raw(self.width, self.height, self.pixels)?;
        let scaled = imageops::resize(&source, width, height, imageops::FilterType::Triangle);
        Some(Self {
            width,
            height,
            pixels: scaled.into_raw(),
            size: self.size,
        })
    }
}
