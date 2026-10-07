# Architecture

## Workspace layout

```
apps/berth            binary: window setup, root view, screens, actions/keymap
crates/ui             design system: theme tokens, fonts, icons, styled components
crates/docker         engine domain model + Engine trait + fixtures (+ bollard client in M1)
docs/plan             these documents
reference/            (git-ignored) OrbStack screenshots, zeron source + screenshots
```

Planned additions as the app grows:

```
crates/store          app state: entities for containers/images/…, selection, pending ops
crates/terminal       alacritty_terminal + gpui element (exec tab)
crates/settings       persisted settings (serde JSON, XDG paths)
```

## Dependencies

| Need | Choice | Notes |
| --- | --- | --- |
| UI framework | `gpui-pre 0.3.8` + `gpui-pre-platform` | The crates.io snapshot of Zed's gpui that GPUI Kit is built on. |
| Behavioural components | `gpui-base 0.7.1` (GPUI Kit) | Inputs, focus traps, popovers, virtual lists, scrollbars. Unstyled. |
| Docker API | `bollard` | Async, supports unix socket / named pipe / TLS, streaming logs/stats/events/exec. |
| Async runtime | `tokio` on a background thread, bridged into gpui's executor | bollard needs tokio; gpui has its own executor. Results are sent back to the UI with `cx.spawn` + channels. |
| Terminal | `alacritty_terminal` | Same as zeron/Zed. |
| Fonts | Geist / Geist Mono (OFL) | Bundled, registered at startup. |
| Icons | Solar Icons Linear (CC BY 4.0) + a few hand-drawn in the same weight | Bundled SVGs served by an `AssetSource`. |

## Layers

```
┌──────────────────────── apps/berth ────────────────────────┐
│  Shell (titlebar, sidebar, content outlet)                 │
│  Screens: Containers, Volumes, Images, Networks, Activity  │
│        │ read entities / dispatch actions                  │
│  ┌─────▼──────── store (gpui Entities) ──────────────┐     │
│  │ ContainersModel, ImagesModel, …, Selection, Ops   │     │
│  └─────▲────────────────────────────┬────────────────┘     │
│        │ snapshots + events         │ commands             │
└────────┼────────────────────────────┼──────────────────────┘
         │                            ▼
┌────────┴──────────── crates/docker ──────────────────────┐
│  trait Engine { list_*, inspect_*, start/stop/…, events, │
│                 logs, stats, exec, prune_*, df }         │
│  BollardEngine (tokio thread)      FixtureEngine (tests) │
└──────────────────────────────────────────────────────────┘
```

### Engine trait
- Async methods returning plain domain structs from `berth-docker::model`
  (never bollard types — keeps the UI decoupled and fixtures trivial).
- Streams (`events`, `logs`, `stats`, `exec`) are returned as
  `futures::Stream`s; the store drives them on the background runtime and
  forwards batches to the UI.

### State flow
1. On connect: list everything once → populate entities.
2. Subscribe to `/events`; each event triggers a targeted re-inspect of that one
   object and patches the entity → `cx.notify()` re-renders only observers.
3. User action → mark object `pending(op)` in the store (instant visual feedback)
   → send command to engine → on error revert + toast; on success the event
   stream settles the state.
4. Stats/logs streams exist only while their view is visible (subscription
   lifetime = view lifetime) → zero idle cost.

### Threading
- UI thread: gpui only. Never awaits engine I/O directly.
- Engine thread: a tokio multi-thread runtime owned by `BollardEngine`.
- Bridge: `async_channel`/`futures::channel::mpsc` from tokio tasks to a
  gpui `cx.spawn` loop that applies batched updates (coalesced per frame for
  high-rate streams like logs).

## Window chrome

| Platform | Approach |
| --- | --- |
| Linux | `WindowDecorations::Client`; our own titlebar strip is the drag region (`start_window_move`, double-click → `zoom_window`); caption buttons drawn by us following `cx.button_layout()` when the compositor reports CSD; resize edges via `start_window_resize`. |
| macOS | `TitlebarOptions { appears_transparent: true, traffic_light_position }`, `app_owns_titlebar_drag: true`; sidebar starts under the traffic lights. |
| Windows | `appears_transparent` + caption buttons rendered with `WindowControlArea::{Min,Max,Close}` so Snap Layouts work. |

This is lifted from how zeron does it (`crates/ui/src/lib.rs`, `shell.rs`
`titlebar_drag_region`, `render_linux_caption_controls`).

## Engine discovery (FR-1)

1. `DOCKER_HOST` env var.
2. `docker context inspect` equivalent: read `~/.docker/config.json`
   `currentContext` → `~/.docker/contexts/meta/<sha256(name)>/meta.json`.
3. `/var/run/docker.sock`.
4. `$XDG_RUNTIME_DIR/docker.sock` (rootless Docker).
5. `$XDG_RUNTIME_DIR/podman/podman.sock` (Podman API service).

On failure, the UI shows the disconnected state with the specific reason
(missing socket vs EACCES) and remediation text.

TLS endpoints use Bollard's own rustls transport (`connect_with_ssl`): mutual
TLS with `cert.pem`/`key.pem`, the daemon verified against `ca.pem`. Docker
contexts with `SkipTLSVerify` are rejected with an explanatory error, since
Bollard has no unverified mode and we don't maintain our own TLS code.

## Testing
- `berth-docker` fixtures power both unit tests of grouping/sorting logic and
  gpui visual tests of each screen.
- An integration test suite (opt-in, needs a real engine) spins up a throwaway
  compose project and exercises lifecycle + prune through `BollardEngine`.
