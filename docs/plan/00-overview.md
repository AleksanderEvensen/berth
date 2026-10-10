# Berth — Overview

Berth is a native, GPU-rendered desktop app for managing a local container
engine (Docker first, Podman via its Docker-compatible API). It is written in
Rust with [gpui](https://gpui.rs) and targets **Linux first**.

## Why

| Tool | Problem |
| --- | --- |
| Docker Desktop | Slow (Electron + VM), heavy idle footprint, cluttered UX, licensing friction. On Linux it even insists on running its own VM instead of using the native engine. |
| OrbStack | Excellent UX and speed — but macOS only. |
| lazydocker | Fast, but a TUI: no rich inspect views, no multi-select, poor discoverability for infrequent actions (pruning, volume browsing). |

Berth takes **OrbStack's information architecture** (what screens exist and what
you can do on them) and **zeron's visual language** (how it looks and moves),
and runs on top of the engine you already have.

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
6. **Quiet, dense, beautiful.** zeron-style near-black surfaces, hairline
   borders, Geist typography, a single accent hue, restrained motion.

## Scope

### In scope (v1)
- Containers (incl. compose-stack grouping): list, lifecycle, inspect, logs,
  stats, terminal (exec), files.
- Volumes: list (in use / unused), size, delete, bulk delete, browse files.
- Images: list (in use / unused / dangling), inspect, delete, prune.
- Networks: list, inspect, delete, prune.
- Activity monitor: per-container / per-stack CPU, memory, network, disk I/O.
- Clean-up center: one place to prune stopped containers, dangling/unused
  images, unused volumes, unused networks and build cache.
- Settings: engine connection, appearance (light/dark/system, accent), shortcuts.

### Later
- Builds (BuildKit history), Kubernetes (pods/services), remote engines over SSH,
  Podman-native features (pods), tray icon, `berth` CLI helpers.

### Non-goals
- Shipping a container runtime or VM. On macOS/Windows Berth connects to an
  existing engine (Docker Desktop, OrbStack, Colima, Podman machine, Rancher).
- Kubernetes management beyond a read-mostly view.

## Platforms

| Platform | Engine connection | Status |
| --- | --- | --- |
| Linux (Wayland + X11) | `unix:///var/run/docker.sock`, rootless `$XDG_RUNTIME_DIR/docker.sock`, Podman `$XDG_RUNTIME_DIR/podman/podman.sock` | Primary |
| macOS | Docker context socket (Colima/OrbStack/Docker Desktop/Podman machine) | Straightforward — gpui's most mature backend; same client code. |
| Windows | Named pipe `npipe:////./pipe/docker_engine` | Feasible — gpui Windows backend works; bollard supports named pipes. Lowest priority. |

Because Berth is only an API client, cross-platform support costs little beyond
window chrome (traffic lights vs CSD vs Windows caption buttons) and socket
discovery. See [03-architecture.md](03-architecture.md).

## Documents

| Doc | Contents |
| --- | --- |
| [01-user-stories.md](01-user-stories.md) | User stories with acceptance criteria, grouped by area. |
| [02-requirements.md](02-requirements.md) | Functional and non-functional requirements (incl. performance budgets). |
| [03-architecture.md](03-architecture.md) | Crates, engine layer, state/event flow, platform notes. |
| [04-design-system.md](04-design-system.md) | zeron analysis, tokens, GPUI Kit decision, component inventory. |
| [05-screens.md](05-screens.md) | Screen-by-screen layout specs derived from the OrbStack references. |
| [06-roadmap.md](06-roadmap.md) | Milestones. |
