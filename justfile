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

# Build the release packages into target/packager: deb, AppImage and tar.gz.
[linux]
package:
    NO_STRIP=true cargo packager --release -p berth

# Build a universal Berth.app and Berth_<version>_universal.dmg into
# target/packager. With APPLE_SIGNING_IDENTITY set (a "Developer ID
# Application" identity in the keychain) both are signed, the app with the
# hardened runtime; with APPLE_ID, APPLE_PASSWORD (an app-specific password)
# and APPLE_TEAM_ID too, the .dmg is notarized and its ticket stapled.
[macos]
[script("bash")]
package:
    set -euo pipefail
    cargo packager --release -p berth --target universal-apple-darwin --formats app

    app=target/packager/Berth.app
    version=$(/usr/libexec/PlistBuddy -c "Print CFBundleShortVersionString" "$app/Contents/Info.plist")
    dmg="target/packager/Berth_${version}_universal.dmg"
    identity=${APPLE_SIGNING_IDENTITY:-}

    if [[ -n $identity ]]; then
        codesign --force --options runtime --timestamp --sign "$identity" "$app"
        codesign --verify --strict --verbose=2 "$app"
    else
        echo "APPLE_SIGNING_IDENTITY is not set: the app and .dmg are unsigned" >&2
    fi

    stage=$(mktemp -d)
    trap 'rm -rf "$stage"' EXIT
    ditto "$app" "$stage/Berth.app"
    ln -s /Applications "$stage/Applications"
    rm -f "$dmg"
    hdiutil create -volname Berth -srcfolder "$stage" -fs HFS+ -format UDZO "$dmg"

    [[ -n $identity ]] || exit 0
    codesign --force --timestamp --sign "$identity" "$dmg"

    if [[ -z ${APPLE_ID:-} || -z ${APPLE_PASSWORD:-} || -z ${APPLE_TEAM_ID:-} ]]; then
        echo "APPLE_ID, APPLE_PASSWORD or APPLE_TEAM_ID is not set: the .dmg is not notarized" >&2
        exit 0
    fi
    # Notarizing the .dmg covers the app inside it; stapling lets Gatekeeper
    # check it offline.
    xcrun notarytool submit "$dmg" --wait \
        --apple-id "$APPLE_ID" --password "$APPLE_PASSWORD" --team-id "$APPLE_TEAM_ID"
    xcrun stapler staple "$dmg"
    spctl --assess --type open --context context:primary-signature --verbose=2 "$dmg"

# Build the portable Windows executable, target/packager/Berth_<version>_x64.exe.
[windows]
[script("bash")]
package:
    set -euo pipefail
    cargo build --locked --release -p berth
    id=$(cargo pkgid -p berth)
    mkdir -p target/packager
    cp target/release/berth.exe "target/packager/Berth_${id##*[#@]}_x64.exe"

# Render the app icon from its SVG, the one source for every platform: the
# PNGs (the Linux packages, and the macOS .icns cargo-packager makes from them)
# and berth.ico (embedded in the Windows executable by build.rs).
[script("bash")]
icons:
    set -euo pipefail
    cd apps/berth/packaging/icons
    for size in 16 24 32 48 64 128 256 512; do
        rsvg-convert -w "$size" -h "$size" no.ahse.berth.svg -o "$size.tmp.png"
    done
    for size in 32 64 128 256 512; do
        cp "$size.tmp.png" "${size}x${size}.png"
    done
    magick 16.tmp.png 24.tmp.png 32.tmp.png 48.tmp.png 64.tmp.png 128.tmp.png 256.tmp.png berth.ico
    rm ./*.tmp.png

# Run by cargo-packager first (before-packaging-command): the release binary
# and, on Linux, the desktop entry.
[linux]
before-packaging:
    cargo build --locked --release -p berth
    just desktop-entry

# Both architectures, merged into the universal binary where cargo-packager's
# `--target universal-apple-darwin` looks for it.
[macos]
before-packaging:
    MACOSX_DEPLOYMENT_TARGET=10.15.7 cargo build --locked --release -p berth --target aarch64-apple-darwin
    MACOSX_DEPLOYMENT_TARGET=10.15.7 cargo build --locked --release -p berth --target x86_64-apple-darwin
    mkdir -p target/universal-apple-darwin/release
    lipo -create -output target/universal-apple-darwin/release/berth \
        target/aarch64-apple-darwin/release/berth target/x86_64-apple-darwin/release/berth

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
