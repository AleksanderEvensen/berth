# Berth

A native Docker Desktop alternative written in Rust and GPUI

## Development commands

Install [just](https://just.systems/) and run `just` to list the available actions.
Rust commands can still be run directly without just.

```sh
just setup                # fetch locked dependencies
just run                  # run the desktop app
just build                # development build
just release              # release build
just fmt                  # format the workspace
just check                # check all targets without linking
just lint                 # Clippy, with warnings treated as errors
just test                 # workspace tests
just test -p berth-docker # tests for one crate
just test-docker          # ignored integration tests; requires Docker
just verify               # formatting check → check → lint → tests
just ci                   # reproduce CI, including its build step
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
`just ci` runs the commands above in order and stops on the first failure..

## Layout

```
apps/berth      the app: window chrome, shell, screens
crates/ui       design system: theme tokens, Geist fonts, Solar icons, components
crates/docker   engine domain model, container grouping, fixtures
docs/plan       plan documents
```

## Credits

- [Geist](https://vercel.com/font) fonts — SIL OFL 1.1.
- [Solar Icons](https://www.figma.com/community/file/1166831539721848736) by 480 Design — CC BY 4.0.
