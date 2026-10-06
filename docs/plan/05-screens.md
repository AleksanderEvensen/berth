# Screens

Layouts follow the OrbStack references in `reference/orbstack-screenshots/`;
visuals follow the [design system](04-design-system.md). Differences from
OrbStack are called out explicitly.

## Shell (all screens)

```
┌────────────┬──────────────────────┬──────────────────────────────────────┐
│ ◧  (title- │ Containers      ⚲  + │        [Info|Stats|Logs|Terminal]  ⋯ │ 38px titlebar (drag)
│  bar strip)│ 3 running            │                                      │
│ DOCKER     ├──────────────────────┤                                      │
│ ▣ Contain 3│ RUNNING              │                                      │
│ ▤ Volumes  │ ▾ ≋ tihlde-voting ■ 🗑│        detail pane (bg plane)        │
│ ▦ Images   │     postgres         │                                      │
│ ⋔ Networks │     rustfs           │                                      │
│ GENERAL    │ STOPPED              │                                      │
│ ⌁ Activity │ ▸ ≋ photon_dev   ▶ 🗑 │                                      │
│ ⌫ Clean up │                      │                                      │
│            │                      │                                      │
│ ● Docker   │                      │                                      │
│   28.3 · ⚙ │                      │                                      │
└────────────┴──────────────────────┴──────────────────────────────────────┘
  sidebar        list column            detail
  (surface)      (surface)              (bg — darkest plane)
```

- **Sidebar** (collapsible): groups *Docker* (Containers, Volumes, Images,
  Networks) and *General* (Activity, Clean up). Footer shows engine status
  (dot + name + version) and a settings button. Kubernetes/Machines from
  OrbStack are deliberately omitted for v1.
- **Collapsed sidebar**: width animates to 0; the toggle stays in the titlebar
  at the left of the list column header.
- **Titlebar**: no separate bar — the top 38–52 px of each column *is* the
  titlebar and is draggable. Linux CSD caption buttons sit at the far right.
- Difference from OrbStack: selection is a white wash (zeron), not a blue bar;
  section headings are small muted text; the detail pane is the darkest plane.

## Containers (`orb-container-screen-*`)

**List column**
- Header: "Containers" + "N running"; actions: search, filter/sort menu, new (+).
- Sections: *Running* (implicit in OrbStack — we label it), *Stopped*.
- Compose stack row: chevron, stack icon, project name, `n/m running`;
  hover actions: stop/start, delete.
- Container row (indented under stack, or top-level): image badge with status
  dot, name, `image:tag` subtitle; hover actions: open-port link (running +
  published port), stop/start, delete.
- Selected row: active wash; its actions are always visible.

**Detail pane** — segmented tabs `Info · Stats · Logs · Terminal · Files`.
- *Stack selected*: Info lists services with status dots + "Open in file
  manager" for the compose working dir (`com.docker.compose.project.working_dir`).
- *Container Info*: card 1 Name/ID/Image/Status; card 2 Domain/IP;
  "Ports" table (host → container / protocol, host port clickable);
  "Mounts" table (source → destination, named volumes link);
  "Labels" table (key / value, compose labels de-emphasised).
- *Stats*: 2×2 grid of cards (CPU, Memory, Network, Disk) with current value top
  right and a sparkline.
- *Logs*: toolbar (search input, wrap toggle, copy, clear) + monospace log view;
  follow-tail by default, pauses when the user scrolls up.
- *Terminal*: full-pane terminal; reconnect button when the exec ends.
- *Files*: path breadcrumb + file list (P2).

## Volumes (`orb-volumes-*`)
- Header: "Volumes" + total size. Sections *In use*, *Unused*.
- Row: coloured volume glyph, name (middle-ellipsized for hashes), size; delete
  on hover (disabled for in-use with tooltip).
- Multi-select → detail pane shows "N selected · X MB" and a Delete button
  (OrbStack only shows "6 Selected" — we add the action).
- Tabs: Info · Files.

## Images (`orb-image-list-and-inspect.png`)
- Header: "Images" + total size; header menu: *Remove dangling*, *Remove unused*.
- Sections *In use*, *Unused*, *Dangling*. Row: badge, `repo/`**name**`:tag`
  (name emphasised, repo/tag muted), "size, age".
- Info: ID/Tag/Created/Size/Platform card, Export row, Config card
  (Command/Entrypoint/WorkingDir), Environment table, Volumes, Exposed ports,
  Used by (links to containers).
- Tabs: Info · Terminal (run a throwaway container) · Files.

## Networks
- Sections *In use*, *Unused*. Row: name, driver · subnet.
- Info: driver, scope, subnet/gateway, internal/attachable, connected containers.

## Activity monitor (`orb-activity-monitor.png`)
- Full-width (no list column). Tree table: Containers → stacks → containers,
  plus an "Engine" row; columns CPU %, Memory, Network, Disk, sortable.
- Bottom strip: four summary cards with totals + sparklines.
- Zebra striping replaced by hairline row dividers + hover wash.

## Clean up (new — not in OrbStack)
- Full-width page of category cards: Stopped containers, Dangling images,
  Unused images, Unused volumes, Unused networks, Build cache — each with count,
  reclaimable size, a checkbox and an expandable list of what goes.
- Primary button "Clean up X GB" (solid plate) → confirm → progress → result.
