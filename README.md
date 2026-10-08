# Berth

A native Docker Desktop alternative written in Rust and GPUI — OrbStack's
layout, zeron's look, Linux first.

> Status: **M0 visual prototype** — the Containers screen runs on fixture data.
> See [docs/plan](docs/plan/00-overview.md) for the plan, user stories and roadmap.

## Development commands

Install [just](https://just.systems/) and run `just` to list the available actions.
Rust commands can still be run directly without just.

```sh
just setup              # fetch locked dependencies
just run                # run the desktop app
just build              # development build
just release            # release build
just fmt                # format the workspace
just check              # check all targets without linking
just lint               # Clippy, with warnings treated as errors
just test               # workspace tests
just test -p berth-docker # tests for one crate
just test-docker        # ignored integration tests; requires Docker
just verify             # formatting check → check → lint → tests
just ci                 # reproduce CI, including its build step
```

## Run

```sh
just run
# Or: cargo run --locked -p berth
```

Linux needs the usual gpui system libraries (Wayland/X11, xkbcommon, fontconfig,
Vulkan driver). `ctrl+b` toggles the sidebar, `ctrl+q` quits.

## CI

GitHub Actions runs on pull requests and pushes to `main`. Formatting gates the
slower checks; Clippy, building all targets, and tests then run in order on one
Linux runner, sharing build artifacts and a dependency cache. New commits cancel
superseded runs.

To run the same checks locally:

```sh
cargo +stable fmt --all -- --check
cargo +stable clippy --locked --workspace --all-targets --profile ci -- -D warnings
cargo +stable build --locked --workspace --all-targets --profile ci
cargo +stable test --locked --workspace --profile ci
```

The `ci` profile disables dependency optimization and debug information to reduce
compilation time; normal development and release profiles retain their settings.
`just ci` runs the commands above in order and stops on the first failure.

## T3 Code worktrees and compilation caches

`t3.json` runs `just setup` when a worktree is created. This fetches dependencies;
it does not compile or copy `target/`. Cargo already shares downloaded registry
and Git dependencies through `CARGO_HOME` across worktrees.

[T3 Code's own setup](https://github.com/pingdotgg/t3code/blob/main/scripts/setup-worktree.ts)
installs dependencies, links ignored `.env` files from `T3CODE_PROJECT_ROOT`, and
warms its Vite dependency cache. The same hook could copy Berth's build cache
from `$T3CODE_PROJECT_ROOT/target`, but Vite's cache is not Rust's incremental
compilation cache.

**Copying `target/` can help, but is not reliable incremental-cache relocation.**
Unchanged registry dependencies may be reused when the compiler, target, profile,
features, and flags match. Workspace sources have different absolute paths and
fresh checkout timestamps; dep-info, build-script output, and compiler state can
refer to the original checkout. Expect local crates and some dependencies to
rebuild. Changing paths in `.d` files alone does not make the cache portable.
See [Cargo's build-cache documentation](https://doc.rust-lang.org/cargo/reference/build-cache.html).

Automatic copying is deliberately disabled: the main checkout's `target/` was
about 61 GB when inspected. A normal copy multiplies disk use; reflink copies can
reduce that cost on supported filesystems, but do not solve cache invalidation.
If experimenting, seed only a new, empty worktree's target directory, preserve
timestamps, and wait until the source checkout has no active Cargo commands.
Keep each worktree's subsequent build output independent: do not symlink or
hard-link `target/`, or set all worktrees to one shared `CARGO_TARGET_DIR`.
No compile-time benchmark was run for this investigation.

For a supported cross-workspace dependency cache, consider installing
[sccache](https://github.com/mozilla/sccache/blob/main/docs/Rust.md), then opt in:

```sh
CARGO_INCREMENTAL=0 RUSTC_WRAPPER=sccache just ci
```

sccache requires incremental compilation to be disabled and does not cache
linker-producing crates such as binaries or proc macros. It complements separate
worktree target directories; it does not replace them. For repeated edits within
one worktree, normal `just build` retains Cargo's development incremental cache.

## Layout

```
apps/berth      the app: window chrome, shell, screens
crates/ui       design system: theme tokens, Geist fonts, Solar icons, components
crates/docker   engine domain model, container grouping, fixtures
docs/plan       plan documents
```

## Credits

- Theme token roles and titlebar handling follow [zeron](https://github.com/zeronsh) (MIT).
- [Geist](https://vercel.com/font) fonts — SIL OFL 1.1.
- [Solar Icons](https://www.figma.com/community/file/1166831539721848736) by 480 Design — CC BY 4.0.
