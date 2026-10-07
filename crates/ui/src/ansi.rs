//! The xterm colour model shared by the log view and the terminal: ANSI 0–15
//! come from the theme, 16–231 are the 6×6×6 cube, 232–255 the grey ramp.

use gpui::Hsla;

use crate::Theme;
use crate::theme::Appearance;

/// A colour as programs request it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AnsiColor {
    Indexed(u8),
    Rgb(u8, u8, u8),
}

/// xterm 256-colour cube component levels.
const CUBE_LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];

/// Resolve `color` against the theme.
#[must_use]
pub fn ansi_color(color: AnsiColor, theme: &Theme) -> Hsla {
    match color {
        AnsiColor::Indexed(ix) => theme.ansi.get(usize::from(ix)).copied().unwrap_or_else(|| {
            let (r, g, b) = extended_rgb(theme.appearance, ix);
            rgb8(r, g, b)
        }),
        AnsiColor::Rgb(r, g, b) => rgb8(r, g, b),
    }
}

/// The cube is literal in both appearances (index 196 *is* `#ff0000`); the
/// grey ramp means "de-emphasis", so light mode mirrors it to stay faint.
fn extended_rgb(appearance: Appearance, index: u8) -> (u8, u8, u8) {
    if let Some(step) = index.checked_sub(232) {
        let step = match appearance {
            Appearance::Dark => step,
            Appearance::Light => 23u8.saturating_sub(step),
        };
        let v = step.saturating_mul(10).saturating_add(8);
        return (v, v, v);
    }
    let n = index.saturating_sub(16);
    let level = |i: u8| CUBE_LEVELS.get(usize::from(i % 6)).copied().unwrap_or(0);
    (level(n / 36), level(n / 6), level(n))
}

#[must_use]
pub fn rgb8(red: u8, green: u8, blue: u8) -> Hsla {
    gpui::Rgba {
        r: f32::from(red) / 255.0,
        g: f32::from(green) / 255.0,
        b: f32::from(blue) / 255.0,
        a: 1.0,
    }
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cube_and_ramp() {
        assert_eq!(extended_rgb(Appearance::Dark, 16), (0, 0, 0));
        assert_eq!(extended_rgb(Appearance::Dark, 196), (255, 0, 0));
        assert_eq!(extended_rgb(Appearance::Dark, 21), (0, 0, 255));
        assert_eq!(extended_rgb(Appearance::Dark, 231), (255, 255, 255));
        assert_eq!(extended_rgb(Appearance::Dark, 232), (8, 8, 8));
        assert_eq!(extended_rgb(Appearance::Dark, 255), (238, 238, 238));
        assert_eq!(extended_rgb(Appearance::Light, 232), (238, 238, 238));
    }

    #[test]
    fn named_slots_come_from_the_theme() {
        let theme = Theme::dark();
        assert_eq!(ansi_color(AnsiColor::Indexed(1), &theme), theme.ansi[1]);
        assert_eq!(ansi_color(AnsiColor::Indexed(15), &theme), theme.ansi[15]);
    }
}
