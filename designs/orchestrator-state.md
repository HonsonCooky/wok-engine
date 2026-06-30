# wok-engine - Orchestrator State

Boot document for the design-orchestration conversation: current truth, the active plan, and recent decisions. Read this plus orchestrator-handbook.md and the design canon (editor-design.md, high-level-design.md, project-canon.md) and you hold the board. This doc holds CURRENT STATE only - it is NOT a per-bite log; git is the history (git log / git show for the blow-by-blow). Update HEAD, the active plan, and the decisions at each arc's close, and prune stale narrative rather than appending to it.

## Operating model

This conversation drafts copy-paste Claude Code (CC) briefs and reviews their reports; Harrison relays both directions and gives play verdicts. Full role, brief format, tiering, and constraints live in orchestrator-handbook.md. Tiering in short: T0 Harrison hand-edits (constants, tuning.json, content JSON); T1 sonnet (docs, cosmetics, one-file mechanical); T2 opus (app-side multi-file in wok or taste); T3 opus (engine crates, cross-crate contracts, audits). Conventions: ASCII only (no em or double dashes), files near a ~400-line target, hand-formatted with NO cargo fmt, no new dependencies without asking, commits direct to main with push only on instruction, design docs unwrapped one-line-per-paragraph. Boot caveat: this sandbox cannot read the git working index (a Windows index-extension mismatch), so read committed state via git log / git show / git diff, never git status; uncommitted state comes from Harrison.

## Repo state

Workspace wok-engine, Windows, repo HonsonCooky/wok-engine, main. HEAD bc241ee.

What is live: the engine crates are stable. The editor (the wok binary) has its shell - a native-decorated window, a hamburger app-menu with project lifecycle (open folder / open recent / close), one dockable and toggleable navigation panel with a bottom icon bar (project group Scenes/Prefabs, this-scene group Instances/Lighting), a tab bar, a status bar, and an OS-following light/dark theme - and its scene-view core: open any folder as a project, the nav lists content via wok-scene's ContentLayout, open a scene as a tab, a group-by-prefab Instances tree, a floating inspector that renames and edits the transform (Position editable, Rotation a read-only axis-angle readout, Scale uniform; fixed 2dp), Ctrl+S writing the chunks back, and a 3D viewport rendering the open scene (terrain + placeholder prefab shapes + the scene's lighting) with live edits reflected the same frame. The editor opens taste/ to author. The 3D pass is not snapshot-tested (egui_kittest sees only the egui chrome); chrome is PNG-snapshot regression-tested.

Current work: the editor's interaction layer is being rebuilt incrementally, one workflow per bite - see the active plan. D0 (demolish) and W1 (get-around camera) are DONE and feel-locked; the rebuild proceeds from W2 (click-to-select). The viewport now flies: hold RMB to fly with WASD + mouse-look, scroll to dolly.

Crates: wok-platform (window, GPU, input, gamepads, audio-device, frame loop), wok-scene (chunked scene/prefab data, heightmaps, serde, hot-reload watcher, ContentLayout), wok-physics (collision, sweeps, slide, gravity, camera orbit/spring-arm math), wok-mesh (MeshCpu/Gpu, primitive + terrain mesh gen, GPU upload), wok-light (lighting data model, curves, point lights), wok-content (chunk lifecycle part 1), wok-render (forward renderer: banded lighting 32 bands, single shadow map, fog, gradient sky, screen-door fades, debug lines, occlusion ghosting), plus apps wok (editor) and taste. The dependency graph and crate truths are in the HLD.

taste: a minimal feel core (tuning.json schema move_speed/gravity/jump_velocity/max_jumps/camera) plus the Level-2 replay harness; the demo and feel lab, not the game. Latest game-feel fix af59796: a gated ground-snap so the player does not jitter walking DOWN terrain slopes (prefab ramps - the collide-and-slide path - are not covered; a separate follow-up if they jitter).

editor: editor-design.md is the canonical design. Reset then rebuilt to the shell + scene-view core above; proven prior-build code (picking, content I/O, undo, the action layer) is lifted from git history as the rebuild reaches each piece. Prior complete build preserved at c35ac01.

## Active plan

INTERACTION REBUILD - Unity-incremental (decided 2026-06-29). Demolish the interaction layer, then build it up ONE user workflow per bite, Unity-style: each mechanic introduced only when its workflow needs it and locked once it feels right. (History: a bespoke keyboard-first cluster/toggle/held-layers grammar was designed and partly built through R1, then rejected as unintuitive; the movement-camera-design.md that held it is retired and its surviving camera/snapping direction folded into editor-design.md's Input section.)

- D0 DEMOLITION - DONE (6eec389): interaction.rs deleted; camera reduced to one static vantage (camera.rs 147, camera/modes.rs 78 - the parked file-size debt cleared); ViewportGesture / ToggleTarget / the Target enum removed. KEPT live: the action single-writer seam (incl. SetInstanceTransform), the selection state + Instances tree, the inspector + Ctrl+S, the render path. PARKED (allow dead_code) for the select + move workflows: RenderScene::pick / surface_ray / instance_aabb, the camera cursor_ray, and the geom helpers (snap / rest_y / rotate_step / ray_vs_ground_plane). Render-only baseline, no viewport interaction; wok 124 tests (104 logic + 20 snapshots).
- W1 GET-AROUND CAMERA - DONE, feel-locked (9f62068; FLY_SPEED hand-tuned 10 -> 16 m/s). The free-fly camera re-lifted from 1677821 (pre-demolition), trimmed to get-around, with one change: smooth dt fly, not the old discrete 2 m steps. Hold RMB to fly (WASD along the look, A/D level strafe, E/Q up/down, Shift 5x boost), mouse-look (sensitivity 0.0035, pitch clamp 1.55), scroll dollies (2 m/notch). New wok/src/viewport.rs is the seam-input home that W2 select / W3 move plug into next; its cursor lock + over-well gating obey sharp-edges 2. Stale snap label dropped (chrome snapshots regenerated). Camera-drive math unit-tested; frame_to / advance / ground_forward stay parked for the Frame verb (returns with selection).
- SUBSTRATE FIX with W1 (bc241ee, wok-platform): the input collector stuck held letter keys across a Shift change (logical_key case-desync) and on focus loss; fixed by case-folding printable keys at the tracking edge and clearing held sets on Focused(false). Cross-cutting (guards taste and any input consumer); rule in sharp-edges.md section 5. Held symbol keys across Shift stay a latent desync, deferred (track by physical_key if a held symbol action ever appears).
- W2 click-to-select in the viewport (NEXT); then W3 move a selected instance; W4 rotate / scale; W5 snapping toggles (grid + rotation + grounded/surface-first). The durable direction (free-fly + angle presets + explicit Frame; snap-assisted placement; one-spatial-system prefab dive) is in editor-design.md's Input section.
- Then: multi-select, the prefab dive, a scatter brush (paint multi-selected prefabs with position/rotation/scale jitter; rides on free-placement + surface-snap), the Walk camera, and the controller pass.

REBUILD ROADMAP (shape first, the editor's overall build order): 0 minimal shell - DONE. 1 shell frame - DONE. 2 project model - DONE. 3 scene view: frame + scene-view core DONE, interaction being rebuilt (above). 4 scene assembly: place via a stamp hotbar, transform, delete, duplicate, multi-select, undo/redo, snapping. 5 terrain tool modes in the Scene view: sculpt + surface paint. 6 prefab view -> the content-authoring suite (model/paint/rig/animate/sockets; content-authoring-design.md). 7 lighting view. 8 playtest insight (a log/trace explorer over the saved run log). 9 integrity (the missing-assets queue + dead-ref/orphan scan). 10 cutscene (gated on the unbuilt wok-sequence crate). Per-view layouts are pre-specified in design_handoff_editor_surfaces/ (README + views/1-8 + the spec HTML + dark/light screenshots); each view's brief loads its views/N file and the README and takes the layout from there.

ENGINE BITE QUEUED - cross-chunk editing: placements are stored chunk-local, so authoring an instance across a chunk boundary needs a world-to-local re-home (which also fixes the known chunk-local move bug - a move writes world XZ into chunk-local translation, exact only for chunk 0,0). A T3 engine bite, sequenced when multi-chunk authoring or a macro survey view needs it; single-chunk scenes (all we have) are unaffected.

## Recent decisions

- Interaction model (2026-06-29): Unity-incremental rebuild; free-fly camera + angle presets + explicit Frame; snap-assisted placement (grid / rotation / grounded toggles, surface-first, defaults on); the world and a prefab as one spatial system entered by a dive; controller-mappable, never required. Canon in editor-design.md (Input). The earlier bespoke cluster grammar is dropped and movement-camera-design.md retired.
- Inspector layout (2026-06-26): Position even editable X/Y/Z, Rotation a read-only axis-angle readout (the quaternion shown readably), Scale uniform (per-axis only when non-uniform), fixed 2dp monospace. (editor-design.md.)
- Content-authoring suite (2026-06-26): the Prefab view expands into an opinionated in-editor model/paint/rig/animate/socket tool (MS-Paint-grade), so the team authors low-poly assets without Blender. CSG-core modeling via a kernel dependency, capped at envelope skinning, height-painted normals, glTF interchange with organic rigged characters via import. Canon in content-authoring-design.md. ONE open decision: which CSG kernel (csgrs prototype vs Manifold escalation), pending Harrison's dependency approval.
- In-repo content (2026-06-25, HLD): while the engine is co-developed the demo content lives in-repo under taste/ (tracked); moving it to a downstream game repo is deferred until the engine stabilizes.
- Placement boundary (2026-06-16, editor-design + HLD): the editor authors space, physical properties, and identity; the game owns behavior and per-instance config, bound by instance id or name. No property bag.
- Fog decoupled (43e7437): fog is a per-scene config with an explicit enabled flag (serde-default true); render distance is the scene's streaming extent (load_radius x chunk size), independent of fog.

## Verdict backlog (Harrison owes)

1. Editor friction from real authoring, once it is exercised by hand.
2. taste feel: prior air/jump verdicts SUPERSEDED by the reduction; revisit only if the minimal core needs tuning.
3. SHOW_RETICLE default (play-test question, carried from before).

## Parked ledger (deferred; build later)

Lifted from git history as each returns (built in the prior arc, then reset): undo/redo, multi-select (click/Ctrl/marquee + set ops + multi-field inspector), the ground-plane god-cam, object-mode movement.

Editor shell (built; editor-design.md has the locked shape). Still unbuilt: a rebindable keybind table, multi-chunk authoring (the cross-chunk engine bite above), gizmos, surface and grid snapping (now the W5 workflow), non-uniform scale in the inspector. Declined: a command palette (the menu + home-row verbs cover dispatch; a go-to finder is the fallback only if navigation at scale bites), a bottom dock, a separate inspector dock (the inspector floats; one dockable nav panel). Deferred: nav-panel width does not persist across restarts (the editor drives egui directly, not eframe).

Editor content surfaces (unbuilt; the view-fill work after the scene interaction): terrain sculpt + surface paint; lighting authoring (states, curves, fog/lighting zones, sky); content scan + integrity (the missing-assets queue for Ryan, dead-ref/orphan checks; pairs with the glTF loader). Prefab creation is the content-authoring suite.

Asset pipeline: glTF loader (real meshes; wok-mesh), asset binding (placeholder shape to final mesh by name).

Content-authoring suite (content-authoring-design.md): the multi-bite model/paint/rig/animate/socket track after the scene editor; engine prereqs deferred until the track starts (wok-mesh CSG kernel + tri-planar UV + height-to-normal; wok-render material path + GPU linear-blend skinning; wok-anim to planned; wok-scene prefab-model extension + glTF). HLD integration held until build-ready.

Publishing/packaging (parked): release game builds strip hot reload (feature-gated); baking authored JSON into a packed binary is deferred under earn-its-place.

Terrain/gravity scope (parked): the heightmap is optional per chunk (absent = no terrain), implement when a no-terrain level or terrain authoring needs it. Radial-gravity planet-walking is outside the current down-gravity model; game-composable over a gravity-vector primitive if a game wants a planetoid level.

Engine (parked; anchored in code or HLD): wok-content part 2 (streaming/eviction; criterion: worlds outgrow load-everything); wok-light bake (needs wok-physics raycasts) + dynamic pool; surface-tag friction (criterion: content wants ice/mud); swept ellipsoid, capsule prefab colliders, broadphase, swept terrain slide (wok-physics); the engine capsule locomotion path (pinned by locomotion_replay, no consumer); a Level-3 screenshot harness (wok-render); faded-item shadow/depth policies; wok-anim, wok-audio, wok-sequence crates. 120hz declined; SIM_HZ 60 locked.

## People and context

Harrison: founding backend engineer, builds wok-engine with brother Ryan (creative direction, assets via meshy.ai; the missing-assets scan becomes his queue). Game lineage: Ratchet & Clank / Jak / Sly / BFBB; working title context "Unstitched" (never in docs). The game itself is a future downstream repo; taste is the demo and feel laboratory, not the game. Harrison's setup: ZSA Voyager (left hand) plus mouse (right hand), which is why the editor is device-split and keyboard-first (left hand verbs and precision, right hand camera and big jumps, all focus-gated).
