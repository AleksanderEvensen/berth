# Roadmap

Progress is tracked as checklists. Tick an item only when it is on `main`
and meets the acceptance criteria of its story in
[01-user-stories.md](01-user-stories.md). Partially done items stay unticked
with a note saying what is missing.

**Status (audited 2026-10-08, at `8bdee48`):** M0–M3 done apart from the
gaps listed under them. M4 has started: settings screen, light theme, accent
picker, reduce motion, persistence and the Networks screen are in.

## M0 — Visual prototype ✅
- [x] Workspace: `apps/berth`, `crates/ui`, `crates/docker`.
- [x] Theme tokens ported from zeron (dark), Geist fonts, Solar icons.
- [x] Collapsible sidebar component; Containers screen (list + Info tab) on fixture data.
- [x] Linux CSD titlebar: drag, double-click maximize, caption buttons.

## M1 — Real engine, containers (P0)
- [x] `Engine` trait + `BollardEngine`; discovery (FR-1); disconnected state.
- [x] Event-driven store (FR-2), optimistic lifecycle actions (FR-4/5).
- [x] Container Info on real inspect data; open-port links; confirm dialog; toasts.
- [x] Search input (gpui-base) on the list column.
- Stories:
  - [x] A1 — Sidebar with grouped resource areas, active/hover washes, counts.
  - [x] A2 — Collapsible sidebar (`ctrl+b`/`cmd+b`), animated, persisted.
  - [x] A4 — Linux CSD window chrome.
  - [ ] A5 — Engine status in the sidebar footer. *Missing:* fix-it hint for
        permission denied ("add yourself to the docker group"); the error only
        says `permission denied on <endpoint>`.
  - [x] B1 — Running / Stopped sections with counts.
  - [x] B2 — Compose projects as expandable stacks.
  - [x] B3 — Start/stop/restart/delete for containers and stacks, with confirm and pending spinners.
  - [ ] B4 — Open published port. *Missing:* menu listing all ports when there
        are several (the row button opens the first web port only).
  - [ ] B5 — Info tab. *Missing:* env and command sections; port and
        named-volume cells are styled as links but not clickable (volumes
        should navigate to the Volumes screen); copy button for IDs.
  - [x] B10 — Search/filter the container list.

## M2 — Volumes, images, cleanup (P0)
- [x] Volumes + Images screens, multi-selection model (FR-12), bulk delete.
- [x] `/system/df` sizes; Clean-up page with prunes (G1).
- [ ] FR-12 keyboard parts: `ctrl+a` select all and `Esc` to clear are not bound.
- Stories:
  - [x] C1 — Volumes in "In use" / "Unused" with sizes and total.
  - [x] C2 — Delete a volume; in-use volumes guarded.
  - [x] C3 — Multi-select volumes, "N selected · X GB", bulk delete.
  - [x] D1 — Images in "In use" / "Unused" / "Dangling" with size, age, total.
  - [x] D2 — Delete image from row; multi-select delete.
  - [x] D3 — Remove dangling / unused images with reclaimable preview.
  - [ ] B11 — Multi-select containers. *Missing:* the Containers screen still
        uses single selection; `MultiSelect` is only wired into Volumes and Images.
  - [x] G1 — Clean-up page: per-category reclaimable space, pick, prune, report.

## M3 — Live views (P1) ✅
- [x] Logs (virtualized, ANSI), Stats sparklines, Terminal (exec), Activity monitor.
- Stories:
  - [x] B6 — Logs: follow, filter, timestamps, copy, clear, ANSI (wrap toggle deferred to M4).
  - [x] B7 — Stats: CPU, memory, network, block I/O sparklines.
  - [x] B8 — Terminal: interactive exec with tty.
  - [x] F1 — Activity monitor tree table, sortable, totals and sparklines (engine row deferred to M4).

## M4 — Polish & ship Linux (P1)
- [x] Light theme.
- [x] Accent picker.
- [x] Settings persistence (FR-14, `$XDG_CONFIG_HOME/berth/`).
- [x] Reduce-motion.
- [ ] Command palette.
- [ ] Keyboard map — every action reachable by keyboard (NFR-U1); a shortcut
      reference. Bound today: sidebar, search, settings, focus cycling, quit,
      dialog confirm/cancel.
- [x] Networks screen.
- Deferred from M3:
  - [ ] Activity monitor "Engine" row (daemon usage isn't in the Docker API —
        read `/proc` for `dockerd` on Linux).
  - [ ] Logs wrap toggle.
- Packaging:
  - [ ] AppImage
  - [ ] Flatpak
  - [ ] AUR
  - [ ] .deb
  - [ ] .rpm
- Stories:
  - [ ] A3 — Command palette (`ctrl+k`): run any action, jump to any container/image/volume.
  - [x] E1 — Networks: list (driver, scope, subnet, attached containers), delete, prune unused.
        Swarm networks count as possibly in use (services on other nodes have no
        local endpoints), so Berth doesn't delete them.
  - [x] H1 — Engine endpoint: auto-detect, Docker context, custom socket/URL; active one shown.
  - [x] H2 — Appearance: System / Light / Dark, accent colour.
  - [ ] C4 — Volume info. *Done:* driver, mountpoint, created, labels, compose
        project, used-by list. *Missing:* used-by containers aren't clickable.
  - [ ] D4 — Image info. *Done:* ID, tags, created, size, used-by, labels.
        *Missing:* platform, command, entrypoint, workdir, env, volumes,
        exposed ports (needs an `inspect_image` engine call); clickable used-by.
  - [ ] D5 — Pull an image by reference with progress (no engine call yet).

## M5 — macOS / Windows
- [ ] macOS: traffic-light inset chrome (titlebar already clears the traffic lights).
- [ ] macOS: context-socket discovery.
- [ ] macOS: signed, notarized universal `.dmg`.
- [ ] Windows: named pipe.
- [ ] Windows: caption buttons.
- [ ] Windows: MSI.

## Later (P2)
- [ ] B9 — Container Files tab.
- [ ] C5 — Browse volume files.
- [ ] B12 — Run-container form.
- [ ] H3 — Rebind keyboard shortcuts.
- [ ] Builds.
- [ ] Kubernetes.
- [ ] SSH remotes.
