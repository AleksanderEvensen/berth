//! Persisted user settings (FR-14): one JSON file under the platform config
//! directory — `$XDG_CONFIG_HOME/berth/settings.json` on Linux.
//!
//! Loading is forgiving so a file from an older or newer Berth never costs
//! the user their other settings: each field is read on its own (a value
//! this version doesn't understand falls back to its default), keys this
//! version doesn't know are kept and written back, and a file that can't be
//! used as-is is backed up to `settings.json.bak` before anything replaces it.
//! If it can't be read or backed up, [`LoadProblem::saving_blocked`] is set
//! and callers must not save over it.

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

const FILE_NAME: &str = "settings.json";

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    pub appearance: AppearanceMode,
    pub accent: Accent,
    /// Skip decorative animations, including spinners (NFR-U4).
    pub reduce_motion: bool,
    pub sidebar_collapsed: bool,
    /// Titlebar buttons where Berth draws them (Linux).
    pub window_buttons: WindowButtons,
    pub engine: EngineChoice,
    /// The OS light/dark scheme last seen. Linux only learns it from the
    /// desktop portal after the first frame, so this avoids starting in the
    /// wrong palette.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_os_scheme: Option<Scheme>,
    /// Keys written by a newer Berth, preserved verbatim.
    #[serde(flatten)]
    pub unknown: Map<String, Value>,
}

/// A concrete light/dark scheme.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Scheme {
    Light,
    Dark,
}

/// What [`Settings::load`] had to work around, for the UI to report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadProblem {
    pub message: String,
    /// Where the original file was copied, if it was.
    pub backup: Option<PathBuf>,
    /// The original couldn't be read or backed up: saving would destroy it.
    pub saving_blocked: bool,
}

/// Light/dark preference: follow the OS, or pin one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AppearanceMode {
    #[default]
    System,
    Light,
    Dark,
}

impl AppearanceMode {
    pub const ALL: [Self; 3] = [Self::System, Self::Light, Self::Dark];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::System => "System",
            Self::Light => "Light",
            Self::Dark => "Dark",
        }
    }
}

/// The interactive accent hue. Each appearance has its own authored tone
/// (see `berth_ui::Theme`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Accent {
    #[default]
    Indigo,
    Orange,
    Amber,
    Green,
    Cyan,
    Blue,
    Pink,
}

impl Accent {
    pub const ALL: [Self; 7] = [
        Self::Indigo,
        Self::Orange,
        Self::Amber,
        Self::Green,
        Self::Cyan,
        Self::Blue,
        Self::Pink,
    ];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Indigo => "Indigo",
            Self::Orange => "Orange",
            Self::Amber => "Amber",
            Self::Green => "Green",
            Self::Cyan => "Cyan",
            Self::Blue => "Blue",
            Self::Pink => "Pink",
        }
    }
}

/// Which titlebar buttons to draw where Berth draws its own (Linux).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WindowButtons {
    /// The desktop's buttons; only close while a tiling window manager
    /// tiles the window.
    #[default]
    Auto,
    /// The desktop's buttons, always.
    All,
    CloseOnly,
    Hidden,
}

impl WindowButtons {
    pub const ALL: [Self; 4] = [Self::Auto, Self::All, Self::CloseOnly, Self::Hidden];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Auto => "Automatic",
            Self::All => "All",
            Self::CloseOnly => "Close only",
            Self::Hidden => "None",
        }
    }
}

/// Which engine to connect to (story H1).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum EngineChoice {
    /// `DOCKER_HOST`, the current Docker context, then the default sockets.
    #[default]
    Auto,
    /// A named Docker context, regardless of the CLI's current one.
    Context { name: String },
    /// A fixed `DOCKER_HOST`-style address: `unix://…`, `tcp://…`, `npipe://…`.
    Host { host: String },
}

impl Settings {
    /// `settings.json` in the platform config directory, if there is one.
    #[must_use]
    pub fn default_path() -> Option<PathBuf> {
        dirs::config_dir().map(|dir| dir.join("berth").join(FILE_NAME))
    }

    /// Load `path`. Missing → defaults. Anything that had to be dropped is
    /// reported, after the original file was copied to `settings.json.bak`.
    #[must_use]
    pub fn load(path: &Path) -> (Self, Option<LoadProblem>) {
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return (Self::default(), None);
            }
            Err(error) => {
                // Unreadable but present: nothing to back up, so never overwrite.
                let message = format!("Couldn't read {}: {error}", path.display());
                log::warn!("{message}");
                return (
                    Self::default(),
                    Some(LoadProblem {
                        message,
                        backup: None,
                        saving_blocked: true,
                    }),
                );
            }
        };
        let (settings, issues) = match serde_json::from_slice::<Value>(&bytes) {
            Ok(value) => Self::from_value(value),
            Err(error) => (Self::default(), vec![format!("not valid JSON ({error})")]),
        };
        if issues.is_empty() {
            return (settings, None);
        }
        let backup = path.with_extension("json.bak");
        let (backup, backup_error) = match std::fs::write(&backup, &bytes) {
            Ok(()) => (Some(backup), String::new()),
            Err(error) => (
                None,
                format!(" Couldn't back it up to {}: {error}.", backup.display()),
            ),
        };
        let message = format!(
            "Some settings in {} couldn't be read and were reset: {}.{backup_error}",
            path.display(),
            issues.join("; ")
        );
        log::warn!("{message}");
        let saving_blocked = backup.is_none();
        (
            settings,
            Some(LoadProblem {
                message,
                backup,
                saving_blocked,
            }),
        )
    }

    /// Read each field on its own; returns what had to be reset.
    fn from_value(value: Value) -> (Self, Vec<String>) {
        fn take<T: DeserializeOwned>(
            map: &mut Map<String, Value>,
            key: &str,
            slot: &mut T,
            issues: &mut Vec<String>,
        ) {
            let Some(raw) = map.remove(key) else { return };
            match serde_json::from_value(raw) {
                Ok(value) => *slot = value,
                Err(error) => issues.push(format!("{key} ({error})")),
            }
        }

        let Value::Object(mut map) = value else {
            return (Self::default(), vec!["not a JSON object".into()]);
        };
        let mut settings = Self::default();
        let mut issues = Vec::new();
        take(
            &mut map,
            "appearance",
            &mut settings.appearance,
            &mut issues,
        );
        take(&mut map, "accent", &mut settings.accent, &mut issues);
        take(
            &mut map,
            "reduceMotion",
            &mut settings.reduce_motion,
            &mut issues,
        );
        take(
            &mut map,
            "sidebarCollapsed",
            &mut settings.sidebar_collapsed,
            &mut issues,
        );
        take(
            &mut map,
            "windowButtons",
            &mut settings.window_buttons,
            &mut issues,
        );
        take(&mut map, "engine", &mut settings.engine, &mut issues);
        take(
            &mut map,
            "lastOsScheme",
            &mut settings.last_os_scheme,
            &mut issues,
        );
        settings.unknown = map;
        (settings, issues)
    }

    /// Write `path` atomically (unique temp file + rename), creating its
    /// directory. Concurrent saves, even from other processes, each land
    /// whole; the last rename wins.
    ///
    /// # Errors
    /// Returns the I/O error if the directory, temp file or rename fails.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        let dir = path.parent().unwrap_or_else(|| Path::new(""));
        std::fs::create_dir_all(dir)?;
        let json = serde_json::to_vec_pretty(self).map_err(io::Error::other)?;
        let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
        tmp.write_all(&json)?;
        tmp.persist(path).map_err(|error| error.error)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/settings.json");
        let settings = Settings {
            appearance: AppearanceMode::Light,
            accent: Accent::Green,
            reduce_motion: true,
            sidebar_collapsed: true,
            window_buttons: WindowButtons::CloseOnly,
            engine: EngineChoice::Host {
                host: "tcp://build:2376".into(),
            },
            last_os_scheme: Some(Scheme::Dark),
            unknown: Map::new(),
        };
        settings.save(&path).unwrap();
        assert_eq!(Settings::load(&path), (settings, None));
        assert_eq!(
            std::fs::read_dir(path.parent().unwrap()).unwrap().count(),
            1
        );
    }

    #[test]
    fn unsynchronized_saves_dont_collide() {
        // Separate app instances share no lock; each save must still land.
        let dir = tempfile::tempdir().unwrap();
        let path = std::sync::Arc::new(dir.path().join("settings.json"));
        let threads: Vec<_> = Accent::ALL
            .into_iter()
            .map(|accent| {
                let path = path.clone();
                std::thread::spawn(move || {
                    let settings = Settings {
                        accent,
                        ..Settings::default()
                    };
                    for _ in 0..50 {
                        settings.save(&path).unwrap();
                    }
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
        assert_eq!(Settings::load(&path).1, None);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn failed_backup_blocks_saving() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::create_dir(path.with_extension("json.bak")).unwrap();
        std::fs::write(&path, r#"{"accent":"futureHue","appearance":"light"}"#).unwrap();
        let (settings, problem) = Settings::load(&path);
        assert_eq!(settings.appearance, AppearanceMode::Light);
        let problem = problem.unwrap();
        assert_eq!(problem.backup, None);
        assert!(problem.saving_blocked);
        assert!(
            problem.message.contains("back it up"),
            "{}",
            problem.message
        );
    }

    #[cfg(unix)]
    #[test]
    fn unreadable_file_blocks_saving() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, "{}").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
        if std::fs::read(&path).is_ok() {
            return; // root ignores permissions
        }
        let (settings, problem) = Settings::load(&path);
        assert_eq!(settings, Settings::default());
        assert!(problem.unwrap().saving_blocked);
    }

    #[test]
    fn missing_file_is_default() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            Settings::load(&dir.path().join("settings.json")),
            (Settings::default(), None)
        );
    }

    #[test]
    fn corrupt_file_is_backed_up_and_reported() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, "{ not json").unwrap();
        let (settings, problem) = Settings::load(&path);
        assert_eq!(settings, Settings::default());
        let problem = problem.unwrap();
        assert!(!problem.saving_blocked);
        let backup = problem.backup.unwrap();
        assert_eq!(std::fs::read_to_string(backup).unwrap(), "{ not json");
    }

    #[test]
    fn one_bad_value_keeps_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let original = r#"{
            "accent": "purple",
            "appearance": "light",
            "engine": { "kind": "host", "host": "tcp://box:2375" }
        }"#;
        std::fs::write(&path, original).unwrap();
        let (settings, problem) = Settings::load(&path);
        assert_eq!(settings.accent, Accent::Indigo);
        assert_eq!(settings.appearance, AppearanceMode::Light);
        assert_eq!(
            settings.engine,
            EngineChoice::Host {
                host: "tcp://box:2375".into()
            }
        );
        let problem = problem.unwrap();
        assert!(problem.message.contains("accent"), "{}", problem.message);
        assert_eq!(
            std::fs::read_to_string(problem.backup.unwrap()).unwrap(),
            original
        );
    }

    #[test]
    fn unknown_keys_survive_a_save() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(
            &path,
            r#"{ "accent": "pink", "fromTheFuture": { "a": 1 } }"#,
        )
        .unwrap();
        let (mut settings, problem) = Settings::load(&path);
        assert_eq!(problem, None);
        assert_eq!(settings.accent, Accent::Pink);
        settings.sidebar_collapsed = true;
        settings.save(&path).unwrap();
        let saved: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(saved["fromTheFuture"], serde_json::json!({ "a": 1 }));
        assert_eq!(saved["sidebarCollapsed"], true);
    }

    #[test]
    fn wire_format_is_stable() {
        let json = serde_json::to_value(Settings {
            engine: EngineChoice::Host {
                host: "unix:///run/podman.sock".into(),
            },
            ..Settings::default()
        })
        .unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "appearance": "system",
                "accent": "indigo",
                "reduceMotion": false,
                "sidebarCollapsed": false,
                "windowButtons": "auto",
                "engine": { "kind": "host", "host": "unix:///run/podman.sock" },
            })
        );
    }
}
