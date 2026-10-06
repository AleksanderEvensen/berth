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

    pub fn of(cx: &App) -> &Theme {
        cx.global::<Theme>()
    }

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
            font_sans: "Geist".into(),
            font_mono: "Geist Mono".into(),
        }
    }

    /// Light is designed, not inverted (see docs/plan/04-design-system.md):
    /// content plane white, chrome grey, accents one step darker.
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
            font_sans: "Geist".into(),
            font_mono: "Geist Mono".into(),
        }
    }

    /// A deterministic, theme-tuned tint for an arbitrary key (image name,
    /// volume name) — used for monogram badges.
    pub fn tint_for(&self, key: &str) -> Hsla {
        let hash = key.bytes().fold(0x811c9dc5u32, |h, b| {
            (h ^ b as u32).wrapping_mul(0x01000193)
        });
        let hue = (hash % 360) as f32;
        match self.appearance {
            Appearance::Dark => oklch(0.74, 0.12, hue),
            Appearance::Light => oklch(0.52, 0.14, hue),
        }
    }
}

/// An exact achromatic tone from an 8-bit channel value (`grey(13)` ≡ `#0d0d0d`).
pub fn grey(value: u8) -> Hsla {
    hsla(0.0, 0.0, value as f32 / 255.0, 1.0)
}

/// A neutral (chroma 0) oklch tone.
pub fn neutral(lightness: f32) -> Hsla {
    let [v, _, _] = oklch_to_srgb(lightness, 0.0, 0.0);
    hsla(0.0, 0.0, v, 1.0)
}

/// oklch (CSS notation: L 0..1, C, H in degrees) → gpui Hsla.
pub fn oklch(l: f32, c: f32, h_deg: f32) -> Hsla {
    let [r, g, b] = oklch_to_srgb(l, c, h_deg);
    let (h, s, l) = rgb_to_hsl(r, g, b);
    hsla(h, s, l, 1.0)
}

/// Björn Ottosson's OKLab → sRGB (the matrices CSS Color 4 uses), gamut-clipped.
fn oklch_to_srgb(l: f32, c: f32, h_deg: f32) -> [f32; 3] {
    let h = h_deg.to_radians();
    let a = c * h.cos();
    let b = c * h.sin();

    let l_ = l + 0.396_337_78 * a + 0.215_803_76 * b;
    let m_ = l - 0.105_561_346 * a - 0.063_854_17 * b;
    let s_ = l - 0.089_484_18 * a - 1.291_485_5 * b;
    let (l3, m3, s3) = (l_ * l_ * l_, m_ * m_ * m_, s_ * s_ * s_);

    let r = 4.076_741_7 * l3 - 3.307_711_6 * m3 + 0.230_969_93 * s3;
    let g = -1.268_438 * l3 + 2.609_757_4 * m3 - 0.341_319_4 * s3;
    let b = -0.004_196_086_3 * l3 - 0.703_418_6 * m3 + 1.707_614_7 * s3;

    [gamma_encode(r), gamma_encode(g), gamma_encode(b)]
}

fn gamma_encode(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    if x <= 0.003_130_8 {
        12.92 * x
    } else {
        1.055 * x.powf(1.0 / 2.4) - 0.055
    }
}

fn rgb_to_hsl(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    if (max - min).abs() < f32::EPSILON {
        return (0.0, 0.0, l);
    }
    let d = max - min;
    let s = if l > 0.5 {
        d / (2.0 - max - min)
    } else {
        d / (max + min)
    };
    let h = if max == r {
        (g - b) / d + if g < b { 6.0 } else { 0.0 }
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (h / 6.0, s, l)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neutral_950_is_0a0a0a() {
        let c = gpui::Rgba::from(neutral(0.145));
        assert!((c.r * 255.0 - 10.0).abs() < 1.0, "{c:?}");
    }

    #[test]
    fn tint_is_stable() {
        let t = Theme::dark();
        assert_eq!(t.tint_for("postgres"), t.tint_for("postgres"));
    }
}
