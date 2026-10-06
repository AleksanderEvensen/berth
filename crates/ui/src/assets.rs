//! Embedded assets: icons (served to gpui's SVG renderer) and fonts.
//!
//! Icons are Solar Icons (Linear) by 480 Design, CC BY 4.0, plus a few
//! hand-drawn glyphs in the same 24-unit / 1.5px-stroke style.
//! Fonts are Geist and Geist Mono, SIL OFL 1.1 (see assets/fonts/licenses).

use std::borrow::Cow;

use gpui::{App, AssetSource, Result, SharedString};

macro_rules! icon_assets {
    ($(($const_name:ident, $path:literal)),+ $(,)?) => {
        pub mod icons {
            $(pub const $const_name: &str = concat!("icons/", $path, ".svg");)+
        }

        pub struct Assets;

        impl AssetSource for Assets {
            fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
                Ok(match path {
                    $(concat!("icons/", $path, ".svg") => Some(Cow::Borrowed(
                        include_bytes!(concat!("../assets/icons/", $path, ".svg")).as_slice(),
                    )),)+
                    _ => None,
                })
            }

            fn list(&self, path: &str) -> Result<Vec<SharedString>> {
                let all = [$(concat!("icons/", $path, ".svg")),+];
                Ok(all
                    .iter()
                    .filter(|p| p.starts_with(path))
                    .map(|p| SharedString::from(*p))
                    .collect())
            }
        }
    };
}

icon_assets![
    (BOX, "box"),
    (LAYERS, "layers"),
    (DISC, "disc"),
    (HARD_DRIVE, "hard-drive"),
    (NETWORK, "network"),
    (CHART, "chart"),
    (BROOM, "broom"),
    (HAMMER, "hammer"),
    (PLAY, "play"),
    (STOP, "stop"),
    (RESTART, "restart"),
    (TRASH, "trash-bin-minimalistic"),
    (LINK, "link"),
    (SEARCH, "magnifer"),
    (PLUS, "plus"),
    (SETTINGS, "settings"),
    (TERMINAL, "terminal"),
    (CHEVRON_DOWN, "alt-arrow-down"),
    (CHEVRON_RIGHT, "alt-arrow-right"),
    (ARROW_UP_RIGHT, "arrow-up-right"),
    (GLOBE, "global"),
    (MONITOR, "monitor"),
    (FOLDER, "folder"),
    (COPY, "copy"),
    (SORT, "sort"),
    (LIST, "list"),
    (MORE, "more-horizontal"),
    (CLOSE, "close"),
    (SPINNER, "spinner"),
    (DANGER, "danger-circle"),
];

const FONTS: &[&[u8]] = &[
    include_bytes!("../assets/fonts/Geist.ttf"),
    include_bytes!("../assets/fonts/Geist-Medium.ttf"),
    include_bytes!("../assets/fonts/Geist-SemiBold.ttf"),
    include_bytes!("../assets/fonts/Geist-Bold.ttf"),
    include_bytes!("../assets/fonts/GeistMono.ttf"),
    include_bytes!("../assets/fonts/GeistMono-Medium.ttf"),
];

pub fn register_fonts(cx: &App) -> anyhow::Result<()> {
    cx.text_system()
        .add_fonts(FONTS.iter().map(|f| Cow::Borrowed(*f)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_loads() {
        for path in Assets.list("icons/").unwrap() {
            let bytes = Assets.load(&path).unwrap().expect("icon present");
            let text = std::str::from_utf8(&bytes).unwrap();
            assert!(text.contains("<svg") && text.contains("viewBox"), "{path}");
        }
    }
}
