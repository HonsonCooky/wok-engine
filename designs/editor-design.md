# wok-engine - Editor Design

Design canon for the wok editor, the GUI that authors content for the engine. The editor is an application: it composes engine primitives, owns no engine logic, and the engine never depends on it. It is co-developed in the workspace as the reference application (the wok binary) and is replaceable without the engine noticing. This document carries boundaries, concepts, and invariants; tuning and mechanism for an unbuilt surface settle in that surface's brief when it is built, not here. Status, history, and the build sequence live in orchestrator-state.md, not here.

## Governing principles

Two rules decide every design question below. When a feature fights them, the feature loses.

1. One obvious way. Borrowed from Go: the editor offers one obvious way to do each thing, and complexity must earn its place against the workflow and modest scope, so when two designs reach the same outcome the simpler one wins. Modes reuse one small key set across contexts to cover more ground with fewer keys; they never add a second way to do the same thing.

2. Context-driven surfacing. The shell is fixed, but its contents bind to the active workflow, and only that workflow's surfaces are on screen. Established editors show every option at once, which is the clutter that makes it hard to focus on one thing; ours shows one workflow at a time. Corollary: because every project is the same shape (a Rust game on this engine, with a known layout), the editor needs no configuration UI for what it can infer - Run is one verb, not a panel of run options.

## Role and boundary

The editor is a full content creator. It authors every authored-on-disk form the engine consumes (scenes, prefabs, terrain heightmaps, lighting states) and surfaces content integrity. It writes only the authored forms (HLD data-flow states 1 and 2); it never holds or writes runtime state, and runtime never flows back into it.

The placement boundary (load-bearing). The editor authors space (transform), physical properties (surface tag, is_hitbox / is_visible flags), and identity (the prefab reference, the per-placement instance id, and an optional author-given name). It does not author behavior or per-instance gameplay configuration. The game owns interpretation: it binds its logic and any per-instance config to placements by instance id or name, and if configuration must travel with a placement the game appends it on its own side. There is no general property bag in the editor, so the editor never learns a gameplay schema and stays decoupled from any game built on the engine. Trigger and region volumes are spatial content the editor may author (a volume is just a placed box); the game routes the events, never the editor.

Code and text editing are out of scope; that lives in Zed. The editor hands the engine the materials and a starting state, and the engine brings them to life - it is not a Roblox-style all-in-one, precisely because folding a code editor in would drag a second toolchain into a tool that should stay spatial.

## The shell

Zed is the reference for the frame: when an opinion on layout or structure is needed, follow Zed and tweak mildly for a 3D content tool. Zed governs the frame; it has no opinion on the 3D viewport, which is ours.

Regions:
- Menu bar. File (project lifecycle), Edit (undo, redo, placement operations), Selection, View (the panel, overlays, camera), Go (jump to chunk, frame selection, go to instance), Run (playtest the open content in taste), Window, Help. Run is a single verb, not a configuration surface.
- Navigation panel. One panel, dockable to the left or the right (the user's choice, which suits a left-hand-keyboard and right-hand-mouse setup) and toggleable to reclaim the full editor width. It hosts the active context's navigation (scene tree, prefab library, lighting states, scan), bound to the active tab. A workflow shows at most two navigation views; if it would need more, simplify the workflow rather than the panel.
- Tab bar. One tab per open edit context, over the editor area.
- Editor area. The per-context surface (see Edit contexts); its layout is conditional on the open context.
- Status bar. Contextual: mode, snap setting, placement and draw counts, framerate and frame time, save state, and an integrity count.

The shell deliberately omits a bottom dock, a separate inspector dock (the inspector floats, below), and a command palette. The menu and the home-row verbs already cover action dispatch, so a palette would add a third trigger for no new capability; navigation at scale, if it ever bites, becomes a focused go-to finder, not a general palette.

Action layer. One command vocabulary. The menu and the keybinds dispatch the same actions, and every model mutation is applied at one point, the single writer, which is what makes undo and redo possible. The minimal shell already carries this seam (action::handle).

Floating layer. Scoped to the editor area and clipped to it. It hosts the conditional inspector - present only in selection contexts, tightly coupled to the current selection - and any popovers.

Window model. One editor window. The playtest runs as its own OS window (Run does cargo run into taste, the way Visual Studio launches a separate process); logs are written to a file and explored in a log view rather than tailed in a dock. Nothing else spawns a window.

## Input

The editor is a modal manipulator, not a flight simulator. The hardware sets the grammar, left hand on the keyboard and right hand on the mouse, so the split is by device: the mouse is the motion, the left hand is the operators and the precision.

One obvious way per task, at a given precision. Coarse spatial work is the mouse: drag to move, snapped. Precise and numeric work is the keyboard and the inspector: type an exact value. These are not two ways to move; they are one way each at different precisions. Mouse moves snap to a 1m grid and rotations to 5 degree steps; hold Shift or type for finer values. Concrete tuning settles in the brief.

Modes reuse one small key set across contexts: Object (the selection is the cursor and the left hand operates on it), Place (a prefab is armed and dropped on click), and Free-fly (a god-cam to get around). The same keys mean different things per mode. Because the mouse now moves things, drag-to-move rides a gizmo, which is load-bearing rather than parked.

Stamping. Place mode carries a hotbar of armed prefabs, selected by the number row. The armed set is scene-context model state, mutated through the action layer; the navigation panel edits it and the scene view's hotbar is a view of it, so hiding the panel never drops the stamps.

Commands: Ctrl+S save, Ctrl+Z undo, Ctrl+Shift+Z redo, Delete deletes the selection, Esc unwinds place mode then the selection. Bindings are focus-gated (a focused text field types; otherwise keys drive the editor) and become a rebindable table later. The richer grammar (object-to-object motions, repeat-last, align and distribute) grows as authoring demands it, not all at once.

## Edit contexts

The editor area hosts one tab per open thing, in two kinds. Viewport contexts are a 3D view; data contexts are a parameter or list surface. The active tab drives the navigation panel and the floating layer. Per-context internal layouts are designed as each view is reached, not pre-declared here.

Viewport contexts:
- Scene. The spatial hub: place, select, and transform prefab instances, sculpt and paint the terrain, and drop spawners, triggers, and fog or lighting zones, all in one view through tool modes (place and select, sculpt, paint), because a level is built holistically rather than by tabbing between ground and props. The floating inspector appears on selection; brush parameters appear while sculpting.
- Prefab. One prefab in isolation: compose primitive shapes, define named states (default, open, destroyed), set each shape's surface tag and is_hitbox / is_visible flags, and bind a mesh name for when a real mesh arrives.

Data contexts:
- Lighting. Edit a lighting state (sky gradient, sun, fog, band count, ambient), its animation curves, and the region markers for fog and lighting zones. The 3D view is only a preview.
- Playtest insight. A log and trace explorer over the saved run log (tracing output, structured by spans), for reviewing a playtest after the fact.
- Integrity. The missing-assets queue (the artist's to-build list) and the project-wide deep scan for dead references, orphans, and empty slots, each navigable to its source. The conventions and the scan are an engine concern the editor consumes, not one it owns.
- Cutscene. A timeline over a finished scene. Deferred: it needs the wok-sequence crate, which is unbuilt.

Not their own views: asset binding is the mesh-name field in the prefab editor plus a category in the scan; spawners, triggers, and zones are placeables in the Scene view.

## Build path

Structure before features: the shell frame is built first so each authoring surface drops into it rather than bolting on. The parent shell (the regions, the dockable and toggleable navigation panel, the tab bar, the per-context editor host, the status bar) and the project model (open folder, open recent, browse and open contexts as tabs, save and save all) come first; then each edit context is built as its own frame and then its features, one view at a time. The detailed sequence is the rebuild roadmap in orchestrator-state.md. Proven code from the prior editor (picking, content I/O, the inspector's rotation handling, undo, the action layer) is lifted from git history as each piece returns.

## Cross-cutting

Undo and redo (carried by the action layer), gizmos, snapping, multi-select, multi-chunk authoring, glTF import and asset-name binding, and hot reload (the file watcher re-transforms changed chunks during authoring, and compiles out of release game builds).

## Locked decisions

- Full content creator (2026-06-14). The editor owns authoring for all four authored data types plus integrity. Revisit only if a surface proves better served by an external tool.
- Zed shell grammar (2026-06-14). The editor borrows Zed's frame - a menu bar, docked navigation, a tabbed editor area, a status bar - and tweaks mildly; Zed governs the frame, the viewport is ours.
- Shell shape and interaction (2026-06-16). The two governing principles; a single dockable, toggleable navigation panel (no second dock); a floating conditional inspector; no bottom dock and no command palette; the device-split input model with 1m and 5 degree snap defaults and a stamp hotbar; edit contexts split into viewport and data kinds. All captured above.
- Placement boundary (2026-06-16). The editor authors space, physical properties, and identity only; the game owns behavior and per-instance configuration, bound by id or name. No property bag. Mirrored in the HLD.
