//! Browsing files in containers and volumes (B9, C5).
//!
//! The engine API can't list a directory, and a container may have no
//! shell or tools to list with. So nothing runs in the browsed container:
//! a short-lived helper container ([`FileMount`]) with known tools
//! (busybox) sees its files, and lists them with [`list_command`]:
//!
//! - a container's, by joining its process namespace and reading them
//!   through `/proc/1/root`, whatever its image;
//! - a volume's, by mounting it read-only.
//!
//! Paths are the browsed tree's own (`/` is its root) and escaped
//! ([`escape_name`]), as names needn't be UTF-8. Symlinks would resolve
//! inside the helper, so they are resolved here instead ([`resolve_path`]),
//! reading their targets only as needed; a listing checks that the folders
//! leading to it are still no links.
//!
//! Files and folders are copied out as a tar archive the helper writes
//! ([`FileMount::copy_command`]), into a [`Staging`] place next to their
//! destination on this machine.

use std::collections::{HashMap, VecDeque};
use std::fmt::{self, Write as _};
use std::io;
use std::path::{Path, PathBuf};

/// Label marking Berth's own helper containers, which the UI hides.
pub const HELPER_LABEL: &str = "no.ahse.berth.helper";

/// The helper's image: small, with `find`, `stat`, `readlink`, `timeout`
/// and `cat`. Pinned, as [`parse_listing`] reads its output.
pub const HELPER_IMAGE: &str = "docker.io/library/busybox:1.37";

/// Where a browsed container's files appear in a helper sharing its
/// process namespace: the root of its process 1.
pub const CONTAINER_ROOT: &str = "/proc/1/root";

/// Where a browsed volume is mounted in its helper.
pub const VOLUME_ROOT: &str = "/volume";

/// A listing taking longer than this (a hung network mount) is stopped.
pub const LIST_TIMEOUT_SECS: u64 = 60;

/// What to browse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilesTarget {
    /// A running container, by ID.
    Container(String),
    /// A volume, by name.
    Volume(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FileKind {
    Directory,
    File,
    Symlink,
    /// Device, socket, pipe.
    Other,
}

impl FileKind {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Directory => "Folder",
            Self::File => "File",
            Self::Symlink => "Link",
            Self::Other => "Special",
        }
    }
}

/// One entry of a directory listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    /// Escaped ([`escape_name`]); see [`display_name`].
    pub name: String,
    pub kind: FileKind,
    /// A symlink's target, escaped, as stored (absolute or relative); read
    /// separately ([`crate::Engine::read_link`]), when it is followed.
    pub link_target: Option<String>,
    /// Bytes.
    pub size: Option<u64>,
    /// Last modification, unix seconds.
    pub modified: Option<i64>,
}

impl FileEntry {
    /// Whether the entry may open as a folder: a directory, or a symlink,
    /// which [`resolve_path`] finds out about.
    #[must_use]
    pub const fn opens(&self) -> bool {
        matches!(self.kind, FileKind::Directory | FileKind::Symlink)
    }
}

/// A listed folder's entries, with their names indexed for
/// [`resolve_path`].
#[derive(Debug, Clone, Default)]
pub struct Folder {
    entries: Vec<FileEntry>,
    index: HashMap<String, usize>,
}

impl Folder {
    #[must_use]
    pub fn new(entries: Vec<FileEntry>) -> Self {
        let index = entries
            .iter()
            .enumerate()
            .map(|(ix, e)| (e.name.clone(), ix))
            .collect();
        Self { entries, index }
    }

    #[must_use]
    pub fn entries(&self) -> &[FileEntry] {
        &self.entries
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<&FileEntry> {
        self.entries.get(*self.index.get(name)?)
    }

    pub fn get_mut(&mut self, name: &str) -> Option<&mut FileEntry> {
        self.entries.get_mut(*self.index.get(name)?)
    }
}

/// Name bytes as text naming them exactly, in `printf` escape syntax: UTF-8
/// as is with `\` doubled, other bytes as three-digit octal `\ooo`.
#[must_use]
pub fn escape_name(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len());
    for chunk in bytes.utf8_chunks() {
        out.push_str(&chunk.valid().replace('\\', "\\\\"));
        for byte in chunk.invalid() {
            let _ = write!(out, "\\{byte:03o}");
        }
    }
    out
}

/// The bytes an escaped name or path stands for.
fn unescape(escaped: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(escaped.len());
    let mut rest = escaped.as_bytes();
    while let Some((&byte, tail)) = rest.split_first() {
        rest = tail;
        if byte != b'\\' {
            out.push(byte);
            continue;
        }
        let octal = rest
            .get(..3)
            .and_then(|digits| std::str::from_utf8(digits).ok())
            .and_then(|digits| u8::from_str_radix(digits, 8).ok());
        match (octal, rest.split_first()) {
            (Some(value), _) => {
                out.push(value);
                rest = rest.get(3..).unwrap_or_default();
            }
            (None, Some((&next, tail))) => {
                out.push(next);
                rest = tail;
            }
            (None, None) => out.push(byte),
        }
    }
    out
}

/// How an escaped name or path reads on one line: bytes that aren't UTF-8
/// show as `�`, control characters (a newline) as escapes.
#[must_use]
pub fn display_name(escaped: &str) -> String {
    String::from_utf8_lossy(&unescape(escaped))
        .chars()
        .map(|c| {
            if c.is_control() {
                c.escape_default().to_string()
            } else {
                c.to_string()
            }
        })
        .collect()
}

/// `path` joined with `name`, both escaped absolute paths.
#[must_use]
pub fn join_path(path: &str, name: &str) -> String {
    if path.ends_with('/') {
        format!("{path}{name}")
    } else {
        format!("{path}/{name}")
    }
}

fn parent_path(path: &str) -> String {
    match path.trim_end_matches('/').rsplit_once('/') {
        Some(("", _)) | None => "/".into(),
        Some((parent, _)) => parent.into(),
    }
}

/// Where a path of the browsed tree leads once its symlinks are followed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    /// This directory (or, from [`resolve_entry`], any entry), with no
    /// symlinks left in its path.
    Real(String),
    /// The listing of this (real) directory is needed to go on.
    Needs(String),
    /// The target of this (real) symlink is needed to go on.
    NeedsLink(String),
    /// Not a folder that can be opened; why, for the user.
    Failed(String),
}

/// Follow the symlinks in `path`, as the kernel would in the browsed
/// container.
///
/// Uses the listings `lookup` knows (by real path). `confined` keeps a
/// volume's links inside it: one leading out points into whichever
/// container uses the volume.
pub fn resolve_path<'a>(
    path: &str,
    confined: bool,
    lookup: impl Fn(&str) -> Option<&'a Folder>,
) -> Resolution {
    resolve(path, confined, false, lookup)
}

/// [`resolve_path`] for any entry: `path` may lead to a file (or a special
/// file) as well as to a folder, e.g. to copy it.
pub fn resolve_entry<'a>(
    path: &str,
    confined: bool,
    lookup: impl Fn(&str) -> Option<&'a Folder>,
) -> Resolution {
    resolve(path, confined, true, lookup)
}

fn resolve<'a>(
    path: &str,
    confined: bool,
    any_entry: bool,
    lookup: impl Fn(&str) -> Option<&'a Folder>,
) -> Resolution {
    const MAX_LINKS: u32 = 40;
    let outside = || Resolution::Failed("Points outside the volume".into());
    let mut pending: VecDeque<String> = path.split('/').map(str::to_owned).collect();
    let mut current = "/".to_owned();
    let mut links: u32 = 0;
    while let Some(component) = pending.pop_front() {
        match component.as_str() {
            "" | "." => continue,
            ".." if current == "/" && confined => return outside(),
            ".." => {
                current = parent_path(&current);
                continue;
            }
            _ => {}
        }
        let Some(folder) = lookup(&current) else {
            return Resolution::Needs(current);
        };
        let child = join_path(&current, &component);
        let Some(entry) = folder.get(&component) else {
            return Resolution::Failed(format!("{} doesn't exist", display_name(&child)));
        };
        match (entry.kind, &entry.link_target) {
            (FileKind::Directory, _) => current = child,
            (FileKind::Symlink, None) => return Resolution::NeedsLink(child),
            (FileKind::Symlink, Some(target)) => {
                links = links.saturating_add(1);
                if links > MAX_LINKS {
                    return Resolution::Failed("Too many levels of links".into());
                }
                if target.starts_with('/') {
                    if confined {
                        return outside();
                    }
                    current = "/".into();
                }
                for part in target.split('/').rev() {
                    pending.push_front(part.to_owned());
                }
            }
            _ if any_entry && pending.is_empty() => current = child,
            _ => {
                return Resolution::Failed(format!("{} isn't a folder", display_name(&child)));
            }
        }
    }
    Resolution::Real(current)
}

/// The command listing `dir` (a path in the helper) without a shell.
///
/// Starting at `dir/.` makes a non-directory an error rather than empty.
/// `checked` (the helper paths of the folders leading to `dir`, and `dir`)
/// are start points too: each reports its own type (`/A<raw mode hex>`,
/// not following links), so a folder replaced by a link since it was
/// resolved is caught rather than followed inside the helper.
#[must_use]
pub fn list_command(dir: &str, checked: &[String]) -> Vec<String> {
    let start = list_start(dir);
    let children = format!("{}/*", glob_escape(&start));
    let mut command: Vec<String> = ["timeout", &LIST_TIMEOUT_SECS.to_string(), "find"]
        .map(str::to_owned)
        .to_vec();
    command.extend(checked.iter().cloned());
    command.push(start.clone());
    // Children of `dir/.`: an entry each. `dir/.` itself: walked into.
    // Anything else (the checked folders): not walked into, its type told.
    // No parentheses: busybox never runs a `-exec … +` inside them.
    command.extend(
        [
            "-maxdepth",
            "1",
            "-path",
            &children,
            "-exec",
            "stat",
            "-c",
            STAT_FORMAT,
            "{}",
            "+",
            "-o",
            "-path",
            &glob_escape(&start),
            "-o",
            "-prune",
            "-exec",
            "stat",
            "-c",
            CHECK_FORMAT,
            "{}",
            "+",
        ]
        .map(str::to_owned),
    );
    command
}

/// One record per entry: `/<raw mode hex>/<size>/<mtime>/<path>`, which
/// `stat` ends with a newline.
const STAT_FORMAT: &str = "/%f/%s/%Y/%n";

/// A checked folder's type: `/A<raw mode hex>`. Raw modes are lower-case
/// hex, so `A` can't start an entry's record.
const CHECK_FORMAT: &str = "/A%f";

fn list_start(dir: &str) -> String {
    format!("{}/.", dir.trim_end_matches('/'))
}

/// `text` matching only itself as a `find -path` pattern.
fn glob_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if matches!(c, '*' | '?' | '[' | ']' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Parse [`list_command`]'s output for `dir`. Directories (and symlinks)
/// come first, then files, each by name.
///
/// Records are delimited exactly, so any name parses: after its metadata
/// and the (known) folder path, a name can't contain `/`, so it ends at the
/// newline before the next record's `/`.
///
/// # Errors
/// A checked folder turned out to be a link, or output that doesn't follow
/// the format; described.
pub fn parse_listing(dir: &str, output: &[u8]) -> Result<Vec<FileEntry>, String> {
    let prefix = format!("{}/", list_start(dir));
    let prefix = prefix.as_bytes();
    let mut entries = Vec::new();
    let mut rest = output;
    while !rest.is_empty() {
        let record = rest.strip_prefix(b"/").ok_or_else(unexpected)?;
        if let Some(check) = record.strip_prefix(b"A") {
            // `A<raw mode hex>\n`: no name, so the first newline ends it.
            let end = check
                .iter()
                .position(|&b| b == b'\n')
                .ok_or_else(unexpected)?;
            let mode = check
                .get(..end)
                .and_then(|m| std::str::from_utf8(m).ok())
                .and_then(|m| u32::from_str_radix(m, 16).ok())
                .ok_or_else(unexpected)?;
            if kind_of(mode) == FileKind::Symlink {
                return Err("This folder changed since it was listed. Reload to see it.".into());
            }
            rest = check.get(end.saturating_add(1)..).unwrap_or_default();
            continue;
        }
        let (entry, tail) = parse_entry(record, prefix).ok_or_else(unexpected)?;
        entries.push(entry);
        rest = tail;
    }
    sort_entries(&mut entries);
    Ok(entries)
}

fn unexpected() -> String {
    "Unexpected listing output".into()
}

const fn kind_of(mode: u32) -> FileKind {
    match mode & 0o170_000 {
        0o040_000 => FileKind::Directory,
        0o100_000 => FileKind::File,
        0o120_000 => FileKind::Symlink,
        _ => FileKind::Other,
    }
}

/// `<raw mode hex>/<size>/<mtime>/<prefix><name>\n`, and what follows it.
fn parse_entry<'a>(record: &'a [u8], prefix: &[u8]) -> Option<(FileEntry, &'a [u8])> {
    let (mode, rest) = field(record)?;
    let (size, rest) = field(rest)?;
    let (modified, rest) = field(rest)?;
    let rest = rest.strip_prefix(prefix)?;
    let end = (0..rest.len()).find(|&at| {
        rest.get(at) == Some(&b'\n')
            && rest
                .get(at.saturating_add(1)..)
                .is_some_and(|tail| tail.is_empty() || tail.starts_with(b"/"))
    })?;
    let name = rest.get(..end)?;
    if name.is_empty() || name.contains(&b'/') {
        return None;
    }
    let entry = FileEntry {
        name: escape_name(name),
        kind: kind_of(u32::from_str_radix(mode, 16).ok()?),
        link_target: None,
        size: size.parse().ok(),
        modified: modified.parse().ok(),
    };
    Some((entry, rest.get(end.saturating_add(1)..)?))
}

/// Fill in the targets of the symlinks in `entries`, the listing of `dir`
/// (a helper path), from [`FileMount::link_targets_command`]'s output.
///
/// busybox prints each as `'<dir>/./<name>' -> '<target>'`, unescaped, so a
/// record is matched against the known link names, and its target ends at
/// the first `'\n` before the next record (only a target containing that
/// and the folder's whole path could be misread). Links left without one
/// are read on their own when followed ([`crate::Engine::read_link`]).
pub fn apply_link_targets(dir: &str, output: &[u8], entries: &mut [FileEntry]) {
    let prefix = format!("'{}/", list_start(dir));
    let prefix = prefix.as_bytes();
    let names: HashMap<Vec<u8>, usize> = entries
        .iter()
        .enumerate()
        .filter(|(_, e)| e.kind == FileKind::Symlink)
        .map(|(ix, e)| (unescape(&e.name), ix))
        .collect();
    let mut rest = output;
    while let Some(record) = rest.strip_prefix(prefix) {
        // The link this record is about: a name can't contain `/`, so its
        // end is an arrow before the first `/`, and exactly one known name
        // may fit.
        let name_area = record
            .iter()
            .position(|&b| b == b'/')
            .unwrap_or(record.len());
        let mut fits = (0..name_area).filter_map(|at| {
            let target = record.get(at..)?.strip_prefix(b"' -> '")?;
            names.get(record.get(..at)?).map(|&ix| (target, ix))
        });
        let (Some((target, ix)), None) = (fits.next(), fits.next()) else {
            return;
        };
        let Some(end) = (0..target.len()).find(|&at| {
            target.get(at..).is_some_and(|t| t.starts_with(b"'\n"))
                && target
                    .get(at.saturating_add(2)..)
                    .is_some_and(|tail| tail.is_empty() || tail.starts_with(prefix))
        }) else {
            return;
        };
        if let (Some(entry), Some(bytes)) = (entries.get_mut(ix), target.get(..end)) {
            entry.link_target = Some(escape_name(bytes));
        }
        rest = target.get(end.saturating_add(2)..).unwrap_or_default();
    }
}

/// A `/`-terminated field.
fn field(bytes: &[u8]) -> Option<(&str, &[u8])> {
    let end = bytes.iter().position(|&b| b == b'/')?;
    let text = std::str::from_utf8(bytes.get(..end)?).ok()?;
    Some((text, bytes.get(end.saturating_add(1)..)?))
}

/// Directories and symlinks first, then by name, case-insensitively.
pub fn sort_entries(entries: &mut [FileEntry]) {
    entries.sort_by(|a, b| {
        b.opens()
            .cmp(&a.opens())
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.name.cmp(&b.name))
    });
}

/// A helper container that sees a browsed container's or volume's files,
/// for [`crate::Engine::list_files`]. Dropping it removes the helper, and
/// with it any listing still running.
pub struct FileMount {
    /// The helper container's ID.
    pub container: String,
    /// Where the browsed files are in the helper.
    pub root: String,
    /// A volume: symlinks may not lead out of it.
    pub confined: bool,
    cleanup: Option<Box<dyn FnOnce() + Send>>,
}

impl FileMount {
    pub fn new(
        container: String,
        root: String,
        confined: bool,
        cleanup: impl FnOnce() + Send + 'static,
    ) -> Self {
        Self {
            container,
            root,
            confined,
            cleanup: Some(Box::new(cleanup)),
        }
    }

    /// Where real path `path` of the browsed tree is in the helper.
    ///
    /// # Errors
    /// The engine API carries only UTF-8, so a path that isn't can't be
    /// listed.
    pub fn helper_path(&self, path: &str) -> Result<String, String> {
        let path = String::from_utf8(unescape(path))
            .map_err(|_| "Folders whose names aren't valid UTF-8 can't be opened".to_owned())?;
        Ok(format!("{}{path}", self.root.trim_end_matches('/')))
    }

    /// The command listing real path `path` (see [`list_command`]).
    ///
    /// # Errors
    /// As [`FileMount::helper_path`].
    pub fn list_command(&self, path: &str) -> Result<Vec<String>, String> {
        Ok(list_command(
            &self.helper_path(path)?,
            &self.on_the_way(path)?,
        ))
    }

    /// The helper paths of the folders leading to real path `path`, and of
    /// `path`.
    fn on_the_way(&self, path: &str) -> Result<Vec<String>, String> {
        let mut checked = Vec::new();
        let mut prefix = String::new();
        for part in path.split('/').filter(|p| !p.is_empty()) {
            prefix = join_path(&prefix, part);
            checked.push(self.helper_path(&prefix)?);
        }
        Ok(checked)
    }

    /// The command telling the types of the folders leading to real path
    /// `path` (`/A<raw mode hex>` each, as [`list_command`]'s checks, for
    /// [`parse_listing`]), so a copy doesn't follow one replaced by a link
    /// since it was resolved. `None` when no folder leads to it.
    ///
    /// # Errors
    /// As [`FileMount::helper_path`].
    pub fn check_command(&self, path: &str) -> Result<Option<Vec<String>>, String> {
        let mut checked = self.on_the_way(path)?;
        checked.pop();
        if checked.is_empty() {
            return Ok(None);
        }
        let mut command: Vec<String> = ["timeout", &LIST_TIMEOUT_SECS.to_string(), "find"]
            .map(str::to_owned)
            .to_vec();
        command.extend(checked);
        command.extend(
            [
                "-maxdepth",
                "0",
                "-exec",
                "stat",
                "-c",
                CHECK_FORMAT,
                "{}",
                "+",
            ]
            .map(str::to_owned),
        );
        Ok(Some(command))
    }

    /// The command reading the targets of the symlinks in the folder at
    /// real path `path`, for [`apply_link_targets`].
    ///
    /// # Errors
    /// As [`FileMount::helper_path`].
    pub fn link_targets_command(&self, path: &str) -> Result<Vec<String>, String> {
        let start = list_start(&self.helper_path(path)?);
        Ok([
            "timeout",
            &LIST_TIMEOUT_SECS.to_string(),
            "find",
            &start,
            "-mindepth",
            "1",
            "-maxdepth",
            "1",
            "-type",
            "l",
            "-exec",
            "stat",
            "-c",
            "%N",
            "{}",
            "+",
        ]
        .map(str::to_owned)
        .to_vec())
    }

    /// The command reading the target of the symlink at real path `path`.
    ///
    /// # Errors
    /// As [`FileMount::helper_path`].
    pub fn read_link_command(&self, path: &str) -> Result<Vec<String>, String> {
        Ok([
            "timeout",
            &LIST_TIMEOUT_SECS.to_string(),
            "readlink",
            "-n",
            &self.helper_path(path)?,
        ]
        .map(str::to_owned)
        .to_vec())
    }

    /// The command writing the file or folder at real path `path` to stdout
    /// as a tar archive, its entries named as [`archive_path`] expects.
    /// Links in a folder are archived as links, not followed.
    ///
    /// # Errors
    /// As [`FileMount::helper_path`].
    pub fn copy_command(&self, path: &str) -> Result<Vec<String>, String> {
        let helper = self.helper_path(path)?;
        // The root (`/proc/1/root` is itself a link) as `.`; anything else
        // as `./<name>` from its folder, so a leading `-` is no option.
        let (dir, name) = if is_root(path) {
            (helper.as_str(), ".".to_owned())
        } else {
            let (dir, name) = helper.rsplit_once('/').unwrap_or(("", &helper));
            (if dir.is_empty() { "/" } else { dir }, format!("./{name}"))
        };
        Ok(["tar", "-c", "-f", "-", "-C", dir, &name]
            .map(str::to_owned)
            .to_vec())
    }
}

fn is_root(path: &str) -> bool {
    path.split('/').all(|part| part.is_empty() || part == ".")
}

/// Where an entry of [`FileMount::copy_command`]'s archive of real path
/// `path` goes, relative to the copy's destination.
///
/// The copied entry is the destination itself. `entry` is the archived
/// name's bytes, split at `/` only, as in the container. `None` for a name
/// that would lead out of the destination, or that this system can't hold
/// as one name (on Windows, one with `\`, `:` and the like).
#[must_use]
pub fn archive_path(path: &str, entry: &[u8]) -> Option<PathBuf> {
    if entry.starts_with(b"/") {
        return None;
    }
    let mut parts = entry
        .split(|&b| b == b'/')
        .filter(|part| !part.is_empty() && *part != b".");
    // All but the root's entries start with the copied entry's name.
    if !is_root(path) && parts.next().is_none_or(|name| name == b"..") {
        return None;
    }
    parts.map(local_name).collect()
}

/// One name of an archived path, as a name here.
#[cfg(unix)]
fn local_name(part: &[u8]) -> Option<&std::ffi::OsStr> {
    use std::os::unix::ffi::OsStrExt as _;
    (part != b"..").then(|| std::ffi::OsStr::from_bytes(part))
}

/// One name of an archived path, as a name here.
#[cfg(not(unix))]
fn local_name(part: &[u8]) -> Option<&std::ffi::OsStr> {
    let name = std::str::from_utf8(part).ok()?;
    (name != ".." && windows_allows(name)).then(|| std::ffi::OsStr::new(name))
}

/// Whether Windows keeps `name` as it is: it drops a trailing dot or
/// space, takes device names for devices whatever their extension, and
/// refuses a few characters.
#[must_use]
pub fn windows_allows(name: &str) -> bool {
    // `COM0`/`LPT0` too, which some versions take for devices.
    const RESERVED: [&str; 32] = [
        "CONIN$",
        "CONOUT$",
        "CON",
        "PRN",
        "AUX",
        "NUL",
        "COM0",
        "COM1",
        "COM2",
        "COM3",
        "COM4",
        "COM5",
        "COM6",
        "COM7",
        "COM8",
        "COM9",
        "COM\u{b9}",
        "COM\u{b2}",
        "COM\u{b3}",
        "LPT0",
        "LPT1",
        "LPT2",
        "LPT3",
        "LPT4",
        "LPT5",
        "LPT6",
        "LPT7",
        "LPT8",
        "LPT9",
        "LPT\u{b9}",
        "LPT\u{b2}",
        "LPT\u{b3}",
    ];
    let stem = name.split('.').next().unwrap_or(name);
    !name.ends_with(['.', ' '])
        && !RESERVED
            .iter()
            .any(|r| r.eq_ignore_ascii_case(stem.trim_end()))
        && !name
            .chars()
            .any(|c| c.is_control() || "<>:\"/\\|?*".contains(c))
}

/// A temporary folder builder whose folders only the user can enter (on
/// Unix), so nobody else reads a copy while it is written, nor an opened
/// copy after.
#[must_use]
pub fn private_dir(prefix: &str) -> tempfile::Builder<'_, '_> {
    let mut builder = tempfile::Builder::new();
    builder.prefix(prefix);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        builder.permissions(std::fs::Permissions::from_mode(0o700));
    }
    builder
}

/// A copy being written in a folder of its own next to its destination.
///
/// It replaces whatever is there only once complete, with a rename.
/// Dropped (failed, cancelled, or finished), the folder goes with what is
/// left in it.
#[derive(Debug)]
pub struct Staging {
    dir: tempfile::TempDir,
    dest: PathBuf,
}

impl Staging {
    /// # Errors
    /// The staging folder can't be made next to `dest`.
    pub fn new(dest: &Path) -> io::Result<Self> {
        let parent = match dest.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => parent,
            _ => Path::new("."),
        };
        Ok(Self {
            dir: private_dir(".berth-").tempdir_in(parent)?,
            dest: dest.to_owned(),
        })
    }

    /// Where the copy is written.
    #[must_use]
    pub fn path(&self) -> PathBuf {
        self.dir.path().join("copy")
    }

    /// Move the copy to its destination. What was there is moved aside
    /// first, and back if the copy can't take its place.
    ///
    /// # Errors
    /// Nothing was written, or a move failed.
    pub fn finish(self) -> io::Result<()> {
        let copy = self.path();
        std::fs::symlink_metadata(&copy)?;
        let old = self.dir.path().join("old");
        let replaced = match std::fs::rename(&self.dest, &old) {
            Ok(()) => true,
            Err(error) if error.kind() == io::ErrorKind::NotFound => false,
            Err(error) => return Err(error),
        };
        if let Err(error) = std::fs::rename(&copy, &self.dest) {
            if replaced && std::fs::rename(&old, &self.dest).is_err() {
                // Something else took its place meanwhile: keep the
                // original where it was moved, and say so.
                let kept = self.dir.keep().join("old");
                return Err(io::Error::new(
                    error.kind(),
                    format!("{error}. What was there is kept at {}", kept.display()),
                ));
            }
            return Err(error);
        }
        Ok(())
    }
}

impl Drop for FileMount {
    fn drop(&mut self) {
        if let Some(cleanup) = self.cleanup.take() {
            cleanup();
        }
    }
}

impl fmt::Debug for FileMount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FileMount")
            .field("container", &self.container)
            .field("root", &self.root)
            .field("confined", &self.confined)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    const DIR: &str = "/proc/1/root/t";
    const P: &str = "/proc/1/root/t/./";

    fn record(mode: &str, size: usize, name: &str) -> String {
        format!("/{mode}/{size}/1700000000/{P}{name}\n")
    }

    #[test]
    fn records_parse_whatever_the_names() {
        let output = [
            record("81a4", 12, "entrypoint.sh"),
            "/A41ed\n".into(),
            record("41ed", 4096, "etc"),
            record("a1ff", 8, "bin"),
            record("a1ff", 0, "a' -> 'b"),
            record("81a4", 0, "nl\nname"),
            record("81a4", 0, "\nlead"),
            "/A41ed\n".into(),
            record("81a4", 0, "trail\n"),
            record("21b6", 0, "null"),
        ]
        .concat();
        let entries = parse_listing(DIR, output.as_bytes()).unwrap();
        let names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "a' -> 'b",
                "bin",
                "etc",
                "\nlead",
                "entrypoint.sh",
                "nl\nname",
                "null",
                "trail\n"
            ]
        );
        let bin = entries.iter().find(|e| e.name == "bin").unwrap();
        assert_eq!(
            (bin.kind, bin.link_target.as_deref()),
            (FileKind::Symlink, None)
        );
        let script = entries.iter().find(|e| e.name == "entrypoint.sh").unwrap();
        assert_eq!((script.kind, script.size), (FileKind::File, Some(12)));
        assert_eq!(script.modified, Some(1_700_000_000));
        assert_eq!(parse_listing(DIR, b"").unwrap(), []);
        assert!(parse_listing(DIR, b"garbage\n").is_err());
    }

    #[test]
    fn folders_with_line_breaks_in_their_paths_parse() {
        let dir = "/volume/trailing\n/x\n";
        let prefix = "/volume/trailing\n/x\n/./";
        let output = format!("/81a4/0/0/{prefix}child\n/A41ed\n/41ed/0/0/{prefix}sub\n\n/A41ed\n");
        let entries = parse_listing(dir, output.as_bytes()).unwrap();
        let names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["sub\n", "child"]);
    }

    #[test]
    fn link_targets_fill_in_where_unambiguous() {
        let link = |name: &str| FileEntry {
            name: name.into(),
            kind: FileKind::Symlink,
            link_target: None,
            size: None,
            modified: None,
        };
        let mut entries = vec![link("arch"), link("ab"), link("a"), link("odd")];
        let output = format!(
            "'{P}arch' -> '/bin/busybox'\n'{P}ab' -> 'x'\n'{P}a' -> 'y\nz'\n'{P}odd' -> 'q'\n"
        );
        apply_link_targets(DIR, output.as_bytes(), &mut entries);
        let targets: Vec<_> = entries.iter().map(|e| e.link_target.as_deref()).collect();
        assert_eq!(
            targets,
            [Some("/bin/busybox"), Some("x"), Some("y\nz"), Some("q")]
        );
        // A record for no known link stops the matching; what's left is
        // read when followed.
        let mut entries = vec![link("a"), link("b")];
        let output = format!("'{P}a' -> 'x'\n'{P}new' -> 'y'\n'{P}b' -> 'z'\n");
        apply_link_targets(DIR, output.as_bytes(), &mut entries);
        let targets: Vec<_> = entries.iter().map(|e| e.link_target.as_deref()).collect();
        assert_eq!(targets, [Some("x"), None]);
    }

    #[test]
    fn many_links_get_targets_and_resolve_quickly() {
        let mut entries: Vec<FileEntry> = (0..50_000)
            .map(|n| FileEntry {
                name: format!("l{n}"),
                kind: FileKind::Symlink,
                link_target: None,
                size: None,
                modified: None,
            })
            .collect();
        let output = (0..50_000).fold(String::new(), |mut out, n| {
            let _ = writeln!(out, "'{P}l{n}' -> 'busybox'");
            out
        });
        let started = std::time::Instant::now();
        apply_link_targets(DIR, output.as_bytes(), &mut entries);
        assert!(
            entries
                .iter()
                .all(|e| e.link_target.as_deref() == Some("busybox"))
        );
        entries.push(FileEntry {
            name: "busybox".into(),
            kind: FileKind::File,
            link_target: None,
            size: None,
            modified: None,
        });
        let root = Folder::new(entries);
        for n in 0..50_000 {
            let resolution = resolve_path(&format!("/l{n}"), false, |dir| {
                (dir == "/").then_some(&root)
            });
            assert!(matches!(resolution, Resolution::Failed(_)));
        }
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
    }

    #[test]
    fn a_folder_turned_link_fails_the_listing() {
        let output = [record("81a4", 0, "x"), "/Aa1ff\n".into()].concat();
        let error = parse_listing(DIR, output.as_bytes()).unwrap_err();
        assert!(error.contains("changed"));
    }

    #[test]
    fn names_that_are_not_utf8_stay_distinct() {
        let mut output = format!("/41ed/0/0/{P}a").into_bytes();
        output.extend_from_slice(b"\x80\n");
        output.extend_from_slice(format!("/41ed/0/0/{P}a").as_bytes());
        output.extend_from_slice(b"\x81\n");
        let entries = parse_listing(DIR, &output).unwrap();
        let names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["a\\200", "a\\201"]);
        assert_eq!(display_name("a\\200"), "a\u{fffd}");
        assert_eq!(display_name("x\\\\y\nz"), "x\\y\\nz");
        assert_eq!(escape_name("ok é".as_bytes()), "ok é");
    }

    #[test]
    fn many_entries_parse_quickly() {
        let output: String = (0..50_000)
            .map(|n| record("a1ff", 1, &format!("l{n}")))
            .collect();
        let started = std::time::Instant::now();
        assert_eq!(parse_listing(DIR, output.as_bytes()).unwrap().len(), 50_000);
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
    }

    #[test]
    fn commands_check_the_way_to_the_folder() {
        let mount = FileMount::new("id".into(), CONTAINER_ROOT.into(), false, || {});
        let command = mount.list_command("/a/b*").unwrap();
        assert_eq!(
            command.get(..6).unwrap(),
            [
                "timeout",
                "60",
                "find",
                "/proc/1/root/a",
                "/proc/1/root/a/b*",
                "/proc/1/root/a/b*/."
            ]
        );
        assert!(command.contains(&"/proc/1/root/a/b\\*/./*".to_owned()));
        let root = mount.list_command("/").unwrap();
        assert_eq!(root.get(3).map(String::as_str), Some("/proc/1/root/."));
        assert_eq!(
            mount.read_link_command("/bin").unwrap(),
            ["timeout", "60", "readlink", "-n", "/proc/1/root/bin"]
        );
    }

    fn tree() -> HashMap<&'static str, Folder> {
        let dir = |name: &str| FileEntry {
            name: name.into(),
            kind: FileKind::Directory,
            link_target: None,
            size: None,
            modified: None,
        };
        let link = |name: &str, target: &str| FileEntry {
            kind: FileKind::Symlink,
            link_target: Some(target.into()),
            ..dir(name)
        };
        let unread = |name: &str| FileEntry {
            kind: FileKind::Symlink,
            ..dir(name)
        };
        let file = |name: &str| FileEntry {
            kind: FileKind::File,
            ..dir(name)
        };
        HashMap::from([
            (
                "/",
                Folder::new(vec![
                    dir("usr"),
                    link("bin", "/usr/bin"),
                    link("lib", "usr/lib"),
                    link("loop", "loop"),
                    link("up", "../.."),
                    link("hosts", "etc-hosts"),
                    unread("unread"),
                    file("etc-hosts"),
                ]),
            ),
            (
                "/usr",
                Folder::new(vec![dir("bin"), dir("lib"), link("lib64", "lib")]),
            ),
            ("/usr/lib", Folder::new(vec![dir("x")])),
        ])
    }

    fn resolve(path: &str, confined: bool) -> Resolution {
        let tree = tree();
        resolve_path(path, confined, |dir| tree.get(dir))
    }

    #[test]
    fn links_resolve_against_the_browsed_root() {
        assert_eq!(resolve("/", false), Resolution::Real("/".into()));
        assert_eq!(resolve("/bin", false), Resolution::Real("/usr/bin".into()));
        assert_eq!(
            resolve("/lib/x", false),
            Resolution::Real("/usr/lib/x".into())
        );
        assert_eq!(
            resolve("/usr/lib64", false),
            Resolution::Real("/usr/lib".into())
        );
        // `..` past the root stays there, as in a container.
        assert_eq!(resolve("/up", false), Resolution::Real("/".into()));
        // What isn't known yet is asked for.
        assert_eq!(
            resolve("/usr/bin/y", false),
            Resolution::Needs("/usr/bin".into())
        );
        assert_eq!(
            resolve("/unread/z", false),
            Resolution::NeedsLink("/unread".into())
        );
        assert!(
            matches!(resolve("/hosts", false), Resolution::Failed(m) if m.contains("isn't a folder"))
        );
        assert!(matches!(resolve("/loop", false), Resolution::Failed(m) if m.contains("Too many")));
        assert!(
            matches!(resolve("/nope", false), Resolution::Failed(m) if m.contains("doesn't exist"))
        );
    }

    #[test]
    fn volume_links_stay_inside_the_volume() {
        let outside = Resolution::Failed("Points outside the volume".into());
        assert_eq!(resolve("/bin", true), outside);
        assert_eq!(resolve("/up", true), outside);
        assert_eq!(
            resolve("/lib/x", true),
            Resolution::Real("/usr/lib/x".into())
        );
    }

    #[test]
    fn helper_paths_need_utf8() {
        let mount = FileMount::new("id".into(), CONTAINER_ROOT.into(), false, || {});
        assert_eq!(mount.helper_path("/").unwrap(), "/proc/1/root/");
        assert_eq!(mount.helper_path("/a\\\\b").unwrap(), "/proc/1/root/a\\b");
        assert!(mount.helper_path("/a\\200").is_err());
    }

    #[test]
    fn links_resolve_to_files_for_copying() {
        let tree = tree();
        let entry = |path| resolve_entry(path, false, |dir| tree.get(dir));
        assert_eq!(entry("/hosts"), Resolution::Real("/etc-hosts".into()));
        assert_eq!(entry("/bin"), Resolution::Real("/usr/bin".into()));
        // Only the last part may be a file.
        assert!(matches!(entry("/hosts/x"), Resolution::Failed(m) if m.contains("isn't a folder")));
        assert!(matches!(entry("/nope"), Resolution::Failed(m) if m.contains("doesn't exist")));
    }

    #[test]
    fn copies_archive_the_entry_from_its_folder() {
        let mount = FileMount::new("id".into(), CONTAINER_ROOT.into(), false, || {});
        assert_eq!(
            mount.copy_command("/etc/-n").unwrap(),
            ["tar", "-c", "-f", "-", "-C", "/proc/1/root/etc", "./-n"]
        );
        assert_eq!(
            mount.copy_command("/").unwrap(),
            ["tar", "-c", "-f", "-", "-C", "/proc/1/root/", "."]
        );
        let volume = FileMount::new("id".into(), VOLUME_ROOT.into(), true, || {});
        assert_eq!(
            volume.copy_command("/x").unwrap(),
            ["tar", "-c", "-f", "-", "-C", "/volume", "./x"]
        );
        assert!(mount.copy_command("/a\\\\200").is_ok());
        assert!(mount.copy_command("/a\\200").is_err());
        assert_eq!(mount.check_command("/x").unwrap(), None);
        assert_eq!(
            mount
                .check_command("/a/b/c")
                .unwrap()
                .unwrap()
                .get(2..5)
                .unwrap(),
            ["find", "/proc/1/root/a", "/proc/1/root/a/b"]
        );
    }

    #[test]
    fn archive_entries_stay_in_the_destination() {
        let at = |path, entry: &str| archive_path(path, entry.as_bytes());
        assert_eq!(at("/etc", "./etc/"), Some(PathBuf::new()));
        assert_eq!(
            at("/etc", "./etc/nginx/x.conf"),
            Some("nginx/x.conf".into())
        );
        assert_eq!(at("/etc/hosts", "hosts"), Some(PathBuf::new()));
        assert_eq!(at("/", "./"), Some(PathBuf::new()));
        assert_eq!(at("/", "./bin/sh"), Some("bin/sh".into()));
        assert_eq!(at("/etc", "./etc/../../x"), None);
        assert_eq!(at("/etc", "/etc/passwd"), None);
        assert_eq!(at("/etc", "../x"), None);
        // Only `/` separates names, as in the container.
        #[cfg(unix)]
        assert_eq!(at("/etc", "./etc/a\\b").unwrap().iter().count(), 1);
        #[cfg(windows)]
        assert_eq!(at("/etc", "./etc/a\\b"), None);
    }

    #[test]
    fn windows_names_are_told_apart() {
        assert!(windows_allows("report.txt"));
        assert!(windows_allows("console.log"));
        for name in [
            "AUX.txt",
            "nul",
            "com1 .tar.gz",
            "COM\u{b9}.txt",
            "lpt\u{b3}",
            "CONIN$",
            "conout$.log",
            "a.",
            "a ",
            "a:b",
            "a\\b",
        ] {
            assert!(!windows_allows(name), "{name}");
        }
    }

    #[test]
    fn staged_copies_replace_their_destination_once_finished() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("out");
        std::fs::create_dir_all(dest.join("old")).unwrap();
        // Copies to the same place don't meet, nor can others look in.
        let staging = Staging::new(&dest).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let parent = staging.path().parent().unwrap().to_owned();
            let mode = std::fs::metadata(parent).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o700);
        }
        let other = Staging::new(&dest).unwrap();
        std::fs::write(staging.path(), "new").unwrap();
        std::fs::write(other.path(), "other").unwrap();
        drop(other);
        staging.finish().unwrap();
        assert_eq!(std::fs::read_to_string(&dest).unwrap(), "new");
        // Unfinished, a copy leaves nothing behind and the destination as it
        // was.
        let staging = Staging::new(&dest).unwrap();
        std::fs::create_dir(staging.path()).unwrap();
        std::fs::write(staging.path().join("part"), "x").unwrap();
        drop(staging);
        let names: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, ["out"]);
        // Nothing written is an error, not an empty copy.
        assert!(Staging::new(&dest).unwrap().finish().is_err());
        assert!(dest.exists());
    }

    #[test]
    fn mount_runs_cleanup_once_on_drop() {
        let (sender, receiver) = std::sync::mpsc::channel();
        let mount = FileMount::new("id".into(), VOLUME_ROOT.into(), true, move || {
            sender.send(()).unwrap();
        });
        drop(mount);
        assert!(receiver.try_recv().is_ok());
        assert!(receiver.try_recv().is_err());
    }
}
