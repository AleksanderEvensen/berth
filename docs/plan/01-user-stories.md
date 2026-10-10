# User stories

Persona: **a developer on a Linux workstation** who runs several compose
projects locally (databases, caches, mail sinks, object storage), switches
between them through the day, and periodically runs out of disk because of old
images and volumes.

IDs are referenced from [02-requirements.md](02-requirements.md) and the roadmap.
Priority: **P0** = MVP, **P1** = v1, **P2** = later.

---

## A. Shell & navigation

**A1 (P0)** — As a user I want a sidebar listing every resource area
(Containers, Volumes, Images, Networks, Activity, …) so I can jump between them.
- Sections grouped under small muted headings (Docker / General).
- Active item has a filled wash; hover has a lighter wash.
- Each item can show a trailing count (e.g. running containers).

**A2 (P0)** — As a user I want to collapse the sidebar so the content gets the
full window width.
- Toggle button in the titlebar; keyboard shortcut `ctrl+b` / `cmd+b`.
- Collapsing animates the width; state persists across restarts (P1).

**A3 (P2)** — As a user I want a command palette (`ctrl+k`) to run any action or
jump to any container/image/volume by name.

**A4 (P0)** — As a Linux user I want proper window chrome under Wayland CSD
(drag region, double-click to maximize, min/max/close buttons, resize edges).

**A5 (P1)** — As a user I want to see engine connection status (connected /
reconnecting / not found) in the sidebar footer, with a fix-it hint when the
socket isn't reachable.
- Permission denied explains that the user can't open the engine socket and
  offers, in order: rootless Docker, rootless Podman (user socket), and last
  the `docker` group, labelled as root-equivalent.
- Each option has a short explanation and copyable commands; Berth runs
  nothing itself and reconnects once a socket becomes reachable.
- Not found lists the endpoints that were tried, with the same options; the
  last one becomes starting the system daemon (plus the `docker` group).

## B. Containers

**B1 (P0)** — As a user I want running and stopped containers listed separately
so I can see at a glance what's up.
- "Running" section on top, "Stopped" section below, each with a count.
- Each row: icon, name, image:tag subtitle, status dot.

**B2 (P0)** — As a user I want compose projects grouped as expandable stacks.
- Stack row shows the project name, a stack icon, and how many services run.
- Children are indented under the stack; the chevron toggles expansion.
- A stack is "running" if any child runs.

**B3 (P0)** — As a user I want to start, stop, restart and delete a container
or a whole stack straight from its row.
- Actions appear on row hover / selection (stop or play, delete), always visible
  on the selected row.
- Stack actions apply to every container in the project.
- Delete asks for confirmation (stack delete = `compose down` semantics: remove
  containers + project network, keep volumes unless opted in).
- While an action is in flight the row shows a spinner and disables its buttons.

**B4 (P0)** — As a user I want to open a published port in my browser with one click.
- A link button on running rows with published ports; first HTTP-ish port wins,
  a menu lists all if there are several.

**B5 (P0)** — As a user I want an Info tab for the selected container: name, ID,
image, status/uptime, IP, published ports, mounts, labels, env, command.
- Values are selectable/copyable; IDs render monospace and shortened with copy.
- Mount sources that are named volumes link to the Volumes screen.

**B6 (P1)** — Logs tab: streaming logs with follow, search/filter, timestamps
toggle, copy, clear view, ANSI colors. Handles 100k+ lines without lag.

**B7 (P1)** — Stats tab: live CPU %, memory, network rx/tx, block I/O sparklines.

**B8 (P1)** — Terminal tab: interactive shell (`exec` with tty) using the first
available of `bash`, `sh`.

**B9 (P2)** — Files tab: browse the container filesystem, download/open files.

**B10 (P1)** — Search/filter the container list by name, image or project.

**B11 (P1)** — Multi-select containers and stop/start/delete them in one go.

**B12 (P2)** — Create/run a container from an image with ports, env and volumes.

## C. Volumes

**C1 (P0)** — Volumes listed in "In use" and "Unused" sections with their size.
Header shows the total size.

**C2 (P0)** — Delete a volume from its row; in-use volumes require stopping their
containers first (button disabled with a tooltip explaining why).

**C3 (P0)** — Multi-select volumes and delete them together; the detail pane
shows "N selected · X GB" with a single Delete action.

**C4 (P1)** — Volume info: driver, mountpoint, created, labels, compose project,
used-by containers (clickable).

**C5 (P2)** — Browse a volume's files (via a short-lived helper container).

## D. Images

**D1 (P0)** — Images listed in "In use", "Unused" and "Dangling" sections with
size and age; header shows total size.

**D2 (P0)** — Delete an image from its row; multi-select delete.

**D3 (P0)** — One-click "Remove dangling images" and "Remove unused images"
with a preview of reclaimable space.

**D4 (P1)** — Image info: ID, tags, created, size, platform, command, entrypoint,
workdir, env, volumes, exposed ports, used-by containers.

## E. Networks

**E1 (P1)** — List networks (driver, scope, subnet, attached containers); delete
and prune unused.

## F. Activity monitor

**F1 (P1)** — A tree table of stacks → containers with CPU %, memory, network
and disk rates, sortable by column, plus totals and sparklines for the engine.

## G. Clean-up

**G1 (P0)** — As a user who is low on disk, I want one "Clean up" sheet that
shows reclaimable space per category (stopped containers, dangling images,
unused images, unused volumes, unused networks, build cache), lets me tick
categories and runs the prunes, then reports what was reclaimed.

## H. Settings

**H1 (P1)** — Choose the engine endpoint (auto-detect, Docker context, custom
socket/URL) and see which one is active.

**H2 (P1)** — Appearance: System / Light / Dark, accent color.

**H3 (P2)** — Rebind keyboard shortcuts.
