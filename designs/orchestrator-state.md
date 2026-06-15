# wok-engine - Orchestrator State

Boot document for the design-orchestration conversation. Read this plus designs/high-level-design.md, designs/project-canon.md, and designs/editor-design.md and you hold the whole board. Updated at each work arc's close; one commit per update.

## Operating model

This conversation drafts copy-paste briefs for Claude Code sessions and reviews their reports; Harrison relays both directions and gives play verdicts. Brief format: header lines `[PASTE INTO: claude-code]`, `[RUN: <tier>]`, `[from: wok-orchestrator]`, then Context, numbered Scope, Outcome criteria (workspace green, counts stated, commit message), Out of scope. New scoped brief = new CC session (`/clear` first); immediate follow-ups to a session's own work stay in that session. CC reports come back as the final report block only, never full transcripts. The canonical docs are the cross-session memory; the orchestrator owns them and Harrison syncs them into designs/.

Model tiering (usage discipline): Tier 0 = no CC, Harrison edits by hand (single constants, tuning.json, content JSON). Tier 1 = sonnet or fable low (docs, cosmetics, one-file mechanical). Tier 2 = opus or fable medium (app-side multi-file features in wok or taste). Tier 3 = fable high (engine crates, cross-crate contracts, architecture, audits; include "read designs/ fresh" only at this tier). Fable is temporarily unavailable; until it returns, run Tier 1 on sonnet and Tier 2 and 3 on opus, raising the effort/thinking with the tier.

Conventions (enforced; full set in CLAUDE.md and project-canon.md): ASCII only, no em or double dashes in prose (divider comments exempt), files kept near a 400-line target and split at natural seams when one holds more than a single concern (a target, not a hard limit), hand-formatted with NO cargo fmt, no new dependencies without asking, commits direct to main with push on instruction, design docs are unwrapped single-line paragraphs.

## Repo state

Workspace: wok-engine, Windows, repo HonsonCooky/wok-engine, main. HEAD 6ba6e8f. wok tests around 155; workspace green and clippy clean at HEAD. The wok crate is lean: no dead code, no TODO markers, every file under the 400 target, modules cleanly split. Boot caveat: this sandbox cannot read the working index (a Windows git index-extension mismatch), so read committed state via git log / git show / git diff A..B, never git status; uncommitted state comes from Harrison (currently only taste/tuning.json, a long-carried hand-tune).

Crates: wok-platform (window, GPU, input, gamepads, audio-device, frame loop), wok-scene (chunked scene/prefab data, heightmaps, serde, hot-reload watcher, slicing), wok-physics (collision, sweeps, slide, gravity, camera orbit/spring-arm math), wok-mesh (MeshCpu/Gpu, primitive + terrain mesh gen, GPU upload), wok-light (lighting data model, curves, point lights), wok-content (chunk lifecycle part 1), wok-render (forward renderer: banded lighting 32 bands, single shadow map, fog, gradient sky, screen-door fades, debug lines, occlusion ghosting), plus apps wok (editor) and taste. Crate truths and the dependency graph: see the HLD.

taste: a minimal feel core (reduced 863311d; air/landing/precision/slide-feel layers removed). tuning.json schema is move_speed/gravity/jump_velocity/max_jumps/camera. The replay harness (Level 2) lives here. Not the elaborate feel lab anymore.

Editor (designs/editor-design.md; full content creator, Zed shell grammar, modal manipulator): the action-layer spine and the modal interaction model are in. Built across this arc: the Action vocabulary is the single writer (3643ad3); undo/redo as snapshot history with transform-run coalescing (4df48a9); selection is an ordered SelectionSet with click / Ctrl+click / marquee multi-select, set delete/duplicate/move as one undo step each, and a multi-field inspector that applies deltas to the whole set (8380fde, d18472f, 98533fe, 68c1c51); a Mode state machine (807d704) with Object mode (default; the orbit camera frames and locks onto the selection) and Free-fly mode (a ground-plane god-cam: WASD pan, Q/E altitude, right-drag look, 6ba6e8f), backtick to toggle; object-mode movement is keyboard-only (home-row grid-nudge via selection_ops; click-drag reposition retired, 302a1fd). The mouse is selection-only; the keyboard moves (object mode). Still single-chunk; terrain, lighting, and prefab internals are load-only, not authored; menu bar, command palette, docks, project model, and center tabs are unbuilt.

## In flight / next actions

STABILIZING (current arc): no new features; consolidate to a clean, committed, coherent baseline before content surfaces. Steps: commit or settle taste/tuning.json; confirm green and clippy at HEAD; this doc refreshed; park the deferrals (below). The wok crate is already lean (the earn-its-place prune mostly happened along the way: drag.rs/reposition.rs removed, files split, no dead code), so this is housekeeping, not surgery.

Next main line (after stable): the editor's content surfaces, the prefab creator first (where content is born; no editor support today), per editor-design.md's build path. The remaining shell pieces and the interaction polish (below) slot in around it.

## Verdict backlog (Harrison owes)

1. Editor friction from real authoring, once it is exercised by hand (the auto-orbit-vs-overview friction is the live one; see parked ledger).
2. taste feel: prior air/jump verdicts SUPERSEDED by the reduction; revisit only if the minimal core needs tuning.
3. SHOW_RETICLE default (play-test question, carried from before).

## Parked ledger (deferred; consolidate now, build later)

Done this arc, no longer parked: undo/redo, multi-select (click/Ctrl/marquee + set ops + multi-field inspector), the modal foundation (Object/Free-fly modes), the ground-plane god-cam, object-mode keyboard movement.

Editor interaction polish (deferred): auto-orbit fights the god overview - toggling to object mode yanks the camera off the wide shot; the fix is to drop the auto-orbit and make "frame the selection" an explicit key (this is the live friction). Left panel should be toggleable, and the viewport should not assume full width when it is open. Slice C: counts (a grid multiplier so coarse keyboard moves are not many taps) plus keyboard rotate/scale verbs (the inspector already does rotate/scale, so the keyboard verbs are only a fast path).

Editor content surfaces (unbuilt, the next main work): prefab creator (compose primitive shapes, named states, per-shape surface tag and hitbox/visible flags, mesh binding); terrain sculpt + surface paint; lighting authoring (light states, curves, fog/lighting zones, sky); content scan + integrity (the reserved Scan page, the missing-assets queue for Ryan, dead-ref/orphan checks; pairs with the glTF loader).

Editor shell (unbuilt): menu bar, command palette, rebindable keybind table, docks (inspector to a right dock, a bottom dock), center tabs, project model (open folder, open recent, many scenes), multi-chunk authoring, gizmos, surface/grid snapping, non-uniform scale in the inspector.

Asset pipeline: glTF loader (real meshes; wok-mesh), asset binding (placeholder shape to final mesh by name).

Engine (parked; anchored in code or HLD): wok-content part 2 (streaming, worker, eviction, eagerness; criterion: worlds outgrow load-everything); wok-light bake (needs wok-physics raycasts) + dynamic pool (needs renderer consumer); surface-tag friction (wok-scene surface_at; criterion: content wants ice/mud); swept ellipsoid, capsule prefab colliders, broadphase, swept terrain slide (wok-physics); engine capsule locomotion path (supported API, no consumer, pinned by locomotion_replay); Level 3 screenshot harness (wok-render); faded-item shadow/depth policies; camera near-plane transit during arm recovery; normal_at widen-on-shimmer; rim exponent; wok-anim, wok-audio, wok-sequence crates. 120hz declined; SIM_HZ 60 locked by verdict.

## People and context

Harrison: founding backend engineer, builds wok-engine with brother Ryan (creative direction, assets via meshy.ai; the missing-assets scan becomes his queue). Game lineage: Ratchet & Clank / Jak / Sly / BFBB; working title context "Unstitched" (never in docs). The game itself is a future downstream repo; taste is the demo and feel laboratory, not the game. Harrison's setup: ZSA Voyager (left hand) plus mouse (right hand), which is why the editor is modal and keyboard-driven (object-mode home-row nudge, free-fly WASD, all focus-gated).
