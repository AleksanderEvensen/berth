//! Theme tokens.
//!
//! Token names and roles follow zeron's theme (reference/zeron-source-code,
//! `crates/ui/src/theme.rs`, MIT) so the two can be compared side by side:
//! near-black achromatic planes with tiny elevation steps, white ink at low
//! alpha for interaction, hairline borders, one accent hue.
//!
//! **Numbers drive layout, colors are paint**: layout constants live here as
//! plain numbers and never depend on the appearance.

use gpui::{App, Global, Hsla, SharedString, hsla};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Appearance {
    Dark,
    Light,
}

#[derive(Debug, Clone)]
pub struct Theme {
    pub appearance: Appearance,

    // ---- neutral planes ----
    /// Main content / detail plane — the deepest plane in dark mode.
    pub bg: Hsla,
    /// Shell: sidebar, titlebar, list column.
    pub surface: Hsla,
    /// Inline card resting on `bg` (info groups, tables).
    pub surface_card: Hsla,
    /// Modal dialog.
    pub surface_dialog: Hsla,
    /// Popovers, menus, command palette.
    pub surface_overlay: Hsla,
    /// Opaque raised pill (segmented control thumb, chips).
    pub surface_raised: Hsla,

    // ---- interaction washes ----
    pub element_hover: Hsla,
    pub element_active: Hsla,
    /// Softer washes for dense list rows.
    pub row_hover: Hsla,
    pub row_selected: Hsla,

    // ---- lines ----
    pub border: Hsla,
    pub border_strong: Hsla,

    // ---- text ----
    pub text: Hsla,
    pub text_muted: Hsla,
    pub text_faint: Hsla,

    // ---- solid (primary button plate) ----
    pub solid: Hsla,
    pub on_solid: Hsla,

    // ---- accents & status ----
    pub accent: Hsla,
    pub accent_wash: Hsla,
    pub success: Hsla,
    pub warning: Hsla,
    pub danger: Hsla,
    pub danger_wash: Hsla,
    /// Solid destructive plate carrying white text.
    pub danger_strong: Hsla,
    /// Dims the window behind a modal.
    pub scrim: Hsla,

    /// ANSI 0–15 for logs and the terminal: black, red, green, yellow, blue,
    /// magenta, cyan, white, then the bright variants.
    pub ansi: [Hsla; 16],

    pub font_sans: SharedString,
    pub font_mono: SharedString,
}

impl Global for Theme {}

impl Theme {
    // ---- layout ----
    pub const TITLEBAR_HEIGHT: f32 = 38.0;
    /// List-column header (title + subtitle) — also the titlebar drag strip.
    pub const HEADER_HEIGHT: f32 = 52.0;
    pub const SIDEBAR_WIDTH: f32 = 232.0;
    pub const LIST_WIDTH: f32 = 340.0;
    pub const SIDEBAR_ROW_HEIGHT: f32 = 30.0;
    pub const RESOURCE_ROW_HEIGHT: f32 = 44.0;

    pub const CONTROL_RADIUS: f32 = 6.0;
    pub const ROW_RADIUS: f32 = 8.0;
    pub const PANEL_RADIUS: f32 = 10.0;

    pub const SPACE_XS: f32 = 4.0;
    pub const SPACE_SM: f32 = 8.0;
    pub const SPACE_MD: f32 = 12.0;
    pub const SPACE_LG: f32 = 16.0;

    // ---- type scale (px) ----
    pub const TEXT_XS: f32 = 11.0;
    pub const TEXT_SM: f32 = 12.0;
    pub const TEXT_MD: f32 = 13.0;
    pub const TEXT_LG: f32 = 14.0;

    pub fn of(cx: &App) -> &Self {
        cx.global::<Self>()
    }

    #[must_use]
    pub fn dark() -> Self {
        let accent = oklch(0.673, 0.182, 276.935);
        Self {
            appearance: Appearance::Dark,
            bg: grey(6),
            surface: grey(13),
            surface_card: grey(0x0e),
            surface_dialog: grey(0x10),
            surface_overlay: grey(0x16),
            surface_raised: neutral(0.235),
            element_hover: hsla(0.0, 0.0, 0.92, 0.11),
            element_active: hsla(0.0, 0.0, 0.92, 0.16),
            row_hover: hsla(0.0, 0.0, 1.0, 0.05),
            row_selected: hsla(0.0, 0.0, 1.0, 0.09),
            border: hsla(0.0, 0.0, 1.0, 0.08),
            border_strong: hsla(0.0, 0.0, 1.0, 0.14),
            text: neutral(0.922),
            text_muted: neutral(0.708),
            text_faint: neutral(0.556),
            solid: neutral(0.922),
            on_solid: grey(0x0e),
            accent,
            accent_wash: accent.opacity(0.14),
            success: oklch(0.765, 0.177, 163.223),
            warning: oklch(0.828, 0.189, 84.429),
            danger: oklch(0.704, 0.191, 22.216),
            danger_wash: oklch(0.704, 0.191, 22.216).opacity(0.14),
            danger_strong: oklch(0.58, 0.16, 25.0),
            scrim: hsla(0.0, 0.0, 0.0, 0.60),
            ansi: [
                neutral(0.37),
                oklch(0.704, 0.191, 22.216),
                oklch(0.792, 0.209, 151.711),
                oklch(0.852, 0.199, 91.936),
                oklch(0.707, 0.165, 254.624),
                oklch(0.714, 0.203, 305.504),
                oklch(0.789, 0.154, 211.53),
                neutral(0.87),
                neutral(0.556),
                oklch(0.808, 0.114, 19.571),
                oklch(0.871, 0.15, 154.449),
                oklch(0.905, 0.182, 98.111),
                oklch(0.809, 0.105, 251.813),
                oklch(0.827, 0.119, 306.383),
                oklch(0.865, 0.127, 207.078),
                neutral(0.985),
            ],
            font_sans: "Geist".into(),
            font_mono: "Geist Mono".into(),
        }
    }

    /// Light is designed, not inverted (see docs/plan/04-design-system.md):
    /// content plane white, chrome grey, accents one step darker.
    #[must_use]
    pub fn light() -> Self {
        let accent = oklch(0.511, 0.262, 276.966);
        Self {
            appearance: Appearance::Light,
            bg: grey(0xff),
            surface: neutral(0.97),
            surface_card: neutral(0.985),
            surface_dialog: grey(0xff),
            surface_overlay: grey(0xff),
            surface_raised: grey(0xff),
            element_hover: hsla(0.0, 0.0, 0.0, 0.06),
            element_active: hsla(0.0, 0.0, 0.0, 0.09),
            row_hover: hsla(0.0, 0.0, 0.0, 0.04),
            row_selected: hsla(0.0, 0.0, 0.0, 0.07),
            border: hsla(0.0, 0.0, 0.0, 0.10),
            border_strong: hsla(0.0, 0.0, 0.0, 0.18),
            text: neutral(0.205),
            text_muted: neutral(0.439),
            text_faint: neutral(0.556),
            solid: neutral(0.205),
            on_solid: grey(0xfa),
            accent,
            accent_wash: accent.opacity(0.10),
            success: oklch(0.596, 0.145, 163.225),
            warning: oklch(0.666, 0.179, 58.318),
            danger: oklch(0.577, 0.245, 27.325),
            danger_wash: oklch(0.577, 0.245, 27.325).opacity(0.10),
            danger_strong: oklch(0.577, 0.245, 27.325),
            scrim: hsla(0.0, 0.0, 0.0, 0.28),
            ansi: [
                neutral(0.205),
                oklch(0.577, 0.245, 27.325),
                oklch(0.527, 0.154, 150.069),
                oklch(0.554, 0.135, 66.442),
                oklch(0.546, 0.245, 262.881),
                oklch(0.558, 0.288, 302.321),
                oklch(0.52, 0.105, 223.128),
                neutral(0.556),
                neutral(0.439),
                oklch(0.505, 0.213, 27.518),
                oklch(0.448, 0.119, 151.328),
                oklch(0.476, 0.114, 61.907),
                oklch(0.488, 0.243, 264.376),
                oklch(0.496, 0.265, 301.924),
                oklch(0.45, 0.085, 224.283),
                neutral(0.708),
            ],
            font_sans: "Geist".into(),
            font_mono: "Geist Mono".into(),
        }
    }

    /// A deterministic, theme-tuned tint for an arbitrary key (image name,
    /// volume name) — used for monogram badges.
    #[must_use]
    pub fn tint_for(&self, key: &str) -> Hsla {
        let hash = key.bytes().fold(0x811c_9dc5_u32, |h, b| {
            (h ^ u32::from(b)).wrapping_mul(0x0100_0193)
        });
        // The remainder fits in u16, which converts losslessly to f32.
        let hue = f32::from(u16::try_from(hash % 360).unwrap_or_default());
        match self.appearance {
            Appearance::Dark => oklch(0.74, 0.12, hue),
            Appearance::Light => oklch(0.52, 0.14, hue),
        }
    }
}

/// An exact achromatic tone from an 8-bit channel value (`grey(13)` ≡ `#0d0d0d`).
#[must_use]
pub fn grey(value: u8) -> Hsla {
    hsla(0.0, 0.0, f32::from(value) / 255.0, 1.0)
}

/// A neutral (chroma 0) oklch tone.
#[must_use]
pub fn neutral(lightness: f32) -> Hsla {
    let [v, _, _] = oklch_to_srgb(lightness, 0.0, 0.0);
    hsla(0.0, 0.0, v, 1.0)
}

/// oklch (CSS notation: L 0..1, C, H in degrees) → gpui Hsla.
#[must_use]
pub fn oklch(lightness: f32, chroma: f32, hue_degrees: f32) -> Hsla {
    let [red, green, blue] = oklch_to_srgb(lightness, chroma, hue_degrees);
    let (hue, saturation, lightness) = rgb_to_hsl(red, green, blue);
    hsla(hue, saturation, lightness, 1.0)
}

/// Björn Ottosson's `OKLab` → sRGB (the matrices CSS Color 4 uses), gamut-clipped.
fn oklch_to_srgb(lightness: f32, chroma: f32, hue_degrees: f32) -> [f32; 3] {
    let hue = hue_degrees.to_radians();
    let a = chroma * hue.cos();
    let lab_b = chroma * hue.sin();

    let l_ = 0.215_803_76f32.mul_add(lab_b, 0.396_337_78f32.mul_add(a, lightness));
    let m_ = 0.063_854_17f32.mul_add(-lab_b, 0.105_561_346f32.mul_add(-a, lightness));
    let s_ = 1.291_485_5f32.mul_add(-lab_b, 0.089_484_18f32.mul_add(-a, lightness));
    let (l3, m3, s3) = (l_ * l_ * l_, m_ * m_ * m_, s_ * s_ * s_);

    let red = 0.230_969_93f32.mul_add(s3, 3.307_711_6f32.mul_add(-m3, 4.076_741_7 * l3));
    let green = 0.341_319_4f32.mul_add(-s3, 2.609_757_4f32.mul_add(m3, -1.268_438 * l3));
    let blue = 1.707_614_7f32.mul_add(s3, 0.703_418_6f32.mul_add(-m3, -0.004_196_086_3 * l3));

    [gamma_encode(red), gamma_encode(green), gamma_encode(blue)]
}

fn gamma_encode(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    if x <= 0.003_130_8 {
        12.92 * x
    } else {
        1.055f32.mul_add(x.powf(1.0 / 2.4), -0.055)
    }
}

// The maximum is one of these exact input channels; equality selects its hue formula.
#[expect(
    clippy::float_cmp,
    reason = "identify the channel that supplied the maximum"
)]
pub(crate) fn rgb_to_hsl(red: f32, green: f32, blue: f32) -> (f32, f32, f32) {
    let max = red.max(green).max(blue);
    let min = red.min(green).min(blue);
    let lightness = f32::midpoint(max, min);
    if (max - min).abs() < f32::EPSILON {
        return (0.0, 0.0, lightness);
    }
    let range = max - min;
    let saturation = if lightness > 0.5 {
        range / (2.0 - max - min)
    } else {
        range / (max + min)
    };
    let hue = if max == red {
        (green - blue) / range + if green < blue { 6.0 } else { 0.0 }
    } else if max == green {
        (blue - red) / range + 2.0
    } else {
        (red - green) / range + 4.0
    };
    (hue / 6.0, saturation, lightness)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neutral_950_is_0a0a0a() {
        let c = gpui::Rgba::from(neutral(0.145));
        assert!(f32::mul_add(c.r, 255.0, -10.0).abs() < 1.0, "{c:?}");
    }

    #[test]
    fn tint_is_stable() {
        let t = Theme::dark();
        assert_eq!(t.tint_for("postgres"), t.tint_for("postgres"));
    }

    #[test]
    fn oklch_primary_colors_match_srgb() {
        for (lightness, chroma, hue, expected) in [
            (0.627_955, 0.257_683, 29.233_885, [1.0, 0.0, 0.0]),
            (0.866_44, 0.294_827, 142.495_3, [0.0, 1.0, 0.0]),
            (0.452_014, 0.313_214, 264.052, [0.0, 0.0, 1.0]),
        ] {
            let color = gpui::Rgba::from(oklch(lightness, chroma, hue));
            for (actual, expected) in [color.r, color.g, color.b].into_iter().zip(expected) {
                assert!((actual - expected).abs() < 0.002, "{color:?}");
            }
        }
    }

    #[test]
    fn rgb_hues_cover_primary_colors_and_tied_maxima() {
        for (rgb, expected_hue) in [
            ([1.0, 0.0, 0.0], 0.0),
            ([1.0, 1.0, 0.0], 1.0 / 6.0),
            ([0.0, 1.0, 0.0], 2.0 / 6.0),
            ([0.0, 1.0, 1.0], 3.0 / 6.0),
            ([0.0, 0.0, 1.0], 4.0 / 6.0),
            ([1.0, 0.0, 1.0], 5.0 / 6.0),
        ] {
            let [red, green, blue] = rgb;
            let (hue, saturation, lightness) = rgb_to_hsl(red, green, blue);
            assert!((hue - expected_hue).abs() < f32::EPSILON);
            assert!((saturation - 1.0).abs() < f32::EPSILON);
            assert!((lightness - 0.5).abs() < f32::EPSILON);
        }
    }
}
