# wok-engine - Orchestrator State

Boot document for the design-orchestration conversation. Read this plus designs/high-level-design.md, designs/project-canon.md, and designs/editor-design.md and you hold the whole board. Updated at each work arc's close; one commit per update.

## Operating model

This conversation drafts copy-paste briefs for Claude Code sessions and reviews their reports; Harrison relays both directions and gives play verdicts. Brief format: header lines `[PASTE INTO: claude-code]`, `[RUN: <tier>]`, `[from: wok-orchestrator]`, then Context, numbered Scope, Outcome criteria (workspace green, counts stated, commit message), Out of scope. New scoped brief = new CC session (`/clear` first); immediate follow-ups to a session's own work stay in that session. CC reports come back as the final report block only, never full transcripts. The canonical docs are the cross-session memory; the orchestrator owns them and Harrison syncs them into designs/.

Model tiering (usage discipline): Tier 0 = no CC, Harrison edits by hand (single constants, tuning.json, content JSON). Tier 1 = sonnet or fable low (docs, cosmetics, one-file mechanical). Tier 2 = opus or fable medium (app-side multi-file features in wok or taste). Tier 3 = fable high (engine crates, cross-crate contracts, architecture, audits; include "read designs/ fresh" only at this tier). Fable is temporarily unavailable; until it returns, run Tier 1 on sonnet and Tier 2 and 3 on opus, raising the effort/thinking with the tier.

Conventions (enforced; full set in CLAUDE.md and project-canon.md): ASCII only, no em or double dashes in prose (divider comments exempt), files kept near a 400-line target and split at natural seams when one holds more than a single concern (a target, not a hard limit), hand-formatted with NO cargo fmt, no new dependencies without asking, commits direct to main with push on instruction, design docs are unwrapped single-line paragraphs.

## Repo state

Workspace: wok-engine, Windows, repo HonsonCooky/wok-engine, main. HEAD 94b56bb (the shell is complete - frame, chrome, and project lifecycle; committed, not pushed). Workspace green and clippy clean at HEAD. The wok crate is the editor shell: 12 source files, 1856 lines, 44 tests - a native-decorated window with a hamburger app-menu (File carries Open Recent and Close Project), a dockable and toggleable navigation panel, a tab bar over a per-context editor area, a status bar, a dynamic light/dark theme following the OS, open-a-project via a native folder picker (rfd) with a persisted recents list (hand-rolled JSON, no serde), and the action::handle single-writer seam over Model { project, shell }. egui_kittest renders the chrome to PNG snapshots (wok/tests/snapshots) as the editor's chrome-level regression check, a dev-dependency only; snapshot tests serialize GPU device creation via a gpu_guard() lock to avoid an intermittent wgpu concurrency crash, and any new snapshot test must take it. The engine crates are unchanged. Boot caveat: this sandbox cannot read the working index (a Windows git index-extension mismatch), so read committed state via git log / git show / git diff A..B, never git status; uncommitted state comes from Harrison.

Crates: wok-platform (window, GPU, input, gamepads, audio-device, frame loop), wok-scene (chunked scene/prefab data, heightmaps, serde, hot-reload watcher, slicing), wok-physics (collision, sweeps, slide, gravity, camera orbit/spring-arm math), wok-mesh (MeshCpu/Gpu, primitive + terrain mesh gen, GPU upload), wok-light (lighting data model, curves, point lights), wok-content (chunk lifecycle part 1), wok-render (forward renderer: banded lighting 32 bands, single shadow map, fog, gradient sky, screen-door fades, debug lines, occlusion ghosting), plus apps wok (editor) and taste. Crate truths and the dependency graph: see the HLD.

taste: a minimal feel core (reduced 863311d; air/landing/precision/slide-feel layers removed). tuning.json schema is move_speed/gravity/jump_velocity/max_jumps/camera. The replay harness (Level 2) lives here. Not the elaborate feel lab anymore.

Editor (designs/editor-design.md is the canonical design): reset, then rebuilt to the shell described above. The prior build's features live in git history for lifting as the rebuild reaches them - the Action vocabulary single writer (3643ad3), undo/redo as snapshot history (4df48a9), the SelectionSet with click / Ctrl+click / marquee multi-select plus set ops and a multi-field inspector (8380fde, d18472f, 98533fe, 68c1c51), the Mode state machine (807d704), and the ground-plane god-cam free-fly (6ba6e8f). The editor design was reworked 2026-06-16 (see In flight); the rebuild follows the shape-first roadmap there.

## In flight / next actions

RESET AND REBUILD (decided 2026-06-15; design reworked 2026-06-16). The editor was reset to a minimal shell (step 0 below, landed a3a1905). Before rebuilding, this arc reworked the editor design: editor-design.md now carries two governing principles (one obvious way; context-driven surfacing), the Zed-framed shell (native window decorations with a hamburger app-menu, one dockable and toggleable navigation panel, a tab bar, a per-context editor area, a floating conditional inspector; no bottom dock, no command palette), a dynamic light/dark theme following the OS, the device-split interaction model (mouse for coarse snapped motion, keyboard and inspector for precision, a stamp hotbar), the placement boundary (the editor authors space, physical properties, and identity only; the game owns behavior and per-instance config, bound by id or name), and the edit contexts split into viewport and data kinds. Only the wok crate resets; the engine crates and the docs are the stable foundation, and proven editor code is lifted from git history as each piece returns. Work proceeds shape first: the shell frame first, then each view as its own frame then its features, one piece per brief, design first, build small, commit green.

Rebuild roadmap (shape first; structure before features):
0. Minimal shell - DONE (a3a1905): window, menu, open a project via a native folder picker, the action single-writer seam.
1. Shell frame - DONE (0adae59; chrome polished in 12bbdcb and 4b0610a): native-decorated window, hamburger app-menu, one dockable and toggleable navigation panel, a tab bar over a per-context editor host, status bar, with placeholder contents and the region behaviors (dock side, toggle, open and close and switch tabs), a Zed-aligned dynamic light/dark theme, and an egui_kittest chrome snapshot harness.
2. Project model - DONE (94b56bb): open folder, open recent (persisted), and close project, through the single-writer seam (Open Recent reuses OpenProject, one obvious way). The content browser, opening content as typed tabs, and Save / Save all were deferred to the scene view, where they have content to act on. This completes the shell.
3. Scene view, frame and spatial core: a god-cam plus load and render the chunk's content (terrain, placeholder prefabs, lighting) into the viewport; picking and selection; the floating inspector.
4. Scene assembly: place via the stamp hotbar, transform (drag and snap on a gizmo, keyboard for precision), delete, duplicate, multi-select, undo and redo, snapping.
5. Terrain tool modes inside the Scene view: sculpt (raise, lower, smooth, flatten) and surface paint.
6. Prefab view: compose shapes, named states, surface tags, hitbox and visible flags, mesh-name binding.
7. Lighting view: lighting states, sky and sun and fog and bands and ambient, animation curves, fog and lighting zones.
8. Playtest insight view: Run into taste (separate OS window), logs to file, the log and trace explorer.
9. Integrity view: the missing-assets queue and the dead-reference and orphan scan; pairs with the glTF loader and asset binding.
10. Cutscene view: a timeline over a finished scene. Gated on the wok-sequence crate.

Per-view internal layouts (the "blue frame components") are designed as each view is reached, not pre-declared. The shell (steps 0-2) is complete; blue begins. Step 3, the Scene view (god-cam plus render the chunk's content, picking and selection, the floating inspector), is next, and it brings the content browser and Save with it; it gets a design pass on its layout before a brief.

## Verdict backlog (Harrison owes)

1. Editor friction from real authoring, once it is exercised by hand.
2. taste feel: prior air/jump verdicts SUPERSEDED by the reduction; revisit only if the minimal core needs tuning.
3. SHOW_RETICLE default (play-test question, carried from before).

## Parked ledger (deferred; consolidate now, build later)

Built in the prior arc, then reset; now in the rebuild roadmap and lifted from git history as each returns: undo/redo, multi-select (click/Ctrl/marquee + set ops + multi-field inspector), the modal foundation (Object/Free-fly modes), the ground-plane god-cam, object-mode movement.

Editor interaction detail (deferred): a count multiplier so coarse keyboard moves are not many taps, plus keyboard rotate and scale verbs as a fast path (the inspector already does rotate and scale). Resolved by the 2026-06-16 rework and no longer open: the navigation panel is toggleable and the editor does not assume full width when it is open, Object mode no longer auto-locks the camera, and frame-the-selection is an explicit Go action.

Editor content surfaces (unbuilt; the view-fill work after the shell): prefab creator (compose primitive shapes, named states, per-shape surface tag and hitbox/visible flags, mesh binding); terrain sculpt + surface paint; lighting authoring (light states, curves, fog/lighting zones, sky); content scan + integrity (the missing-assets queue for Ryan, dead-ref/orphan checks; pairs with the glTF loader).

Editor shell (built; editor-design.md has the locked shape): the frame, the hamburger app-menu with project lifecycle (open recent, close project), the dockable toggleable navigation panel, tabs, status bar, and dynamic light/dark theme are in. Still unbuilt: rebindable keybind table, multi-chunk authoring, gizmos, surface and grid snapping, non-uniform scale in the inspector; the project content browser and Save ride with the scene view. Declined: a command palette (the menu and home-row verbs cover dispatch; a go-to finder is the fallback only if navigation at scale bites), a bottom dock, and a separate inspector dock (the inspector floats; navigation is one dockable panel).

Asset pipeline: glTF loader (real meshes; wok-mesh), asset binding (placeholder shape to final mesh by name).

Publishing and packaging (engine/game layer, parked): release game builds strip hot reload (the wok-scene file watcher compiles out behind a feature, off in release); baking the authored JSON into a packed binary is deferred under earn-its-place, since heightmaps are already binary and parsing JSON at load is fine for modest fidelity on dev laptops; revisit if load time, package size, or tamper-resistance pressure appears.

Engine (parked; anchored in code or HLD): wok-content part 2 (streaming, worker, eviction, eagerness; criterion: worlds outgrow load-everything); wok-light bake (needs wok-physics raycasts) + dynamic pool (needs renderer consumer); surface-tag friction (wok-scene surface_at; criterion: content wants ice/mud); swept ellipsoid, capsule prefab colliders, broadphase, swept terrain slide (wok-physics); engine capsule locomotion path (supported API, no consumer, pinned by locomotion_replay); Level 3 screenshot harness (wok-render); faded-item shadow/depth policies; camera near-plane transit during arm recovery; normal_at widen-on-shimmer; rim exponent; wok-anim, wok-audio, wok-sequence crates. 120hz declined; SIM_HZ 60 locked by verdict.

## People and context

Harrison: founding backend engineer, builds wok-engine with brother Ryan (creative direction, assets via meshy.ai; the missing-assets scan becomes his queue). Game lineage: Ratchet & Clank / Jak / Sly / BFBB; working title context "Unstitched" (never in docs). The game itself is a future downstream repo; taste is the demo and feel laboratory, not the game. Harrison's setup: ZSA Voyager (left hand) plus mouse (right hand), which is why the editor is modal and device-split (left hand operators and precision, right hand motion, all focus-gated).
