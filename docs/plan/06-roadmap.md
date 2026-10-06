# Roadmap

## M0 — Visual prototype ✅ (this commit)
- Workspace: `apps/berth`, `crates/ui`, `crates/docker`.
- Theme tokens ported from zeron (dark), Geist fonts, Solar icons.
- Collapsible sidebar component; Containers screen (list + Info tab) on fixture data.
- Linux CSD titlebar: drag, double-click maximize, caption buttons.

## M1 — Real engine, containers (P0)
- `Engine` trait + `BollardEngine`; discovery (FR-1); disconnected state.
- Event-driven store (FR-2), optimistic lifecycle actions (FR-4/5).
- Container Info on real inspect data; open-port links; confirm dialog; toasts.
- Search input (gpui-base) on the list column.
- Stories: A1, A2, A4, A5, B1–B5, B10.

## M2 — Volumes, images, cleanup (P0)
- Volumes + Images screens, multi-selection model (FR-12), bulk delete.
- `/system/df` sizes; Clean-up page with prunes (G1).
- Stories: C1–C3, D1–D3, B11, G1.

## M3 — Live views (P1)
- Logs (virtualized, ANSI), Stats sparklines, Terminal (exec), Activity monitor.
- Stories: B6–B8, F1.

## M4 — Polish & ship Linux (P1)
- Light theme, accent picker, settings persistence, command palette, keyboard
  map, networks screen, reduce-motion.
- Packaging: AppImage, Flatpak, AUR, .deb/.rpm.
- Stories: A3, E1, H1, H2, C4, D4, D5.

## M5 — macOS / Windows
- macOS: traffic-light inset chrome, context-socket discovery, signed `.dmg`.
- Windows: named pipe, caption buttons, MSI.

## Later
- Files tabs (B9, C5), run-container form (B12), builds, Kubernetes, SSH remotes.
