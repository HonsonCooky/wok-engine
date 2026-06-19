# Handoff: Wok editor authoring surfaces

## Overview

Visual + behavioural spec for the eight per-context views that drop into the
**built, locked shell** of the wok editor (the `wok` binary in the monorepo).
The shell — native title bar, the dockable navigation panel, the tab bar over
the per-context editor area, the contextual status bar — already exists. This
package specifies the views not yet built, in roadmap order.

Read this index for the shared rules (tokens, shell layout, input, invariants),
then the per-view file in `views/` for the surface you're building. Open
`editor-surfaces-spec.html` in a browser for the live look (toggle
System/Light/Dark, top-right).

## Views

| # | View | Roadmap | Spec | Screens |
|---|------|---------|------|---------|
| 1 | Scene view — selection + floating inspector | 3b | [views/1-scene-view.md](views/1-scene-view.md) | [dark](screenshots/dark/1-scene-view.png) · [light](screenshots/light/1-scene-view.png) |
| 2 | Navigation panel — bottom icon bar + Instances tree | 3b | [views/2-nav-panel.md](views/2-nav-panel.md) | [dark](screenshots/dark/2-nav-panel.png) · [light](screenshots/light/2-nav-panel.png) |
| 3 | Place mode — the stamp hotbar | 4 | [views/3-place-mode.md](views/3-place-mode.md) | [dark](screenshots/dark/3-place-mode.png) · [light](screenshots/light/3-place-mode.png) |
| 4 | Terrain — sculpt + paint | 5 | [views/4-terrain.md](views/4-terrain.md) | [dark](screenshots/dark/4-terrain.png) · [light](screenshots/light/4-terrain.png) |
| 5 | Prefab editor — shapes + named states | 6 | [views/5-prefab-editor.md](views/5-prefab-editor.md) | [dark](screenshots/dark/5-prefab-editor.png) · [light](screenshots/light/5-prefab-editor.png) |
| 6 | Lighting — states, sky, curves | 7 | [views/6-lighting.md](views/6-lighting.md) | [dark](screenshots/dark/6-lighting.png) · [light](screenshots/light/6-lighting.png) |
| 7 | Playtest insight — log + trace explorer | 8 | [views/7-playtest.md](views/7-playtest.md) | [dark](screenshots/dark/7-playtest.png) · [light](screenshots/light/7-playtest.png) |
| 8 | Integrity — missing-assets queue + scan | 9 | [views/8-integrity.md](views/8-integrity.md) | [dark](screenshots/dark/8-integrity.png) · [light](screenshots/light/8-integrity.png) |

## About the design files

`editor-surfaces-spec.html` is a **design reference built in HTML** — a prototype
showing intended look, layout, and behaviour. It is **not production code to
copy**. The editor is a **Rust / egui** application; recreate these surfaces in
`wok/src/` using egui and the conventions already in the codebase (`theme.rs`,
`action.rs`, `view.rs`, `workspace.rs`, `menu.rs`).

The screenshots in `screenshots/{dark,light}/` are renders of that HTML, one per
view per theme. Treat any HTML/screenshot detail that conflicts with
`editor-design.md` (the design canon) or `theme.rs` as wrong — the canon and the
live theme win.

## Fidelity

**High-fidelity.** Colours, spacing, and geometry are final and come straight
from `theme.rs`. Recreate the chrome faithfully. The 3D content inside the
viewports (gradient boxes, the grid floor) is **indicative only** — the real
viewport is drawn by `wok-render` into the transparent egui `CentralPanel`; do
not rebuild it in egui.

## The non-negotiables (from `editor-design.md`)

- **One obvious way.** No second path to the same outcome. No command palette,
  no bottom dock, no second inspector dock.
- **Context-driven surfacing.** Only the active workflow's surfaces are on
  screen. The active tab drives the nav panel and the floating layer.
- **The placement boundary.** The editor authors **space** (transform),
  **physical properties** (surface tag, `is_hitbox`, `is_visible`), and
  **identity** (prefab ref, instance id, optional name) — and nothing else.
  **There is no general property bag.** Gameplay config binds in code by id or
  name. Every inspector ends on that boundary line; keep it verbatim.
- **One command vocabulary.** Every model mutation goes through the single
  writer, `action::handle` — that is what makes undo/redo work. Views **read**
  the model and **emit** `Action`s; they never mutate directly. The one
  exception is Playtest insight, which is read-only.
- **Native window.** The OS owns the title bar (title `wok — {project}`); the
  only app-menu is the inline hamburger at the left of the tab bar. No custom
  titlebar, no horizontal menu bar.

## Design tokens — use `theme::palette(ctx)`, never hardcode

Both palettes live in `wok/src/theme.rs`; read the active one back through
`theme::palette(ctx)` so every surface follows the OS light/dark. Values for
reference only (call the palette, don't paste literals):

| Token (Palette field) | Dark      | Light     | Used for |
|-----------------------|-----------|-----------|----------|
| `editor_bg`           | `#181a1f` | `#fcfcfd` | editor well, GPU clear, active tab, input fields |
| `surface`             | `#20232a` | `#f0f1f4` | nav panel, tab bar, status bar, header rows |
| `floating`            | `#262a32` | `#ffffff` | floating inspector / popovers |
| `border`              | `#2e333c` | `#d6d9e0` | hairline borders + separators |
| `hover`               | `#2c313a` | `#e7e9ee` | hover fill |
| `pressed`             | `#343a45` | `#dadde4` | pressed / open fill |
| `text`                | `#c6cad2` | `#2b2d34` | primary text |
| `text_dim`            | `#7d8592` | `#868d9a` | inactive tabs, hints, idle status, hamburger |
| `text_bright`         | `#e4e7ec` | `#16181d` | active tab, hovered controls |
| `accent`              | `#4a86d8` | `#3a6fd6` | active-tab line, selection, the one accent |

Selection fill is the accent at ~30% alpha (`0x4d`), already configured as
`visuals.selection.bg_fill`. Geometry (set in `theme::apply`): no shadows; corner
radius **4** widgets / **6** windows; **1px** borders; `item_spacing 8×6`,
`button_padding 8×4`, `menu_margin 6`, `window_margin 8`, `indent 16`.

Non-palette colours: axis X `#d8534a`, Y `#5bbd5b`, Z `#4a86d8` (deepen slightly
in light); status ok `#5bbd5b`, warn `#caa24a`, error `#d05a4a`.

## Shell layout (built — recreate views inside this)

```
+---------------------------------------------------------------+
| native OS title bar  "wok - sample"          [_] [口] [x]      |  32px, OS-owned
+--------+------------------------------------------------------+
| NAV    | [hamburger] [ tab ][ tab x ]                         |  tab bar 38px (surface)
| PANEL  +------------------------------------------------------+
| 240px  |                                                      |
|(surface| EDITOR AREA (per-context; viewport or data surface)  |  editor_bg
| full   |                                                      |
| height)|                                                      |
|        +------------------------------------------------------+
| [icon  | status bar (left: context | right: diagnostics)      |  26px (surface)
|  bar]  |                                                      |  -- view column only
+--------+------------------------------------------------------+
```

- The **nav panel is full height** on the left (dockable to either side),
  directly under the title bar. In egui terms the `SidePanel` is added **before**
  the `CentralPanel`, so it claims full height and the tab/viewport/status stack
  spans only the remaining width (the **view column**).
- The **status bar belongs to the view column**, not the window — it spans the
  editor area only, never under the nav panel.
- The **hamburger is inline** at the left of the tab bar (File / View / Run /
  Help only).
- The nav panel has a **bottom icon bar** (its own footer). See view 2.

## Interactions & input (shell-wide)

- Camera is **mouse-only and always live** (never a mode): right-drag look,
  scroll dolly along the look, middle-drag pan. **No WASD, no fly mode, no
  camera-mode toggle.**
- Left-click selects, or stamps when the Prefabs tab is active.
- Moves snap to **1m**; rotations to **5°**. Hold Shift or type for finer.
- Commands: `Ctrl+S` save, `Ctrl+Z` undo, `Ctrl+Shift+Z` redo, `Delete` deletes
  selection, `Esc` unwinds place mode then selection.
- Bindings are **focus-gated**: a focused text field types; otherwise keys drive
  the editor.

## State & data

- Views **read** `model` (shell state + open content) and **emit** `Action`s;
  `action::handle` is the single writer (undo/redo). Playtest insight is the lone
  read-only consumer.
- Authored-on-disk forms only: scenes, prefabs, terrain heightmaps, lighting
  states. The editor never holds or writes runtime state.
- Hot reload: the file watcher re-transforms changed chunks during authoring;
  compiled out of release game builds.

## Files in this bundle

- `README.md` — this index (shared rules).
- `views/1..8-*.md` — one spec per view.
- `screenshots/{dark,light}/` — one render per view per theme.
- `editor-surfaces-spec.html` — the live visual reference (open in a browser).

## Files to reference / extend in the codebase

- `wok/src/theme.rs` — palettes + `apply` + `palette(ctx)`. Read colours here.
- `wok/src/action.rs` — the `Action` enum + `handle` (the single writer).
- `wok/src/view.rs` — chrome composition root (region order).
- `wok/src/workspace.rs` — nav panel, tab bar, per-context editor host.
- `wok/src/menu.rs` — the hamburger menu + the status bar.
- `designs/editor-design.md` — the design canon (boundaries + invariants).
- `designs/orchestrator-state.md` — status + the rebuild roadmap.

## How to use this with Claude Code

1. Point CC at this folder + `editor-design.md` + `orchestrator-state.md` — the
   canon is the contract.
2. Build **one view per session**, in roadmap order, design-first, commit green.
   Load only that view's `views/N-*.md` plus this index.
3. Open `editor-surfaces-spec.html` (or the screenshots) for the look; read the
   view file for the widget + action mapping; never copy the HTML.
