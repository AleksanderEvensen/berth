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

package:
    NO_STRIP=true cargo packager --release -p berth

# Write the Linux desktop entry from apps/berth/Cargo.toml, so the manifest is
# the one source for the app ID, name, summary and keywords. cargo-packager
# runs this as part of before-packaging-command.
[positional-arguments]
[script("python3")]
desktop-entry out="target/packaging/linux":
    import json, subprocess, sys
    from pathlib import Path

    metadata = json.loads(subprocess.run(
        ["cargo", "metadata", "--locked", "--no-deps", "--format-version", "1"],
        check=True, capture_output=True, text=True,
    ).stdout)

    package = next(p for p in metadata["packages"] if p["name"] == "berth")
    packager = package["metadata"]["packager"]

    # The freedesktop category comes from cargo-packager's category name (fails
    # on one not listed here) and Exec from the binary target's name.
    categories = {"DeveloperTool": "Development"}

    if packager["category"] not in categories:
        sys.exit(f"no freedesktop category for {packager['category']}")

    binary = next(t["name"] for t in package["targets"] if "bin" in t["kind"])

    out = Path(sys.argv[1])
    out.mkdir(parents=True, exist_ok=True)

    (out / f"{packager['identifier']}.desktop").write_text(f"""\
    [Desktop Entry]
    Type=Application
    Name={packager["product-name"]}
    GenericName={package["metadata"]["desktop"]["generic-name"]}
    Comment={package["description"]}
    Exec={binary}
    TryExec={binary}
    Icon={packager["identifier"]}
    Terminal=false
    StartupNotify=true
    StartupWMClass={packager["identifier"]}
    Categories={categories[packager["category"]]};
    Keywords={";".join(package["keywords"])};
    """)
