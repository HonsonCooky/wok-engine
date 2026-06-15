# wok-engine - Editor Design

Design canon for the wok editor, the GUI that authors content for the engine. The editor is an application: it composes engine primitives, owns no engine logic, and the engine never depends on it. It is co-developed in the workspace as the reference application (the wok binary) and is replaceable without the engine noticing. Like the HLD, this document carries boundaries, concepts, and invariants; tuning and mechanism for an unbuilt surface are settled in that surface's brief when it is built, not pre-declared here.

## Role

The editor is a full content creator. It authors every authored-on-disk form the engine consumes (scenes, prefabs, terrain heightmaps, lighting states) and surfaces content integrity. It is the tool side of the compass "engine creates content; game connects content": it produces the authored data, and the game runs it. It writes only the authored forms (HLD data-flow states 1 and 2); it never holds or writes runtime state, and runtime never flows back into it.

## Workflows

Authoring is not a linear pipeline; it is two tracks that build in parallel, converge at placement, and sit inside an iterate loop, with cross-cutting work running throughout. The journeys:

- Open a project (a root folder for one game) and create a scene. A project holds many scenes; level one is the first of many.
- Environment track: sculpt the terrain (mountains, hills, ponds), paint its surfaces (grass, stone, and so on), then set lighting, water levels, and fog and lighting zones.
- Object track: create prefabs, hitboxes built from placeholder primitive shapes with named states and surface tags, standing in for the final mesh that arrives later.
- Place: the hub where both tracks converge and the scene is assembled. This is the finicky core and the reason the modal manipulator model (see Input) exists; it is the convergence point of every other workflow, so it earns the most care (selection-as-cursor, snapping, grid steps) and is built before the feeder surfaces.
- Iterate: author, play-test in taste (the Run verb), adjust. Hot reload makes this a loop, not a one-way march, and placement especially lives in it.
- Cutscenes: a future edit context layered on a finished scene (wok-sequence data plus an authoring UI).

Cross-cutting, running throughout rather than as a final step:

- Content integrity: the scan answers what is still to build (the artist's queue) and what is broken (dead references, orphans, empty slots). It is the asset-decoupled payoff, content and code proceeding in parallel because the project can always be scanned for what is missing.
- Asset binding: the placeholder-to-final swap. A prefab is hitboxes and placeholder shapes now; when a real mesh arrives it is bound by name. This is the moment content and code merge.

Scaling and boundary: a scene spans many 128m chunks, so terrain and placement scale through multi-chunk authoring, and a project scales through multi-scene management. The editor authors spatial data only; it may author trigger and region volumes (content the game consumes), but the game routes the events, never the editor.

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

The editor is a modal manipulator, not a flight simulator: the selection is the cursor, and most work is acting on it. The hardware sets the grammar, left hand on the keyboard and right hand on the mouse, so unlike two-handed vim the split is by device: the mouse is the motion (which object, where), the left hand is the operators (what, and how much). Bindings are focus-gated (a focused text field types, otherwise keys drive the editor) and become a rebindable table with the command palette.

Modes. Object mode is the default: the selection is the cursor, the camera auto-locks to it, and the left hand operates on it. Place mode arms a prefab and drops it on click (the insert analog). Free-fly mode is toggled by a thumb or layer key: a first-person fly to get around, where the left hand drives the camera and the mouse looks. The same keys mean different things per mode, which is what lets a one-handed key set cover everything.

Mouse, the motion. Left-click selects one placement (replace); Ctrl+click toggles one in or out; a left-drag is a marquee (replace, or Ctrl to extend); right-click opens the edit menu; right-drag looks (free-fly mode); the scroll wheel sets fly speed. The mouse is selection-only: it answers which object (and, in place mode, where to drop); the keyboard moves the selection.

Left hand, the operators. In object mode the home row acts on the selection: nudge, rotate, and scale, axis-locked and quantified by a count (the 1-5 number row) so motion is in discrete grid steps, plus delete, duplicate, and change-the-prefab. Verb plus count plus axis is the core grammar, the mouse having already supplied the which. These verbs call the same transform-the-selection primitives the inspector's multi-edit uses.

Commands. Ctrl+S save, Ctrl+Z undo, Ctrl+Shift+Z redo, Delete deletes the selection, Esc unwinds place mode then the context menu then the selection. The command palette is the typed command line (the vim colon analog): place, select by query, align, go to chunk.

The model commits to the paradigm (modal, selection-as-cursor, mouse-motion and left-operator, a free-fly toggle) and grows the richer grammar (object-to-object motions, repeat-last-op, registers, marks, align and distribute) as authoring demands it, not all at once. The modal rework is underway: object and free-fly modes are split and toggled (backtick), the camera locks to the selection in object mode while WASD flies in free-fly, the mouse is selection-only, and the object-mode home row nudges the selection a grid step; rotate, scale, and the count multiplier follow. It reworks the input layer while the spine (the selection set, the action layer, undo) stands.

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

The Zed-shaped shell exists (paged left panel, floating details window, status bar, panel-switch icons). Scene assembly works on a single chunk: place, delete, duplicate, rename, keyboard nudge of the selection (object mode), and a transform, YXZ-rotation, and state inspector. The Prefabs page is a read-only list that arms placement; the Scan page is a reserved, disabled slot. Terrain and lighting load and render but are not editable; prefabs are placed but not authored. An action vocabulary and a frame-loop apply point exist and are the seam the command layer grows from.
