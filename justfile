# List available actions when invoked without a recipe.
default:
    @just --list

# Fetch locked dependencies (also used when T3 Code creates a worktree).
setup:
    cargo fetch --locked

# Run the desktop app; forward options to the application.
run *args:
    cargo run --locked -p berth -- {{ args }}

# Build the desktop app for development.
build:
    cargo build --locked -p berth

# Build the desktop app for release.
release:
    cargo build --locked --release -p berth

# Check every workspace target without linking.
check:
    cargo check --locked --workspace --all-targets

# Format the workspace.
fmt:
    cargo fmt --all

# Check formatting without changing files.
fmt-check:
    cargo +stable fmt --all -- --check

# Lint every workspace target, treating warnings as errors.
lint:
    cargo clippy --locked --workspace --all-targets -- -D warnings

# Test the workspace; forward Cargo options, e.g. just test -p berth-docker.
test *args:
    cargo test --locked --workspace {{ args }}

# Run ignored Docker integration tests (requires Docker; creates disposable resources).
test-docker:
    cargo test --locked -p berth-docker -- --ignored

# Run development checks in order, stopping on the first failure.
verify: fmt-check check lint test

# Run the same ordered checks as GitHub Actions, sharing the ci profile artifacts.
ci: fmt-check lint-ci build-ci test-ci

# Lint using the faster CI profile.
lint-ci:
    cargo +stable clippy --locked --workspace --all-targets --profile ci -- -D warnings

# Build all targets using the faster CI profile.
build-ci:
    cargo +stable build --locked --workspace --all-targets --profile ci

# Test using the faster CI profile.
test-ci:
    cargo +stable test --locked --workspace --profile ci
