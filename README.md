# Berth

A native Docker Desktop alternative written in Rust and GPUI — OrbStack's
layout, zeron's look, Linux first.

> Status: **M0 visual prototype** — the Containers screen runs on fixture data.
> See [docs/plan](docs/plan/00-overview.md) for the plan, user stories and roadmap.

## Run

```sh
cargo run -p berth
```

Linux needs the usual gpui system libraries (Wayland/X11, xkbcommon, fontconfig,
Vulkan driver). `ctrl+b` toggles the sidebar, `ctrl+q` quits.

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
