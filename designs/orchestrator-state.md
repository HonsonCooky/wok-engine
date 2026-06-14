# wok-engine - Orchestrator State

Boot document for the design-orchestration conversation. Read this plus designs/high-level-design.md, designs/project-canon.md, and designs/editor-design.md and you hold the whole board. Updated at each work arc's close; one commit per update.

## Operating model

This conversation drafts copy-paste briefs for Claude Code sessions and reviews their reports; Harrison relays both directions and gives play verdicts. Brief format: header lines `[PASTE INTO: claude-code]`, `[RUN: <tier>]`, `[from: wok-orchestrator]`, then Context, numbered Scope, Outcome criteria (workspace green, counts stated, commit message), Out of scope. New scoped brief = new CC session (`/clear` first); immediate follow-ups to a session's own work stay in that session. CC reports come back as the final report block only, never full transcripts. The canonical docs are the cross-session memory; the orchestrator owns them and Harrison syncs them into designs/.

Model tiering (usage discipline): Tier 0 = no CC, Harrison edits by hand (single constants, tuning.json, content JSON). Tier 1 = sonnet or fable low (docs, cosmetics, one-file mechanical). Tier 2 = opus or fable medium (app-side multi-file features in wok or taste). Tier 3 = fable high (engine crates, cross-crate contracts, architecture, audits; include "read designs/ fresh" only at this tier). Fable is temporarily unavailable; until it returns, run Tier 1 on sonnet and Tier 2 and 3 on opus, raising the effort/thinking with the tier.

Conventions (enforced; full set in CLAUDE.md and project-canon.md): ASCII only, no em or double dashes in prose (divider comments exempt), files kept near a 400-line target and split at natural seams when one holds more than a single concern (a target, not a hard limit), hand-formatted with NO cargo fmt, no new dependencies without asking, commits direct to main with push on instruction, design docs are unwrapped single-line paragraphs.

## Repo state

Workspace: wok-engine, Windows, repo HonsonCooky/wok-engine, main. HEAD 4d8aef3. This arc rebuilt the editor (see designs/editor-design.md, added this arc) and reduced taste. Crates: wok-platform, wok-scene, wok-light, wok-physics, wok-mesh, wok-content (part 1 only), wok-render, plus apps wok (editor) and taste. wok tests 130; workspace green and clippy clean at HEAD. Boot caveat: this orchestration sandbox cannot read the working index (a Windows git index-extension mismatch), so read committed state via git log / git show / git diff A..B, never git status; uncommitted state comes from Harrison.

Engine, unchanged this arc: the authored loop (editor authors, hot reload both sides); forward renderer with banded lighting (authored smooth, 32 bands), fog, sky, screen-door fades, debug lines, single texel-snapped shadow map; collider family Aabb/Sphere/VertCylinder/Obb with classification. Dependency list and crate truths: see the HLD.

taste: reduced to a minimal feel core this arc (863311d, roughly 2000 lines dropped; the air, landing, precision, and slide-feel layers removed). The elaborate movement model the prior state doc described (parameterized jump, coyote, jump buffer, double jump, air steering) is gone; tuning.json now carries a small schema (move_speed, gravity, jump_velocity, max_jumps, camera params). taste is a minimal feel core now, not the elaborate feel lab.

Editor: the arc's main line (designs/editor-design.md; locked as a full content creator on a Zed shell grammar with tabbed edit contexts, built structure before features). Phase 1 (the action layer) is the spine and is most of the way in. Landed this arc, in order: the Action vocabulary became the single writer (3643ad3); undo/redo as snapshot history with drag coalescing (4df48a9); vim home-row camera keymaps, focus-gated, Ctrl-vertical (49f925e); input.rs split into input/{camera,viewport} then input/reposition with shared mod.rs (08413a1, 4d8aef3); selection became an ordered SelectionSet (8380fde); click-driven multi-select with Ctrl+click toggle, multi-aware tree/inspector, and set delete/duplicate one undo step each (d18472f); group-reposition drag that moves the whole set in one undo step (4d8aef3). The input scheme is captured in editor-design.md (vim home row, focus-gated; mouse: right-drag look, scroll speed, right-click menu; the click/marquee selection model). Still single-chunk; terrain and lighting load-only; prefabs placed not authored; the menu bar, command palette, docks, project model, and center tabs are unbuilt.

## In flight / next actions

1. NEXT BRIEF: marquee box-select (multi-select part 3 of 3; editor-design.md Input section). A left-drag from empty or unselected space draws a rect that selects the placements inside (plain = replace, Ctrl = extend); needs a rect pick query in wok/src/pick.rs, the marquee rect in UiState, and a SelectMany/extend action. The drag state machine's press/hold/release lifecycle was deliberately left in place for it. Then multi-field inspector editing (edit all selected, not just the primary).
2. AFTER multi-select: resume the editor build-path. Phase 1's menu bar + command palette + a rebindable keybind table (the vim scheme becomes its default; the mouse back/forward idea was dropped), then Phase 2 docks (the floating inspector moves into a right dock), Phase 3 project model (open folder, open recent), Phase 4 center tabs.
3. LOOSE END: designs/editor-design.md has an uncommitted Input section, carried unstaged across this arc. Commit it (with this state-doc refresh).
4. WATCH: input/viewport.rs and details.rs sit near the 400-line target; split at sensible seams as the marquee and menu work grow them.

## Verdict backlog (Harrison owes)

1. taste feel: the prior air/jump-feel and standing-jump-steerability verdicts are SUPERSEDED by the reduction; taste's feel is re-baselined to the minimal core, revisit only if it needs tuning.
2. Editor friction from real authoring (drag, rotation, now multi-select), once the editor is exercised by hand.
3. SHOW_RETICLE default (play-test question, carried from before).

## Resolved this arc

THE FORK is resolved: the editor rebuild (full content creator, Zed shell) became the next main line, settled in editor-design.md. The content-conventions + scan engine (the old standing lean) stays parked as a future main line; it fills the editor's reserved Scan page and feeds Ryan's missing-assets queue, and pairs with the glTF loader.

## Parked ledger (anchored in code or HLD)

No longer parked: undo/redo (built, 4df48a9); multi-select (in progress this arc); docks and command palette (now scheduled in the editor build-path). Still parked: wok-content part 2 (streaming, worker, eviction, eagerness enforcement; criterion: worlds outgrow load-everything). wok-light bake (needs wok-physics raycasts) and dynamic pool (needs renderer consumer). Content scan + conventions (the editor's Scan page, Ryan's queue, glTF pairing; a future main line). Surface-tag friction (wok-scene surface_at; criterion: content wants ice/mud). Swept ellipsoid, capsule prefab colliders, broadphase, swept terrain slide (wok-physics). Engine capsule locomotion path (supported API, no app consumer, pinned by locomotion_replay). glTF loader (wok-mesh; pairs with scan). Level 3 screenshot harness (wok-render). Rotation gizmos / drag-rotate, non-uniform scale, surface/grid snapping (editor). Faded-item shadow/depth policies. Camera near-plane transit of crates during arm recovery. normal_at widen-on-shimmer. Rim exponent. wok-anim, wok-audio, wok-sequence crates. 120hz declined; SIM_HZ 60 locked by verdict.

## People and context

Harrison: founding backend engineer, builds wok-engine with brother Ryan (creative direction, assets via meshy.ai; the missing-assets scan becomes his queue). Game lineage: Ratchet & Clank / Jak / Sly / BFBB; working title context "Unstitched" (never in docs). The game itself is a future downstream repo; taste is the demo and feel laboratory, not the game. Harrison's setup: ZSA Voyager (left hand) plus mouse (right hand), which is why the editor's input scheme is home-row and focus-gated.
