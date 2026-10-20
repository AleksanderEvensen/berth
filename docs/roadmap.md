# Roadmap

Progress is tracked as checklists. Tick an item only when it is on `main`
and meets the acceptance criteria of its story in
[user-stories.md](user-stories.md). Partially done items stay unticked
with a note saying what is missing.

## Done

- **M0 — Visual prototype**: workspace, theme, sidebar, Linux CSD titlebar.
- **M1 — Real engine, containers**: A1, A2, A4, A5, B1–B5, B10.
- **M2 — Volumes, images, cleanup**: B11, C1–C3, D1–D3, G1.
- **M3 — Live views**: B6, B7, B8, F1.
- **M4 — Polish & ship Linux**: light theme, accent picker, settings
  persistence, reduce-motion, keyboard map, Networks screen, AppImage /
  `.deb` / `.tar.gz`; C4, D4, E1, H1, H2.
- **M5 — macOS / Windows**: traffic-light chrome, context-socket discovery,
  universal `.dmg`; named pipe, caption buttons, portable `.exe`.
- **C5** — Browse volume files.

## Open

- [x] macOS release: the `.dmg` is built, signed and notarized; v0.1.0
      ships `Berth_0.1.0_universal.dmg`.
- [ ] B9 — Container Files tab. Browsing, opening and downloading are in
      for any running container. Missing: stopped containers can't be
      browsed, nor ones sharing another's process namespace (`--pid=host`,
      `--pid=container:…`).
- [x] Files: open a file with double-click / Enter (or Open) in the program
      the OS picks, from a copy in a temporary folder, and download files and
      folders (Download, with a save dialog), in container and volume Files
      tabs. Progress and errors show below the tree. Copies go through the
      helper's `tar`, so they work over SSH too (not yet checked against a
      real remote). Later: links inside a downloaded folder are left out on
      Windows; pseudo-files that report no size (`/proc`, `/sys`) copy
      empty; opened copies stay in the temporary folder, and edits to them
      don't go back into the container.
- [ ] B12 — Run-container form.
- [ ] H3 — Rebind keyboard shortcuts.
- [x] A3 — Command palette (`ctrl+k` / `cmd+k`): runs any command in the
      keyboard map and jumps to any container, image or volume by name.
- [ ] Builds.
- [ ] Kubernetes.
- [x] H4 — SSH remotes. `ssh://` endpoints connect through the system
      `ssh` with `BatchMode=yes`; checked against a real remote with
      `BERTH_TEST_SSH_HOST`. Later: password, passphrase and host-key
      prompts in the app (`SSH_ASKPASS`), an ssh-config host picker, and a
      pure-Rust (russh) transport so no `ssh` client is needed.

## Open questions

- Build `Button`/`IconButton` and `Dialog` on `gpui-base`'s unstyled
  primitives instead of plain `div`s, keeping our styling? Tried on
  `feat/gpui-base-primitives`: buttons, the confirm dialog (on `Root` and
  `AlertDialog`) and the toasts (`ToastManager`/`ToastStack`) are rebuilt,
  looking as before. Gained: every button and toast has a screen-reader role
  and label, the dialog is an alert dialog, and Tab, Enter and Esc in modals
  are gpui-base's, so berth's own Tab cycling, dialog actions and toast
  timers are gone. Cost: about the same production code (+429/−287 lines),
  a `NoAction` binding so Enter on a focused dialog button doesn't confirm,
  `#[doc(hidden)]` setters for the dialog's focus, a viewport-wide dialog
  host that has to be inset for the CSD frame, and a toast layout tuned
  through its collapse parameters. Open: keep it or not; the PR has the
  details.
