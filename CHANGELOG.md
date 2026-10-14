# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0](https://github.com/AleksanderEvensen/berth/releases/tag/v0.1.0) - 2026-10-14

### Added

- new window chrome crate
- portable Windows .exe release with the app icon (M5)
- container and volume Files tabs (B9, C5)
- Windows caption buttons and titlebar drag (M5)
- macOS titlebar drag and signed, notarized universal .dmg (M5)
- keyboard map with shortcut reference; every action reachable by keyboard (M4, NFR-U1)
- logs wrap toggle (M4)
- engine fix-it guide for permission denied and missing engine; complete M1 (A5)
- port menu and full container info (M1: B4, B5) ([#1](https://github.com/AleksanderEvensen/berth/pull/1))
- container multi-select and select-all/clear keys; complete M2 (B11, FR-12)
- volume and image info with inspect_image and clickable used-by (M4, C4, D4)
- networks screen with list, delete and prune unused (M4, E1)
- settings screen with appearance, accent, reduce motion and engine endpoint (M4)
- activity monitor; complete M3
- *(chrome)* clear macOS traffic lights in the titlebar
- live Logs, Stats and Terminal tabs
- volumes, images and clean-up screens with sidebar navigation
- container search (ctrl+f) on gpui-base input
- event-driven store, confirm dialog, toasts, pending states
- *(docker)* Engine contract, model fields and FixtureEngine
- M0 visual prototype and plan docs

### Fixed

- close-only fallback and Windows caption focus ring
- build the window chrome on Windows
- expose Windows caption buttons to screen readers
- draw Windows caption glyphs with the system icon font
- put the engine guide below the endpoint settings
- no guessed subordinate ID range; pin advice covers every fix (A5 review)
- pin-aware recovery text, Arch rootless steps, accurate rootless wording (A5 review)
- remove inert new-container button from Containers header
- address review findings on M3

### Other

- cleaned up repo
- release and packaging flow
- show the engine guide only on Settings → Engine
- replace hand-rolled helpers with popular crates
- *(logs)* parse ANSI with vte instead of a hand-rolled scanner
- virtualize the containers list
- added clippy lints and fixed errors/warnings
- replace fixtures with actual docker engine implementation
- Initial Commit
