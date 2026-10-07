//! The live settings: the in-memory [`Settings`] every view reads, ordered
//! writes to disk, and turning appearance choices into the active theme.
//!
//! Colours are read at paint time (`Theme::of(cx).text`), so a theme change
//! only shows after `refresh_windows`, which also drops gpui's per-view
//! caches for the frame — `notify` on a single view would not be enough.
//!
//! Writes: every change bumps a revision. A debounced task hands the latest
//! snapshot to the background executor; the quit flush writes synchronously.
//! Both go through [`Writer`], which serializes them and refuses to write a
//! revision older than the one on disk — cancelling a task can't stop a
//! write that is already running, so ordering has to be enforced here.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use berth_settings::{AppearanceMode, EngineChoice, Scheme, Settings};
use berth_ui::{Appearance, Theme};
use gpui::{App, Global, SharedString, Subscription, Task, Window};

/// Coalesces bursts of changes (clicking through accents) into one write.
const SAVE_DEBOUNCE: Duration = Duration::from_millis(400);

/// The engine choice, shared with the connector, which runs off the UI
/// thread and reads it at the start of every session.
pub type EngineTarget = Arc<Mutex<EngineChoice>>;

/// Serializes writes of one settings file; remembers the newest revision on
/// disk so a stale snapshot is never written over a newer one.
#[derive(Default)]
struct Writer {
    written: Mutex<u64>,
}

impl Writer {
    #[expect(
        clippy::significant_drop_tightening,
        reason = "the lock is what serializes the write"
    )]
    fn write(&self, path: &Path, settings: &Settings, revision: u64) -> std::io::Result<()> {
        let mut written = self.written.lock().unwrap_or_else(PoisonError::into_inner);
        if revision <= *written {
            return Ok(());
        }
        settings.save(path)?;
        *written = revision;
        Ok(())
    }

    fn written(&self) -> u64 {
        *self.written.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

pub struct Preferences {
    current: Settings,
    revision: u64,
    /// `None` when the platform has no config directory: settings then
    /// live for the session only.
    path: Option<PathBuf>,
    writer: Arc<Writer>,
    /// What loading had to reset; stays for the session.
    load_problem: Option<SharedString>,
    /// The last write failed; cleared by the next successful write.
    save_error: Option<SharedString>,
    /// The OS appearance last reported.
    system: Appearance,
    engine: EngineTarget,
    save: Option<Task<()>>,
}

impl Global for Preferences {}

/// Load settings from disk and install the matching theme. Call before the
/// first window opens so it paints in the right palette.
pub fn init(cx: &mut App) {
    let path = Settings::default_path();
    let (current, problem) = path.as_deref().map(Settings::load).unwrap_or_default();
    if let Some(path) = &path {
        log::info!("settings: {}", path.display());
    }
    let problem = problem.map(|problem| match problem.backup {
        Some(backup) => format!(
            "{} The original is in {}.",
            problem.message,
            backup.display()
        ),
        None => problem.message,
    });
    let system = current.last_os_scheme.map_or_else(
        || Appearance::from_window(cx.window_appearance()),
        from_scheme,
    );
    cx.set_global(Preferences {
        engine: Arc::new(Mutex::new(current.engine.clone())),
        current,
        revision: 0,
        path,
        writer: Arc::default(),
        load_problem: problem.map(Into::into),
        save_error: None,
        system,
        save: None,
    });
    apply_theme(cx);
    cx.on_app_quit(|cx| {
        flush(cx);
        async {}
    })
    .detach();
}

pub fn get(cx: &App) -> &Settings {
    &cx.global::<Preferences>().current
}

pub fn engine_target(cx: &App) -> EngineTarget {
    cx.global::<Preferences>().engine.clone()
}

/// The settings problem to report: a failing save, else what loading reset.
pub fn problem(cx: &App) -> Option<SharedString> {
    let prefs = cx.global::<Preferences>();
    prefs
        .save_error
        .clone()
        .or_else(|| prefs.load_problem.clone())
}

/// Whether the last write failed (so "save again" makes sense).
pub fn save_failed(cx: &App) -> bool {
    cx.global::<Preferences>().save_error.is_some()
}

/// Change settings; re-themes if the appearance moved and schedules a save.
/// Returns whether anything changed.
pub fn update(cx: &mut App, change: impl FnOnce(&mut Settings)) -> bool {
    let prefs = cx.global_mut::<Preferences>();
    let before = prefs.current.clone();
    change(&mut prefs.current);
    if prefs.current == before {
        return false;
    }
    prefs.revision = prefs.revision.wrapping_add(1);
    if prefs.current.engine != before.engine {
        *prefs.engine.lock().unwrap_or_else(PoisonError::into_inner) = prefs.current.engine.clone();
    }
    apply_theme(cx);
    schedule_save(cx);
    true
}

/// Try writing again after a failure.
pub fn retry_save(cx: &mut App) {
    schedule_save(cx);
}

/// Follow OS light/dark switches while the mode is `System`.
///
/// The window is asked directly: before any window exists the platform's
/// answer can be stale. On Linux even the window starts out reporting Light
/// until the desktop portal answers — it then always notifies — so an
/// initial Light there is not trusted over the remembered scheme.
pub fn observe_system_appearance(window: &Window, cx: &mut App) -> Subscription {
    let initial = Appearance::from_window(window.appearance());
    if !(cfg!(any(target_os = "linux", target_os = "freebsd")) && initial == Appearance::Light) {
        set_system(initial, cx);
    }
    window.observe_window_appearance(|window, cx| {
        set_system(Appearance::from_window(window.appearance()), cx);
    })
}

fn set_system(system: Appearance, cx: &mut App) {
    let prefs = cx.global_mut::<Preferences>();
    prefs.system = system;
    let scheme = Some(match system {
        Appearance::Light => Scheme::Light,
        Appearance::Dark => Scheme::Dark,
    });
    // Remembered for the next start; applies the theme when it moved.
    if !update(cx, |s| s.last_os_scheme = scheme) {
        apply_theme(cx);
    }
}

const fn from_scheme(scheme: Scheme) -> Appearance {
    match scheme {
        Scheme::Light => Appearance::Light,
        Scheme::Dark => Appearance::Dark,
    }
}

const fn resolve(mode: AppearanceMode, system: Appearance) -> Appearance {
    match mode {
        AppearanceMode::System => system,
        AppearanceMode::Light => Appearance::Light,
        AppearanceMode::Dark => Appearance::Dark,
    }
}

/// Install the theme the settings ask for, repainting only when it changed.
fn apply_theme(cx: &mut App) {
    let prefs = cx.global::<Preferences>();
    let appearance = resolve(prefs.current.appearance, prefs.system);
    let accent = prefs.current.accent;
    let reduce_motion = prefs.current.reduce_motion;
    // gpui's own flag freezes `with_animation` (spinners) and gpui-base.
    cx.set_reduce_motion(reduce_motion);
    let theme = Theme::of(cx);
    if theme.appearance == appearance
        && theme.accent_family == accent
        && theme.reduce_motion == reduce_motion
    {
        return;
    }
    berth_ui::install_theme(
        Theme::new(appearance, accent).with_reduce_motion(reduce_motion),
        cx,
    );
    cx.refresh_windows();
}

fn schedule_save(cx: &mut App) {
    if cx.global::<Preferences>().path.is_none() {
        return;
    }
    // Replacing the task cancels a pending (not yet running) write.
    let task = cx.spawn(async move |cx| {
        cx.background_executor().timer(SAVE_DEBOUNCE).await;
        let Some((settings, revision, path, writer)) = cx.update(|cx| {
            let prefs = cx.global::<Preferences>();
            prefs.path.clone().map(|path| {
                (
                    prefs.current.clone(),
                    prefs.revision,
                    path,
                    prefs.writer.clone(),
                )
            })
        }) else {
            return;
        };
        let result = cx
            .background_executor()
            .spawn(async move { writer.write(&path, &settings, revision) })
            .await;
        cx.update(|cx| record(result, cx));
    });
    cx.global_mut::<Preferences>().save = Some(task);
}

fn record(result: std::io::Result<()>, cx: &mut App) {
    let prefs = cx.global_mut::<Preferences>();
    let save_error = match result {
        Ok(()) => None,
        Err(error) => {
            let path = prefs.path.as_deref().unwrap_or_else(|| Path::new(""));
            let message = format!("Couldn't save settings to {}: {error}", path.display());
            log::warn!("{message}");
            Some(message.into())
        }
    };
    if prefs.save_error != save_error {
        prefs.save_error = save_error;
        cx.refresh_windows();
    }
}

/// Write pending changes now. Runs at quit; waits for an in-flight write.
fn flush(cx: &mut App) {
    let prefs = cx.global_mut::<Preferences>();
    prefs.save = None;
    let Some(path) = prefs.path.clone() else {
        return;
    };
    if prefs.writer.written() >= prefs.revision {
        return;
    }
    if let Err(error) = prefs.writer.write(&path, &prefs.current, prefs.revision) {
        log::warn!("couldn't save settings: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_mode_follows_the_os() {
        for system in [Appearance::Light, Appearance::Dark] {
            assert_eq!(resolve(AppearanceMode::System, system), system);
            assert_eq!(resolve(AppearanceMode::Light, system), Appearance::Light);
            assert_eq!(resolve(AppearanceMode::Dark, system), Appearance::Dark);
        }
    }

    fn accent_in(path: &Path) -> berth_settings::Accent {
        Settings::load(path).0.accent
    }

    #[test]
    fn writer_never_goes_backwards() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let writer = Writer::default();
        let newer = Settings {
            accent: berth_settings::Accent::Pink,
            ..Settings::default()
        };
        writer.write(&path, &newer, 2).unwrap();
        // A slow, older snapshot arriving late is dropped.
        writer.write(&path, &Settings::default(), 1).unwrap();
        assert_eq!(accent_in(&path), berth_settings::Accent::Pink);
        assert_eq!(writer.written(), 2);
    }

    #[test]
    fn concurrent_writes_serialize() {
        let dir = tempfile::tempdir().unwrap();
        let path = Arc::new(dir.path().join("settings.json"));
        let writer = Arc::new(Writer::default());
        let accents = berth_settings::Accent::ALL;
        let threads: Vec<_> = (1..=40u64)
            .map(|revision| {
                let (path, writer) = (path.clone(), writer.clone());
                let settings = Settings {
                    accent: accents[usize::try_from(revision).unwrap() % accents.len()],
                    ..Settings::default()
                };
                std::thread::spawn(move || writer.write(&path, &settings, revision).unwrap())
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
        assert_eq!(writer.written(), 40);
        assert_eq!(accent_in(&path), accents[40 % accents.len()]);
    }
}
