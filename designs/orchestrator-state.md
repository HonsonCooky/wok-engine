# wok-engine - Orchestrator State

Boot document for the design-orchestration conversation. Read this plus designs/high-level-design.md and designs/project-canon.md and you hold the whole board. Updated at each work arc's close; one commit per update.

## Operating model

This conversation drafts copy-paste briefs for Claude Code sessions and reviews their reports; Harrison relays both directions and gives play verdicts. Brief format: header lines `[PASTE INTO: claude-code]`, `[RUN: <tier>]`, `[from: wok-orchestrator]`, then Context, numbered Scope, Outcome criteria (workspace green, counts stated, commit message), Out of scope. New scoped brief = new CC session (`/clear` first); immediate follow-ups to a session's own work stay in that session. CC reports come back as the final report block only, never full transcripts. The canonical docs are the cross-session memory; the orchestrator owns them and Harrison syncs them into designs/.

Model tiering (usage discipline): Tier 0 = no CC, Harrison edits by hand (single constants, tuning.json, content JSON). Tier 1 = sonnet or fable low (docs, cosmetics, one-file mechanical). Tier 2 = opus or fable medium (app-side multi-file features in wok or taste). Tier 3 = fable high (engine crates, cross-crate contracts, architecture, audits; include "read designs/ fresh" only at this tier).

Conventions (enforced; full set in CLAUDE.md and project-canon.md): ASCII only, no em or double dashes in prose (divider comments exempt), files under 400 lines, hand-formatted with NO cargo fmt, no new dependencies without asking, commits direct to main with push on instruction, design docs are unwrapped single-line paragraphs.

## Repo state

Workspace: wok-engine, Windows, repo HonsonCooky/wok-engine, main. HEAD edfceb4. Since the prior boot (e8c47bd): the HLD post-audit sync landed (fa5a225), a committed Claude Code permissions policy landed (46592e9), global conventions were deduped into ~/.claude/CLAUDE.md (edfceb4), and taste live tuning shipped (3c8976e). Test count: 666 green and clippy clean at e8c47bd, now stale; the tuning commit added a Tuning test module (18 tests) and the uncommitted prune below removes six, so restate the exact number from a fresh cargo test once the working tree is committed. Crates: wok-platform, wok-scene, wok-light, wok-physics, wok-mesh, wok-content (part 1 only), wok-render, plus applications wok (editor) and taste (demo and feel lab). Dependency list and crate truths: see the HLD, corrected against the 2026-06-12 repo audit.

What exists and works: full authored loop (editor authors, taste plays, hot reload both sides); forward renderer with banded lighting (authored smooth, 32 bands), fog, sky, screen-door fades, debug lines (tested and x-ray), single texel-snapped shadow map; collider family Aabb/Sphere/VertCylinder/Obb with classification; player is a flat-bottomed vertical cylinder (face-aware support up to 60 degrees, step-up policy, ledge overhang) drawn as the capsule bean (documented mismatch); movement model: ground accel+friction, air steering only (pure momentum, no airborne speed change), parameterized jump (apex height + time), fall gravity mult, coyote, jump buffer, double jump (vertical impulse; neutral stick = horizontal reset); camera: instant orbit, smoothed anchor, contains-gated arm clamp with recovery, floor reconciliation (the one world-writes-orbit exception), look-ahead framing scaled by cos(pitch), reticle; occlusion ghosting; three-mode hitbox overlay (F1: Faces/Visible/All); editor v2 complete (Zed-parity tree, floating details, place/delete/duplicate/rename, drag-to-move with Shift-vertical, full YXZ rotation fields, camera speed readout, status-bar pages with disabled Scan slot). Feel constants for movement and camera now live in a hot-reloadable tracked taste/tuning.json (Tuning struct, defaults are the shipped values, validation warnings, tests pin defaults), which moved the feel-tweak loop to Tier 0.

## In flight / next actions

1. LIVE (Tier 0): Harrison iterating movement and camera feel by hand in taste/tuning.json (move_speed currently 5, down from the shipped 7.5; uncommitted). When the numbers settle, a Tier 1 brief folds them into Tuning::default(). Verdict-backlog items 1 and 2 (air/jump feel, standing-jump steerability) ride on this pass.
2. UNCOMMITTED test prune in the working tree: six tautological or brittle tests removed across wok-light (point serde round-trip), wok-mesh (triangle_count), wok-scene (aabb_new_sets_fields), and wok (two page-state tests, one status-bar exact-string test). Unrelated to the feel tweak it currently shares the tree with. Commit it separately, re-verify green, restate the test count. Confirm provenance before committing if it was not a reviewed change.
3. THE FORK is open and unstarted: choose the next main line (candidates and standing lean in Verdict backlog item 5).

## Verdict backlog (Harrison owes; one play session covers most)

1. Air and jump feel: his settled numbers (the tuning file now exists; iteration in progress, see In flight 1).
2. Standing-jump steerability: pure-momentum air means a zero-speed jump cannot steer; contingency on record is a small get-moving floor (~2.5 m/s) ONLY if play demands it (pinned by a commented test in taste).
3. Editor friction from real authoring with drag and rotation.
4. SHOW_RETICLE default (currently on as the look-target indicator; play-test question).
5. THE FORK: next main line. Standing lean: the content-conventions + scan engine brief (the HLD's "content conventions and integrity" concern, the editor's disabled third page, the missing-assets queue for Ryan, pairs with the glTF loader). Alternatives: undo (criterion may trip soon), multi-chunk authoring, wok-content part 2, light bake.

## Parked ledger (all anchored in code or HLD; audit verified 2026-06-12)

wok-content part 2 (streaming, worker, eviction, eagerness enforcement; criterion: worlds outgrow load-everything). wok-light bake (needs wok-physics raycasts) and dynamic pool (needs renderer consumer). Content scan + conventions (criterion: called as main line; editor page slot reserved). Surface-tag friction (anchored at wok-scene surface_at; criterion: content wants ice/mud). Undo/redo (anchored at wok/src/model.rs; criterion: first session losing work; Delete and drag predicted first). Swept ellipsoid, capsule prefab colliders, broadphase, swept terrain slide (wok-physics lib.rs). Engine capsule locomotion path: supported API, no app consumer, pinned by locomotion_replay (documented). glTF loader (wok-mesh; pairs with scan). Level 3 screenshot harness (wok-render). Rotation gizmos / drag-rotate (editor v3). Multi-select, docking, command palette. Faded-item shadow/depth policies (eyeball verdicts pending). Camera near-plane transit of crates during arm recovery. normal_at widen-on-shimmer. Rim exponent. wok-anim, wok-audio, wok-sequence crates. 120hz declined; SIM_HZ 60 locked by verdict.

## People and context

Harrison: founding backend engineer, builds wok-engine with brother Ryan (creative direction, assets via meshy.ai; the missing-assets scan becomes his queue). Game lineage: Ratchet & Clank / Jak / Sly / BFBB; working title context "Unstitched" (never in docs). The game itself is a future downstream repo; taste is the demo and feel laboratory, not the game.
