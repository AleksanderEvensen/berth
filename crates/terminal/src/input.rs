//! Keystroke and paste → TTY byte encoding. Pure, so it is unit-tested.
//!
//! Adapted from zeron's `crates/ui/src/terminal/view.rs` (MIT).

use gpui::Modifiers;

/// Encode a keystroke. `None` means "not ours": the event falls through to
/// the app keymap (platform-key combos, ctrl+shift shortcuts, unknown keys).
/// `app_cursor` switches arrows/home/end from CSI to SS3 (DECCKM).
#[must_use]
pub fn keystroke_bytes(
    key: &str,
    key_char: Option<&str>,
    mods: Modifiers,
    app_cursor: bool,
) -> Option<Vec<u8>> {
    if mods.platform || (mods.control && mods.shift) {
        return None;
    }
    if mods.alt {
        let inner = keystroke_bytes(key, key_char, Modifiers { alt: false, ..mods }, app_cursor)?;
        let mut out = vec![0x1b];
        out.extend(inner);
        return Some(out);
    }
    if mods.control {
        return control_bytes(key);
    }
    let seq = |csi: &[u8], ss3: &[u8]| Some(if app_cursor { ss3 } else { csi }.to_vec());
    let fixed: &[u8] = match key {
        "enter" => b"\r",
        "backspace" => b"\x7f",
        "tab" if mods.shift => b"\x1b[Z",
        "tab" => b"\t",
        "escape" => b"\x1b",
        "space" => b" ",
        "up" => return seq(b"\x1b[A", b"\x1bOA"),
        "down" => return seq(b"\x1b[B", b"\x1bOB"),
        "right" => return seq(b"\x1b[C", b"\x1bOC"),
        "left" => return seq(b"\x1b[D", b"\x1bOD"),
        "home" => return seq(b"\x1b[H", b"\x1bOH"),
        "end" => return seq(b"\x1b[F", b"\x1bOF"),
        "insert" => b"\x1b[2~",
        "delete" => b"\x1b[3~",
        "pageup" => b"\x1b[5~",
        "pagedown" => b"\x1b[6~",
        "f1" => b"\x1bOP",
        "f2" => b"\x1bOQ",
        "f3" => b"\x1bOR",
        "f4" => b"\x1bOS",
        "f5" => b"\x1b[15~",
        "f6" => b"\x1b[17~",
        "f7" => b"\x1b[18~",
        "f8" => b"\x1b[19~",
        "f9" => b"\x1b[20~",
        "f10" => b"\x1b[21~",
        "f11" => b"\x1b[23~",
        "f12" => b"\x1b[24~",
        _ => {
            // Printable: prefer the typed character (IME/shift-aware), then
            // single-char key names.
            let text = key_char
                .filter(|c| !c.is_empty())
                .or_else(|| (key.chars().count() == 1).then_some(key))?;
            return Some(text.as_bytes().to_vec());
        }
    };
    Some(fixed.to_vec())
}

/// Ctrl-key caret notation.
fn control_bytes(key: &str) -> Option<Vec<u8>> {
    let byte = match key {
        "space" | "@" => 0x00,
        "backspace" => 0x08,
        "enter" => b'\r',
        "[" => 0x1b,
        "\\" => 0x1c,
        "]" => 0x1d,
        "^" => 0x1e,
        "_" | "/" => 0x1f,
        "?" => 0x7f,
        _ => {
            let mut chars = key.chars();
            let c = chars.next().filter(char::is_ascii_lowercase)?;
            if chars.next().is_some() {
                return None;
            }
            u8::try_from(c).ok()?.checked_sub(b'a')?.checked_add(1)?
        }
    };
    Some(vec![byte])
}

/// Pasted text for the TTY, bracketed when the program asked for it. The
/// one sequence that could break out of a bracketed paste is stripped.
#[must_use]
pub fn paste_bytes(text: &str, bracketed: bool) -> Vec<u8> {
    let sanitized = text.replace("\x1b[201~", "");
    if bracketed {
        let mut out = b"\x1b[200~".to_vec();
        out.extend_from_slice(sanitized.as_bytes());
        out.extend_from_slice(b"\x1b[201~");
        out
    } else {
        sanitized.into_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn none() -> Modifiers {
        Modifiers::default()
    }

    #[test]
    fn printables_and_named_keys() {
        assert_eq!(
            keystroke_bytes("a", Some("A"), none(), false),
            Some(b"A".to_vec())
        );
        assert_eq!(
            keystroke_bytes("/", None, none(), false),
            Some(b"/".to_vec())
        );
        assert_eq!(
            keystroke_bytes("enter", None, none(), false),
            Some(b"\r".to_vec())
        );
        assert_eq!(
            keystroke_bytes("backspace", None, none(), false),
            Some(vec![0x7f])
        );
        assert_eq!(keystroke_bytes("capslock", None, none(), false), None);
        assert_eq!(
            keystroke_bytes("up", None, none(), false),
            Some(b"\x1b[A".to_vec())
        );
        assert_eq!(
            keystroke_bytes("up", None, none(), true),
            Some(b"\x1bOA".to_vec())
        );
    }

    #[test]
    fn modifiers() {
        let ctrl = Modifiers {
            control: true,
            ..none()
        };
        assert_eq!(
            keystroke_bytes("c", Some("c"), ctrl, false),
            Some(vec![0x03])
        );
        assert_eq!(keystroke_bytes("z", None, ctrl, false), Some(vec![0x1a]));
        assert_eq!(keystroke_bytes("1", Some("1"), ctrl, false), None);
        let alt = Modifiers {
            alt: true,
            ..none()
        };
        assert_eq!(
            keystroke_bytes("b", Some("b"), alt, false),
            Some(vec![0x1b, b'b'])
        );
        let ctrl_shift = Modifiers {
            control: true,
            shift: true,
            ..none()
        };
        assert_eq!(keystroke_bytes("c", Some("C"), ctrl_shift, false), None);
    }

    #[test]
    fn paste_brackets_and_sanitizes() {
        assert_eq!(paste_bytes("hi", false), b"hi".to_vec());
        assert_eq!(
            paste_bytes("a\x1b[201~b", true),
            b"\x1b[200~ab\x1b[201~".to_vec()
        );
    }
}
