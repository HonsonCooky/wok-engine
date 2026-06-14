# wok-engine - Editor Design

Design canon for the wok editor, the GUI that authors content for the engine. The editor is an application: it composes engine primitives, owns no engine logic, and the engine never depends on it. It is co-developed in the workspace as the reference application (the wok binary) and is replaceable without the engine noticing. Like the HLD, this document carries boundaries, concepts, and invariants; tuning and mechanism for an unbuilt surface are settled in that surface's brief when it is built, not pre-declared here.

## Role

The editor is a full content creator. It authors every authored-on-disk form the engine consumes (scenes, prefabs, terrain heightmaps, lighting states) and surfaces content integrity. It is the tool side of the compass "engine creates content; game connects content": it produces the authored data, and the game runs it. It writes only the authored forms (HLD data-flow states 1 and 2); it never holds or writes runtime state, and runtime never flows back into it.

## Locked decisions

- Full content creator (2026-06-14). The editor owns authoring for all four authored data types plus integrity, not just scene assembly. Revisit only if a surface proves better served by an external tool.
- Zed shell grammar (2026-06-14). The editor borrows Zed's layout and project model: a menu bar, a left and right and bottom dock frame, a tabbed center, a command palette over one action layer, and a status bar.
- Tabbed edit contexts (2026-06-14). Each thing you edit (a scene, a prefab, terrain, a lighting state) opens as a center tab, the way Zed opens a file. The active tab drives what the left panel lists and what the inspector shows.

## The shell

The shell is Zed's, with wok's verbs.

- Project. A project is a content folder, the same model Zed uses for a source tree. The File menu carries the lifecycle: new, open folder, open recent, save, save all, close project, close window.
- Menu bar. File (project lifecycle), Edit (undo, redo, placement operations), Selection, View (docks, overlays, camera), Go (jump to chunk, frame selection, go to instance), Run (playtest the open content in taste), Window, Help.
- Action layer. One command vocabulary. The menu, the command palette, and keybinds all dispatch the same actions, and every model mutation is applied at one point, which is what makes undo and redo possible. The editor already has this vocabulary (the Action enum the panels emit) and a frame-loop apply point; completing it so it is the single writer is the first structural step.
- Docks. The left dock hosts the panel switcher and the active panel (Scene, Prefabs, Integrity). The right dock hosts the Inspector (today a floating window). The bottom dock hosts Integrity and a log.
- Status bar. Camera speed, framerate, frame time, placement and draw counts, save state, and an integrity count.

## Input

The editor is driven from the left hand on the keyboard and the right hand on the mouse, with the touch-typing home row kept under the fingers so moving between authoring and typing never repositions the hand. Bindings are focus-gated: while a text field holds keyboard focus the keys type, otherwise they drive the viewport. The scheme below is the default; it becomes a rebindable table when the command palette lands, not a hardcoded law.

Camera movement is bare home-row keys, vim-style and all on one row: s strafe-left, d back, f forward, g strafe-right. Holding Ctrl turns that row to vertical and command use: Ctrl+f ascends, Ctrl+d descends, and Ctrl suppresses planar movement so a command chord like Ctrl+S never also strafes. Forward follows the look direction (flying up a slope is look-up-and-go), so the Ctrl vertical is a deliberate world-space elevator. f and g share the index finger, so forward-right has no chord: an accepted cost of the single-row layout.

Mouse: right-drag orbits the camera, right-click opens the edit menu on what it hits, the scroll wheel sets fly speed. Left-click selects one placement, replacing the selection; Ctrl+left-click toggles a placement in or out of the selection set. Left-drag reads its start: pressed on a selected placement it repositions the whole selection, pressed anywhere else (empty space, or an unselected placement, which it passes over rather than grabs) it is an area marquee that replaces the selection, or adds to it while Ctrl is held.

Commands: Ctrl+S save, Ctrl+Z undo, Ctrl+Shift+Z redo, Delete removes the selection, Esc cancels place mode then the context menu then the selection. Duplicate, rename, and frame-on-selection live in the edit menu until they earn hotkeys.

Multi-selection is the set behind Ctrl+click and the marquee: it turns the single selection into a set and touches picking, the inspector, the tree, drag, and delete, so it is built as its own arc rather than bundled with the nav remap.

## Edit contexts

The center hosts one tab per open thing, in four kinds:

- Scene: a chunk, later a multi-chunk region, of placements.
- Prefab: one prefab and its states.
- Terrain: one chunk's heightmap.
- Lighting: one lighting state.

Opening a context from a panel adds a tab; closing it removes it; the active tab sets the left panel's contents and the inspector's fields.

## Authoring surfaces

- Scene, assemble. Place prefab instances, move and rotate and scale by gizmo, multi-select, snap to grid and to surfaces (the same wok-physics queries the editor already uses for picking), duplicate, rename, delete, and author across chunk seams with neighbors visible.
- Prefab, create. Compose primitive shapes into a prefab, transform each shape, set its surface tag and its is_hitbox and is_visible flags, define the prefab's named states (default, open, destroyed), and bind a mesh name for when a real mesh arrives.
- Terrain, shape. Sculpt the heightmap (raise, lower, smooth, flatten) at one-metre resolution within the +/-32m range, and paint per-cell surface tags.
- Lighting, author. Edit a lighting state (sky gradient, sun, fog, band count, ambient), its animation curves, and region markers for fog and lighting zones.
- Integrity, verify. The reserved third page. A missing-assets queue (the to-build list for the artist) and a deep scan for dead references, orphans, and empty slots, each navigable to its source. The conventions and the scan are an engine concern the editor consumes, not one it owns.

## Cross-cutting

Undo and redo (carried by the action layer), gizmos, snapping, multi-select, multi-chunk authoring, glTF import and asset-name binding, and hot reload (already present: the file watcher re-transforms changed chunks).

## Boundary

The editor owns no engine logic and no game logic. It composes wok-physics queries for picking, placement, and snapping, and authors content only. Gameplay, the actor pool, game cameras, save format, and the runtime loops live in the game, never here.

## Build path

Structure before features: the shell is built first so each authoring feature drops into it rather than bolting on.

1. Action layer as the single writer, then undo and redo riding it, then the menu bar and command palette dispatching through it.
2. Dock frame; the inspector moves from a floating window into the right dock; the bottom dock is reserved.
3. Project model: open folder, open recent, save all, close project.
4. Center tabs: a scene tab plus the ability to open other contexts as tabs.

Then features fill the surfaces, the prefab creator first: it is the surface with no editor support today and the worst hand-authored-JSON experience, and it is where content is born.

## Current state

The Zed-shaped shell exists (paged left panel, floating details window, status bar, panel-switch icons). Scene assembly works on a single chunk: place, delete, duplicate, rename, drag-to-move with a vertical modifier, and a transform, YXZ-rotation, and state inspector. The Prefabs page is a read-only list that arms placement; the Scan page is a reserved, disabled slot. Terrain and lighting load and render but are not editable; prefabs are placed but not authored. An action vocabulary and a frame-loop apply point exist and are the seam the command layer grows from.
