# Requirements

## Product principles

1. **Instant.** Every interaction responds within one frame. Lists are virtualized,
   engine I/O never blocks the UI thread, state updates come from the engine's
   event stream instead of polling.
2. **The common actions are one click.** Start / stop / restart / delete a
   container or a whole compose stack, delete a volume, prune dangling images —
   directly on the row, on hover, without opening anything.
3. **Bulk is first-class.** Multi-select (click, shift-click, ctrl/cmd-click) and
   act on the selection. "Clean up" is a guided action, not a CLI flag hunt.
4. **Safe by default.** Destructive actions confirm once, say exactly what will be
   removed and how much space is reclaimed, and never touch in-use resources
   unless asked.
5. **Use the user's engine.** Berth is a client, not a runtime. It connects to
   `DOCKER_HOST` / Docker contexts / the default socket. No bundled VM on Linux.
6. **Quiet, dense, beautiful.** Near-black surfaces, hairline borders, Geist
   typography, a single accent hue, restrained motion.

### Non-goals
- Shipping a container runtime or VM. On macOS/Windows Berth connects to an
  existing engine (Docker Desktop, OrbStack, Colima, Podman machine, Rancher).
- Kubernetes management beyond a read-mostly view.

## Functional

| ID | Requirement | Stories |
| --- | --- | --- |
| FR-1 | Connect to a Docker-API-compatible engine via unix socket, named pipe, TCP/TLS, or a Docker context. Auto-detect in order: `DOCKER_HOST` → current Docker context → rootful socket → rootless socket → Podman socket. | A5, H1 |
| FR-2 | Subscribe to `/events` and keep an in-memory model of containers, images, volumes and networks consistent with the engine without periodic full refreshes (full resync on reconnect only). | B1, C1, D1 |
| FR-3 | Group containers by `com.docker.compose.project` label; order services by `com.docker.compose.service`. Containers without the label are top-level. | B2 |
| FR-4 | Lifecycle operations: start, stop, restart, kill, pause/unpause, remove (with `force` and `v` options) for containers; the same fan-out for a compose project. | B3 |
| FR-5 | Optimistic UI: rows switch to a pending state immediately, the event stream settles the final state; failures surface as a toast with the engine's error text and the row reverts. | B3 |
| FR-6 | Inspect payloads for container, image, volume and network rendered as structured key/value + table sections. | B5, C4, D4 |
| FR-7 | Log streaming with `follow`, `tail`, `timestamps`; ANSI SGR colours; demultiplex stdout/stderr frames for non-tty containers. | B6 |
| FR-8 | Stats streaming per container; CPU % computed as Docker CLI does (delta cpu / delta system × online CPUs). | B7, F1 |
| FR-9 | Exec with TTY attached to an embedded terminal emulator (alacritty_terminal), resize propagated. | B8 |
| FR-10 | Volume sizes from `/system/df` (`UsageData.Size`); refreshed lazily (on screen open + after prunes), never blocking the list. | C1 |
| FR-11 | Prune endpoints for containers, images (dangling-only / all unused), volumes, networks, build cache, each reporting reclaimed bytes. | D3, G1 |
| FR-12 | Multi-selection model shared by every list: click, shift-click range, ctrl/cmd-click toggle, `ctrl+a`, `Esc` clears. Bulk actions operate on the selection. | B11, C3, D2 |
| FR-13 | Confirmation dialogs for destructive actions list affected objects and reclaimable size; `Enter` confirms, `Esc` cancels. | B3, C2, G1 |
| FR-14 | Settings persisted as JSON under `$XDG_CONFIG_HOME/berth/` (platform equivalents elsewhere). | A2, H1–H3 |

## Non-functional

### Performance budgets
| ID | Budget |
| --- | --- |
| NFR-P1 | Cold start to first painted frame < 300 ms (release build, warm disk). |
| NFR-P2 | Input → visual response ≤ 1 frame (16 ms @60 Hz, 8 ms @120 Hz). No engine call on the UI thread, ever. |
| NFR-P3 | Lists stay at full frame rate with 5 000 containers / 10 000 images (virtualized rows). |
| NFR-P4 | Logs view holds 1 000 000 lines with constant-time scroll (rope/ring buffer + virtualized lines). |
| NFR-P5 | Idle CPU ≈ 0 % when no stats/logs tab is visible (event-driven, no polling timers). Idle RSS < 120 MB. |
| NFR-P6 | Release binary < 40 MB, single file, no runtime dependencies beyond system graphics/Wayland/X11 libs. |

### UX
| ID | Requirement |
| --- | --- |
| NFR-U1 | Every action is reachable by keyboard; focus is always visible. |
| NFR-U2 | Light and dark themes are both designed (not inverted). |
| NFR-U3 | Text contrast ≥ WCAG AA for body copy in both appearances. |
| NFR-U4 | Motion ≤ 200 ms, ease-out; respects reduce-motion. |
| NFR-U5 | Empty, loading, disconnected and error states are designed for every screen. |

### Platform
| ID | Requirement |
| --- | --- |
| NFR-X1 | Linux: Wayland (CSD with own caption buttons, honouring the desktop's button layout) and X11. Ship as AppImage, `.deb` and `.tar.gz`. |
| NFR-X2 | macOS: inset traffic lights, app-owned titlebar drag. Universal binary, notarized `.dmg`. |
| NFR-X3 | Windows: native caption buttons via `WindowControlArea`, portable `.exe`. |

### Engineering
| ID | Requirement |
| --- | --- |
| NFR-E1 | UI crates never depend on the Docker client directly; they talk to an `Engine` trait so the UI can run against fixtures (tests, screenshots, design work). |
| NFR-E2 | `cargo clippy -D warnings` and `cargo test` clean on CI for Linux; macOS/Windows build checks once those targets are enabled. |
| NFR-E3 | Fixture-driven visual tests for each screen (gpui test-support), to catch layout regressions. |
