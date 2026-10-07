# Design system

Goal: OrbStack's *layout and interaction model*, zeron's *look and feel*.

## What makes zeron look the way it does

From the screenshots in `reference/zeron-design/` and its source
(`reference/zeron-source-code/crates/ui/src/theme.rs`, `shell.rs`, `icons.rs`):

1. **Near-black, achromatic planes with tiny elevation steps.** Content `#060606`,
   shell/sidebar `#0d0d0d`, cards `#0e0e0e`, dialogs `#101010`, popovers `#161616`.
   Separation comes from these small steps plus **hairline borders at 8 % white**,
   not from shadows or colour.
2. **Interaction is white ink at low alpha.** Hover = `white @ 11 %`, active/
   selected = `white @ 16 %`. Never an opaque fill, never a saturated selection
   bar (this is the biggest difference from OrbStack's blue selection).
3. **One accent hue, used sparingly** — indigo `oklch(0.673 0.182 277)` for
   focus, links, inline code, the selected theme tile. Status colours are the
   Tailwind 400 steps (emerald/amber/red) in dark, 600 steps in light.
4. **Text is a three-step neutral ladder**: `neutral-200` primary, `neutral-400`
   muted, `neutral-500` faint. Section headings are muted, not bold.
5. **Geist / Geist Mono**, ~13–14 px UI text, medium weight for titles, mono for
   IDs and technical values.
6. **Solar Icons, Linear weight (1.5 px stroke)**, 16 px in rows, tinted with
   the text colour.
7. **Generous radii, compact rows**: controls 6 px, panels/cards 10 px, rows
   ~8 px; sidebar rows ~30 px tall; 4/8/12/16 spacing ladder.
8. **Unified titlebar**: no system title bar; a 38 px strip that holds the
   sidebar toggle and navigation, drag-to-move everywhere that isn't a control.
9. **Light theme is designed, not inverted**: content goes white, chrome goes
   grey, raised surfaces are white + border + shadow, accents move to the 600
   step. We copy this rule set rather than inventing one.
10. **Restrained motion**: sidebar width and glyph morph animate; hovers fade.

Berth keeps the token *names and roles* from zeron so the two can be compared
side by side, implemented in `crates/ui/src/theme.rs`.

## Can we use GPUI Kit and still look like zeron?

**GPUI Kit** (`gpui-kit`, by Longbridge) is two layers:

| Crate | What it is |
| --- | --- |
| `gpui-base` | *Behaviour* — text input/editor, focus traps, popovers/positioning, dialogs/sheets, virtual lists, scrollbars, select/combobox state, tooltips, tree state. Mostly unstyled. |
| `gpui-component` | *Styled* shadcn-flavoured components on top of `gpui-base` (buttons, tables, sidebar, tabs, …) with its own theme system. |

**zeron itself answers the question:** it depends on `gpui-base` (a fork) and
uses it almost exclusively for the text editor/input, `Checkbox`, scrollbar
styles and `gpui_base::init`. Every visible surface — sidebar, rows, pills,
popovers, tabs, settings widgets — is hand-built on raw gpui `div()`s styled
from its own theme tokens. It does **not** use `gpui-component`.

**Decision for Berth:**
- ✅ **Use `gpui-base`** for hard behaviour: text inputs (search, log filter),
  virtual lists, popover positioning, dialogs/focus traps, tooltips, scrollbars.
  This saves the most engineering time and carries no visual opinion.
- ❌ **Don't build on `gpui-component`'s styled widgets.** Its shadcn look,
  sizing and theme model fight the zeron aesthetic; restyling every component
  through its theme ends up costing more than writing the ~15 small components
  we actually need. Individual pieces (e.g. its table/plot code) can be
  consulted or used selectively later if they save real time — e.g. charts for
  the Stats tab.
- ✅ **Own the styled layer in `crates/ui`**: theme tokens + small composable
  components built from `div()`. They're cheap in gpui (a component is a function
  returning a styled element) and keep the look 100 % under our control.

## Tokens (dark — implemented)

| Token | Value | Use |
| --- | --- | --- |
| `bg` | `#060606` | Main content panel |
| `surface` | `#0d0d0d` | Sidebar, titlebar, list column |
| `surface_card` | `#0e0e0e` | Inline cards (info groups) |
| `surface_dialog` | `#101010` | Dialogs |
| `surface_overlay` | `#161616` | Popovers, menus, palette |
| `surface_raised` | `neutral(0.235)` | Opaque pills/chips |
| `element_hover` | `white @ 0.11` (rows use 0.06) | Hover wash |
| `element_active` | `white @ 0.16` (rows use 0.09) | Selected wash |
| `border` / `border_strong` | `white @ 0.08 / 0.14` | Hairlines |
| `text` / `text_muted` / `text_faint` | `neutral 0.922 / 0.708 / 0.556` | Text ladder |
| `accent` | `oklch(0.673 0.182 277)` | Focus, links, selection accents |
| `success` / `warning` / `danger` | emerald / amber / red 400 | Status dots, destructive actions |

Layout constants: titlebar 38 px, sidebar 240 px (min 200 / max 320),
list column 340 px, row 30 px (sidebar) / 44 px (two-line resource rows),
radius 6 / 8 / 10, spacing 4 / 8 / 12 / 16.

## Component inventory

Status: ✅ in prototype · ⏳ next · 🔲 later

| Component | Status | Notes |
| --- | --- | --- |
| `icon(path)` | ✅ | SVG from bundled assets, tinted. |
| `IconButton` | ✅ | 26 px square, ghost, hover wash; danger variant tints red on hover. |
| `Sidebar` + `SidebarSection` + `SidebarItem` | ✅ | Collapsible (animated width), sections, icon + label + trailing count, active/hover washes, footer. |
| `sidebar_toggle_glyph` | ✅ | Frame + panel glyph whose panel narrows when collapsed (zeron idiom). |
| `ListHeader` | ✅ | Title + subtitle ("Containers · 3 running") + trailing actions. |
| `SectionLabel` | ✅ | Muted small heading inside lists ("Running", "Stopped"). |
| `ResourceRow` | ✅ | Leading badge, title/subtitle, status dot, hover-revealed actions, selected state, indent for stack children. |
| `SegmentedTabs` | ✅ | Info / Stats / Logs / Terminal / Files pill. |
| `KeyValueCard` | ✅ | Rounded card of label → value rows with hairline dividers. |
| `DataTable` (simple) | ✅ | Header row + rows inside a card (ports, mounts, labels). |
| `StatusDot` | ✅ | Running / paused / exited / restarting colours. |
| `TextInput` (search) | ⏳ | Wrap `gpui-base` input with our styling. |
| `Dialog` / `ConfirmDialog` | ⏳ | `gpui-base` dialog + focus trap, zeron dialog surface. |
| `Toast` | ⏳ | Bottom-centre pill like zeron's "agent updates" pill. |
| `Menu` / `Popover` | ⏳ | `gpui-base` positioning, `surface_overlay` + border. |
| `Tooltip` | ⏳ | |
| `Sparkline` / `Chart` | 🔲 | Stats + Activity. Custom `canvas` paths. |
| `LogView` | 🔲 | Virtualized monospace lines, ANSI colours. |
| `Terminal` | 🔲 | alacritty_terminal grid element. |
| `TreeTable` | 🔲 | Activity monitor. |

**Open question (undecided):** build `Button`/`IconButton` and `Dialog` on
`gpui-base`'s unstyled primitives instead of plain `div`s, keeping our styling.
The gain would be keyboard focus and accessibility behaviour we don't have to
maintain; `Dialog` focus trapping would also need `gpui-base`'s Root
integration. Not a code-size win. Raised by the M3 crate audit.
