//! Save every native caption-button drawing as a PNG, to check how the
//! desktop theme renders: `gtk_buttons DIR [SCALE]`.
#![allow(clippy::unwrap_used, clippy::expect_used)]

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
use gpui_window_chrome::{ButtonState, CaptionButton, NativeButtons, Side};

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let dir = std::path::PathBuf::from(args.get(1).map_or("/tmp/gtk-buttons", String::as_str));
    std::fs::create_dir_all(&dir).unwrap();
    let scale: f32 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(1.0);
    let native = NativeButtons::get().expect("no native buttons on this platform");
    for dark in [true, false] {
        native.set_dark(dark);
        for maximize in [CaptionButton::Maximize, CaptionButton::Restore] {
            let buttons = [CaptionButton::Minimize, maximize, CaptionButton::Close];
            let row = native
                .row(Side::Right, &buttons)
                .expect("the theme can't lay out the buttons");
            println!("dark {dark}, {maximize:?}: row width {:?}", row.width());
            for button in buttons {
                for state in [
                    ButtonState::Normal,
                    ButtonState::Hover,
                    ButtonState::Pressed,
                ] {
                    for focused in [true, false] {
                        let drawing = row.pixels(button, state, focused, scale).expect("draw");
                        // Straight BGRA to RGBA.
                        let mut pixels = drawing.pixels;
                        for pixel in pixels.as_chunks_mut::<4>().0 {
                            pixel.swap(0, 2);
                        }
                        let name = format!(
                            "{}-{button:?}-{}-{state:?}.png",
                            if dark { "dark" } else { "light" },
                            if focused { "active" } else { "backdrop" },
                        );
                        image::RgbaImage::from_raw(drawing.width, drawing.height, pixels)
                            .unwrap()
                            .save(dir.join(name))
                            .unwrap();
                    }
                }
            }
        }
    }
}

#[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
fn main() {
    eprintln!("GTK caption buttons exist only on Linux and FreeBSD");
}
