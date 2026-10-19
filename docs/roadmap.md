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

- [ ] macOS release: the `.dmg` is built, signed and notarized when the
      `APPLE_*` variables are set. Missing: the Developer ID secrets in the
      repository and a release that ships one.
- [ ] B9 — Container Files tab. Browsing is in for any running container.
      Stopped containers can't be browsed, nor ones sharing another's process
      namespace (`--pid=host`, `--pid=container:…`). Missing: opening and
      downloading files (below).
- [ ] Files: open a file with double-click / Enter in the program the OS
      picks (copy it out of the container to a temp file first), and
      download files and folders. `FileTree::on_open` already fires; nothing
      handles it yet.
- [ ] B12 — Run-container form.
- [ ] H3 — Rebind keyboard shortcuts.
- [ ] A3 — Command palette (`ctrl+k`): run any action, jump to any container/image/volume.
- [ ] Builds.
- [ ] Kubernetes.
- [x] H4 — SSH remotes. `ssh://` endpoints connect through the system
      `ssh` with `BatchMode=yes`; checked against a real remote with
      `BERTH_TEST_SSH_HOST`. Later: password, passphrase and host-key
      prompts in the app (`SSH_ASKPASS`), an ssh-config host picker, and a
      pure-Rust (russh) transport so no `ssh` client is needed.

## Open questions

- Build `Button`/`IconButton` and `Dialog` on `gpui-base`'s unstyled
  primitives instead of plain `div`s, keeping our styling? The gain would be
  keyboard focus and accessibility behaviour we don't have to maintain;
  `Dialog` focus trapping would also need `gpui-base`'s Root integration. Not
  a code-size win.
