# Roadmap

Progress is tracked as checklists. Tick an item only when it is on `main`
and meets the acceptance criteria of its story in
[01-user-stories.md](01-user-stories.md). Partially done items stay unticked
with a note saying what is missing.

**Status (audited 2026-10-09, at `ce159f2`):** M0–M4 done; M4 ended with Linux packaging
(AppImage, .deb, .tar.gz). M5 endpoint discovery (macOS context sockets,
Windows named pipe) is in; the rest of M5 is not.

## M0 — Visual prototype ✅

- [x] Workspace: `apps/berth`, `crates/ui`, `crates/docker`.
- [x] Theme tokens ported from zeron (dark), Geist fonts, Solar icons.
- [x] Collapsible sidebar component; Containers screen (list + Info tab) on fixture data.
- [x] Linux CSD titlebar: drag, double-click maximize, caption buttons.

## M1 — Real engine, containers (P0) ✅

- [x] `Engine` trait + `BollardEngine`; discovery (FR-1); disconnected state.
- [x] Event-driven store (FR-2), optimistic lifecycle actions (FR-4/5).
- [x] Container Info on real inspect data; open-port links; confirm dialog; toasts.
- [x] Search input (gpui-base) on the list column.
- Stories:
  - [x] A1 — Sidebar with grouped resource areas, active/hover washes, counts.
  - [x] A2 — Collapsible sidebar (`ctrl+b`/`cmd+b`), animated, persisted.
  - [x] A4 — Linux CSD window chrome.
  - [x] A5 — Engine status in the sidebar footer. On permission denied or
        no engine found, the footer and a "How to fix" button on the
        Containers list open the fix-it guide on Settings → Engine: rootless Docker,
        rootless Podman, then the `docker` group flagged root-equivalent, with
        copyable commands.
  - [x] B1 — Running / Stopped sections with counts.
  - [x] B2 — Compose projects as expandable stacks.
  - [x] B3 — Start/stop/restart/delete for containers and stacks, with confirm and pending spinners.
  - [x] B4 — Open published port; with several web ports the row button
        opens a menu listing them all.
  - [x] B5 — Info tab: identity, command, network, ports (host ports open in
        the browser), mounts (named volumes open on the Volumes screen),
        environment (from inspect), labels; copy buttons for ID and command.
  - [x] B10 — Search/filter the container list.

## M2 — Volumes, images, cleanup (P0) ✅

- [x] Volumes + Images screens, multi-selection model (FR-12), bulk delete.
- [x] `/system/df` sizes; Clean-up page with prunes (G1).
- [x] FR-12 keyboard parts: `ctrl+a` selects every listed row, `Esc` clears
      (not while typing in an input or the terminal).
- Stories:
  - [x] C1 — Volumes in "In use" / "Unused" with sizes and total.
  - [x] C2 — Delete a volume; in-use volumes guarded.
  - [x] C3 — Multi-select volumes, "N selected · X GB", bulk delete.
  - [x] D1 — Images in "In use" / "Unused" / "Dangling" with size, age, total.
  - [x] D2 — Delete image from row; multi-select delete.
  - [x] D3 — Remove dangling / unused images with reclaimable preview.
  - [x] B11 — Multi-select containers and stacks; the detail pane shows the
        count with bulk start, stop and delete.
  - [x] G1 — Clean-up page: per-category reclaimable space, pick, prune, report.

## M3 — Live views (P1) ✅

- [x] Logs (virtualized, ANSI), Stats sparklines, Terminal (exec), Activity monitor.
- Stories:
  - [x] B6 — Logs: follow, filter, timestamps, copy, clear, ANSI (wrap toggle deferred to M4).
  - [x] B7 — Stats: CPU, memory, network, block I/O sparklines.
  - [x] B8 — Terminal: interactive exec with tty.
  - [x] F1 — Activity monitor tree table, sortable, totals and sparklines.

## M4 — Polish & ship Linux (P1) ✅

- [x] Light theme.
- [x] Accent picker.
- [x] Settings persistence (FR-14, `$XDG_CONFIG_HOME/berth/`).
- [x] Reduce-motion.
- [x] Keyboard map — every action reachable by keyboard (NFR-U1); a shortcut
      reference on Settings → Shortcuts (`ctrl+/`), drawn from the same table
      that binds the keys. `ctrl+1`–`6` switch screens, arrows/Home/End move
      the list selection (shift extends), Delete deletes it, `ctrl+enter` /
      `ctrl+r` / `ctrl+o` start-stop / restart / open a port, ←/→ fold stacks,
      `alt+1`–`5` switch container tabs (also out of the terminal),
      `ctrl+↑/↓` move a cursor and `ctrl+space` toggles the row under it.
      Page Up/Down and `ctrl+home`/`end` scroll the content pane (in Logs,
      scrolling up pauses following); on screens without a list the
      arrows, Home and End scroll it. Every other control is a tab stop with a
      visible focus ring, scrolled into view when focused; row hover buttons
      are not, since list keys cover them. Tab stays inside an open confirm
      dialog, and focus returns to the window when its control disappears.
- [x] Networks screen.
- Deferred from M3:
  - [x] Logs wrap toggle.
- Packaging (NFR-X1): one app ID, `no.ahse.berth`, for the
  desktop entry, icon and the window's `app_id`. Packages ship only as
  GitHub release assets: merging the release-plz release PR tags `v*` and
  attaches them to the GitHub release.
  - [x] AppImage (cargo-packager).
  - [x] .deb (cargo-packager).
  - [x] .tar.gz of the installed tree (cargo-packager's pacman format).
- Stories:
  - [x] E1 — Networks: list (driver, scope, subnet, attached containers), delete, prune unused.
        Swarm networks count as possibly in use (services on other nodes have no
        local endpoints), so Berth doesn't delete them.
  - [x] H1 — Engine endpoint: auto-detect, Docker context, custom socket/URL; active one shown.
  - [x] H2 — Appearance: System / Light / Dark, accent colour.
  - [x] C4 — Volume info: driver, mountpoint, created, labels, compose
        project; used-by containers open on the Containers screen.
  - [x] D4 — Image info: ID, tags, created, size, platform, command,
        entrypoint, workdir, env, volumes, exposed ports (`inspect_image`),
        clickable used-by, labels.

## M5 — macOS / Windows

- [x] macOS: traffic-light inset chrome. The traffic lights sit centred in our
      header and the titlebar clears them; the app owns titlebar dragging
      (`app_owns_titlebar_drag`), so AppKit neither drags from its own strip
      over our buttons nor delays clicks there, and a double-click does what
      System Settings → Desktop & Dock says.
- [x] macOS: context-socket discovery (Docker contexts, then Docker Desktop /
      OrbStack / Colima sockets).
- [x] macOS: signed, notarized universal `.dmg`. `just package` builds the
      universal app and `.dmg`, signing and notarizing when the `APPLE_*`
      variables are set, and the release workflow attaches it. Missing: the
      Developer ID secrets in the repository and a release that ships one.
- [x] Windows: named pipe (`npipe://`, default `//./pipe/docker_engine`).
- [ ] Windows: caption buttons.
- [ ] Windows: MSI.

## Later (P2)

- [ ] B9 — Container Files tab.
- [ ] C5 — Browse volume files.
- [ ] B12 — Run-container form.
- [ ] H3 — Rebind keyboard shortcuts.
- [ ] A3 — Command palette (`ctrl+k`): run any action, jump to any container/image/volume.
- [ ] Builds.
- [ ] Kubernetes.
- [ ] SSH remotes.
