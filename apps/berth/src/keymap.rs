//! The keyboard map (NFR-U1): every app action and its keys, in one table
//! that both binds the keys and renders the reference on Settings →
//! Shortcuts, so the two can't drift apart.

use gpui::{App, KeyBinding, actions};

use crate::dialogs;

actions!(
    berth,
    [
        #[derive(Eq)]
        ToggleSidebar,
        #[derive(Eq)]
        FocusSearch,
        #[derive(Eq)]
        OpenSettings,
        #[derive(Eq)]
        ShowShortcuts,
        #[derive(Eq)]
        GoContainers,
        #[derive(Eq)]
        GoVolumes,
        #[derive(Eq)]
        GoImages,
        #[derive(Eq)]
        GoNetworks,
        #[derive(Eq)]
        GoActivity,
        #[derive(Eq)]
        GoCleanup,
        #[derive(Eq)]
        FocusNext,
        #[derive(Eq)]
        FocusPrev,
        #[derive(Eq)]
        SelectPrev,
        #[derive(Eq)]
        SelectNext,
        #[derive(Eq)]
        SelectFirst,
        #[derive(Eq)]
        SelectLast,
        #[derive(Eq)]
        ExtendPrev,
        #[derive(Eq)]
        ExtendNext,
        #[derive(Eq)]
        ExtendFirst,
        #[derive(Eq)]
        ExtendLast,
        #[derive(Eq)]
        CursorPrev,
        #[derive(Eq)]
        CursorNext,
        #[derive(Eq)]
        ToggleSelected,
        #[derive(Eq)]
        ScrollPageUp,
        #[derive(Eq)]
        ScrollPageDown,
        #[derive(Eq)]
        ScrollTop,
        #[derive(Eq)]
        ScrollBottom,
        #[derive(Eq)]
        SelectAll,
        #[derive(Eq)]
        ClearSelection,
        #[derive(Eq)]
        DeleteSelected,
        #[derive(Eq)]
        Collapse,
        #[derive(Eq)]
        Expand,
        #[derive(Eq)]
        ToggleRunning,
        #[derive(Eq)]
        Restart,
        #[derive(Eq)]
        OpenPort,
        #[derive(Eq)]
        ShowInfo,
        #[derive(Eq)]
        ShowStats,
        #[derive(Eq)]
        ShowLogs,
        #[derive(Eq)]
        ShowTerminal,
        #[derive(Eq)]
        ShowFiles,
        #[derive(Eq)]
        MenuPrev,
        #[derive(Eq)]
        MenuNext,
        #[derive(Eq)]
        MenuConfirm,
        #[derive(Eq)]
        MenuCancel,
        #[derive(Eq)]
        Quit
    ]
);

/// Anywhere in the window.
const SHELL: &str = "Shell";
/// Where list keys apply: anywhere in the shell except text inputs, the
/// terminal and the confirm dialog, which own these keys.
const LIST: &str = "Shell && !Input && !Terminal && !ConfirmDialog";
/// Key context of an open menu (the port menu). Deeper than [`LIST`], so
/// its keys win over the list's.
pub const MENU_CONTEXT: &str = "PortMenu";
/// Detail tabs also switch from inside the terminal, so it can be left by
/// keyboard.
const TABS: &str = "Shell && !ConfirmDialog";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    General,
    Lists,
    Containers,
    Dialogs,
}

impl Group {
    pub const ALL: [Self; 4] = [Self::General, Self::Lists, Self::Containers, Self::Dialogs];

    pub const fn label(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Lists => "Lists",
            Self::Containers => "Containers",
            Self::Dialogs => "Dialogs and menus",
        }
    }
}

type Bind = fn(&str, Option<&str>) -> KeyBinding;

/// One line of the keyboard map. `keys` use gpui's keystroke syntax
/// (`secondary` is cmd on macOS, ctrl elsewhere); alternatives are listed
/// side by side. Entries without `bind` document keys handled elsewhere
/// (focused controls, the port menu).
pub struct Shortcut {
    pub group: Group,
    pub label: &'static str,
    pub keys: &'static [&'static str],
    context: Option<&'static str>,
    bind: Option<Bind>,
}

macro_rules! shortcut {
    ($group:ident, $label:literal, [$($key:literal),+], $context:expr, $action:expr) => {
        Shortcut {
            group: Group::$group,
            label: $label,
            keys: &[$($key),+],
            context: $context,
            bind: Some(|key, context| KeyBinding::new(key, $action, context)),
        }
    };
    ($group:ident, $label:literal, [$($key:literal),+]) => {
        Shortcut {
            group: Group::$group,
            label: $label,
            keys: &[$($key),+],
            context: None,
            bind: None,
        }
    };
}

pub const SHORTCUTS: &[Shortcut] = &[
    shortcut!(
        General,
        "Go to Containers",
        ["secondary-1"],
        None,
        GoContainers
    ),
    shortcut!(General, "Go to Volumes", ["secondary-2"], None, GoVolumes),
    shortcut!(General, "Go to Images", ["secondary-3"], None, GoImages),
    shortcut!(General, "Go to Networks", ["secondary-4"], None, GoNetworks),
    shortcut!(General, "Go to Activity", ["secondary-5"], None, GoActivity),
    shortcut!(General, "Go to Clean up", ["secondary-6"], None, GoCleanup),
    shortcut!(
        General,
        "Search containers",
        ["secondary-f"],
        None,
        FocusSearch
    ),
    shortcut!(
        General,
        "Toggle sidebar",
        ["secondary-b"],
        None,
        ToggleSidebar
    ),
    shortcut!(General, "Settings", ["secondary-,"], None, OpenSettings),
    shortcut!(
        General,
        "Keyboard shortcuts",
        ["secondary-/"],
        None,
        ShowShortcuts
    ),
    shortcut!(General, "Leave settings", ["escape"]),
    shortcut!(
        General,
        "Scroll up a page",
        ["pageup"],
        Some(LIST),
        ScrollPageUp
    ),
    shortcut!(
        General,
        "Scroll down a page",
        ["pagedown"],
        Some(LIST),
        ScrollPageDown
    ),
    shortcut!(
        General,
        "Scroll to the top",
        ["secondary-home"],
        Some(LIST),
        ScrollTop
    ),
    shortcut!(
        General,
        "Scroll to the bottom (Logs: follow new lines)",
        ["secondary-end"],
        Some(LIST),
        ScrollBottom
    ),
    shortcut!(
        General,
        "Scroll (screens without a list)",
        ["up", "down", "home", "end"]
    ),
    shortcut!(General, "Next control", ["tab"], Some(SHELL), FocusNext),
    shortcut!(
        General,
        "Previous control",
        ["shift-tab"],
        Some(SHELL),
        FocusPrev
    ),
    shortcut!(General, "Press the focused control", ["enter", "space"]),
    shortcut!(General, "Quit", ["secondary-q"], None, Quit),
    shortcut!(Lists, "Select previous row", ["up"], Some(LIST), SelectPrev),
    shortcut!(Lists, "Select next row", ["down"], Some(LIST), SelectNext),
    shortcut!(Lists, "Select first row", ["home"], Some(LIST), SelectFirst),
    shortcut!(Lists, "Select last row", ["end"], Some(LIST), SelectLast),
    shortcut!(
        Lists,
        "Extend selection up",
        ["shift-up"],
        Some(LIST),
        ExtendPrev
    ),
    shortcut!(
        Lists,
        "Extend selection down",
        ["shift-down"],
        Some(LIST),
        ExtendNext
    ),
    shortcut!(
        Lists,
        "Extend selection to the first row",
        ["shift-home"],
        Some(LIST),
        ExtendFirst
    ),
    shortcut!(
        Lists,
        "Extend selection to the last row",
        ["shift-end"],
        Some(LIST),
        ExtendLast
    ),
    shortcut!(
        Lists,
        "Move the cursor up",
        ["secondary-up"],
        Some(LIST),
        CursorPrev
    ),
    shortcut!(
        Lists,
        "Move the cursor down",
        ["secondary-down"],
        Some(LIST),
        CursorNext
    ),
    shortcut!(
        Lists,
        "Add or remove the row under the cursor",
        ["ctrl-space"],
        Some(LIST),
        ToggleSelected
    ),
    shortcut!(Lists, "Select all", ["secondary-a"], Some(LIST), SelectAll),
    shortcut!(
        Lists,
        "Clear selection",
        ["escape"],
        Some(LIST),
        ClearSelection
    ),
    shortcut!(
        Lists,
        "Delete selected",
        ["delete", "secondary-backspace"],
        Some(LIST),
        DeleteSelected
    ),
    shortcut!(
        Containers,
        "Start or stop",
        ["secondary-enter"],
        Some(LIST),
        ToggleRunning
    ),
    shortcut!(Containers, "Restart", ["secondary-r"], Some(LIST), Restart),
    shortcut!(
        Containers,
        "Open published port",
        ["secondary-o"],
        Some(LIST),
        OpenPort
    ),
    shortcut!(Containers, "Collapse stack", ["left"], Some(LIST), Collapse),
    shortcut!(Containers, "Expand stack", ["right"], Some(LIST), Expand),
    shortcut!(
        Containers,
        "Scroll the terminal's history (typing returns to the bottom)",
        ["shift-pageup", "shift-pagedown"]
    ),
    shortcut!(Containers, "Select all terminal output", ["ctrl-shift-a"]),
    shortcut!(Containers, "Copy the terminal selection", ["ctrl-shift-c"]),
    shortcut!(Containers, "Paste into the terminal", ["ctrl-shift-v"]),
    shortcut!(Containers, "Info tab", ["alt-1"], Some(TABS), ShowInfo),
    shortcut!(Containers, "Stats tab", ["alt-2"], Some(TABS), ShowStats),
    shortcut!(Containers, "Logs tab", ["alt-3"], Some(TABS), ShowLogs),
    shortcut!(
        Containers,
        "Terminal tab",
        ["alt-4"],
        Some(TABS),
        ShowTerminal
    ),
    shortcut!(Containers, "Files tab", ["alt-5"], Some(TABS), ShowFiles),
    shortcut!(
        Dialogs,
        "Confirm (Enter on a focused button presses it)",
        ["enter"],
        Some(dialogs::CONFIRM_CONTEXT),
        dialogs::Confirm
    ),
    shortcut!(
        Dialogs,
        "Cancel",
        ["escape"],
        Some(dialogs::CONFIRM_CONTEXT),
        dialogs::Cancel
    ),
    shortcut!(
        Dialogs,
        "Previous menu item",
        ["up"],
        Some(MENU_CONTEXT),
        MenuPrev
    ),
    shortcut!(
        Dialogs,
        "Next menu item",
        ["down"],
        Some(MENU_CONTEXT),
        MenuNext
    ),
    shortcut!(
        Dialogs,
        "Open menu item",
        ["enter"],
        Some(MENU_CONTEXT),
        MenuConfirm
    ),
    shortcut!(
        Dialogs,
        "Close menu",
        ["escape"],
        Some(MENU_CONTEXT),
        MenuCancel
    ),
];

/// Bind every key in [`SHORTCUTS`].
pub fn bind(cx: &mut App) {
    cx.bind_keys(key_bindings());
}

fn key_bindings() -> Vec<KeyBinding> {
    SHORTCUTS
        .iter()
        .flat_map(|shortcut| {
            shortcut.bind.into_iter().flat_map(move |bind| {
                shortcut
                    .keys
                    .iter()
                    .map(move |key| bind(key, shortcut.context))
            })
        })
        .collect()
}

/// Keycap labels for a keystroke in gpui syntax, as this platform names
/// them: `secondary-backspace` is `["Ctrl", "Backspace"]` on Linux and
/// `["⌘", "⌫"]` on macOS.
pub fn keycaps(keystroke: &str) -> Vec<&'static str> {
    keycaps_for(keystroke, cfg!(target_os = "macos"))
}

fn keycaps_for(keystroke: &str, mac: bool) -> Vec<&'static str> {
    keystroke
        .split('-')
        .map(|part| match (part, mac) {
            ("secondary" | "cmd", true) => "⌘",
            ("secondary" | "ctrl", false) => "Ctrl",
            ("ctrl", true) => "⌃",
            ("cmd", false) => "Super",
            ("shift", true) => "⇧",
            ("shift", false) => "Shift",
            ("alt", true) => "⌥",
            ("alt", false) => "Alt",
            ("enter", true) => "↩",
            ("enter", false) => "Enter",
            ("backspace", true) => "⌫",
            ("backspace", false) => "Backspace",
            ("delete", true) => "⌦",
            ("delete", false) => "Delete",
            ("escape", _) => "Esc",
            ("tab", _) => "Tab",
            ("space", _) => "Space",
            ("home", _) => "Home",
            ("pageup", _) => "Page Up",
            ("pagedown", _) => "Page Down",
            ("end", _) => "End",
            ("up", _) => "↑",
            ("down", _) => "↓",
            ("left", _) => "←",
            ("right", _) => "→",
            (",", _) => ",",
            ("/", _) => "/",
            ("a", _) => "A",
            ("c", _) => "C",
            ("v", _) => "V",
            ("b", _) => "B",
            ("f", _) => "F",
            ("o", _) => "O",
            ("q", _) => "Q",
            ("r", _) => "R",
            ("1", _) => "1",
            ("2", _) => "2",
            ("3", _) => "3",
            ("4", _) => "4",
            ("5", _) => "5",
            ("6", _) => "6",
            _ => "?",
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use gpui::Keystroke;

    use super::*;

    #[test]
    fn every_key_parses_and_has_a_label() {
        for shortcut in SHORTCUTS {
            for key in shortcut.keys {
                assert!(Keystroke::parse(key).is_ok(), "{key} doesn't parse");
                for mac in [false, true] {
                    assert!(!keycaps_for(key, mac).contains(&"?"), "{key} has no label");
                }
            }
        }
    }

    #[test]
    fn bound_keys_are_unique_per_context() {
        let mut seen = HashSet::new();
        for shortcut in SHORTCUTS.iter().filter(|s| s.bind.is_some()) {
            for key in shortcut.keys {
                assert!(
                    seen.insert((*key, shortcut.context)),
                    "{key} is bound twice in {:?}",
                    shortcut.context
                );
            }
        }
    }

    /// The action a keystroke runs first with `contexts` focused (outermost
    /// first), as gpui resolves it.
    fn resolve(key: &str, contexts: &[&str]) -> Option<Box<dyn gpui::Action>> {
        let keymap = gpui::Keymap::new(key_bindings());
        let contexts: Vec<gpui::KeyContext> = contexts
            .iter()
            .map(|c| gpui::KeyContext::parse(c).unwrap())
            .collect();
        let (bindings, _) = keymap.bindings_for_input(&[Keystroke::parse(key).unwrap()], &contexts);
        bindings.first().map(|b| b.action().boxed_clone())
    }

    fn runs(key: &str, contexts: &[&str], action: &dyn gpui::Action) -> bool {
        resolve(key, contexts).is_some_and(|a| a.partial_eq(action))
    }

    #[test]
    fn keys_resolve_in_their_context() {
        let shell = ["Shell"];
        assert!(runs("down", &shell, &SelectNext));
        assert!(runs("escape", &shell, &ClearSelection));
        assert!(runs("ctrl-space", &shell, &ToggleSelected));
        // The open menu and dialog take their keys from the list.
        assert!(runs("down", &["Shell", MENU_CONTEXT], &MenuNext));
        assert!(runs("escape", &["Shell", MENU_CONTEXT], &MenuCancel));
        assert!(runs("enter", &["Shell", MENU_CONTEXT], &MenuConfirm));
        let dialog = ["Shell", dialogs::CONFIRM_CONTEXT];
        assert!(runs("escape", &dialog, &dialogs::Cancel));
        assert!(runs("enter", &dialog, &dialogs::Confirm));
        assert!(resolve("down", &dialog).is_none());
        assert!(resolve("alt-1", &dialog).is_none());
        // Text fields and the terminal keep their keys, but the tab keys
        // still lead out of the terminal.
        assert!(resolve("down", &["Shell", "Input"]).is_none());
        assert!(resolve("delete", &["Shell", "Terminal"]).is_none());
        assert!(runs("alt-1", &["Shell", "Terminal"], &ShowInfo));
        // Global keys work anywhere.
        assert!(runs("secondary-1", &["Shell", "Input"], &GoContainers));
    }

    #[test]
    fn labels_follow_the_platform() {
        assert_eq!(
            keycaps_for("secondary-backspace", false),
            ["Ctrl", "Backspace"]
        );
        assert_eq!(keycaps_for("secondary-backspace", true), ["⌘", "⌫"]);
        assert_eq!(keycaps_for("shift-up", false), ["Shift", "↑"]);
    }

    #[test]
    fn every_group_has_shortcuts() {
        for group in Group::ALL {
            assert!(SHORTCUTS.iter().any(|s| s.group == group), "{group:?}");
        }
    }
}
