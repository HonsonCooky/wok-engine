# Project Canon

Project-level decisions that don't fit naturally into per-crate plans
or the high-level design. Two kinds of content live here: workflow
and orchestration rules, and cross-crate decisions or compass framings
without a more natural home.

If a decision belongs in a single crate's plan, it goes there. If it
adds or changes an engine principle, update the high-level design.
This doc is for the residue — decisions that span the project but
aren't naturally homed elsewhere.

## Roles

**Orchestrators** — Harrison plus the design-lead AI conversation.
Cross-crate decisions, handoff briefs, plan reviews, final
validation. Maintain canonical documents.

**Per-crate planners** — separate AI conversations, one per crate.
Own only their crate's detailed plan. Receive a handoff brief from
orchestrators; deliver v1; iterate with orchestrators; final plan
hands to implementation. They trust the orchestrators on cross-crate
constraints; they do not re-litigate them.

**Implementers (Claude Code)** — separate sessions. Take a finalized
per-crate plan and produce working code in checkpoint cadence. Defer
back to a planner conversation when problems exceed the plan's
coverage. Defer to orchestrators when an issue is cross-cutting.

## Artifacts

- `high-level-design.md` — engine philosophy, dependency graph,
  principles.
- `multiplayer-model.md` — the parallel-paths approach. Game-design
  doc, not engine architecture.
- Per-crate plans (`wok-<crate>-plan.md`) — detailed plan for each
  `wok-*` crate.
- Handoff briefs — orchestrator's prompt to a per-crate planner.
  The contract for what to take as given vs what to figure out.
- Plan-vs-reality memos — at each phase boundary in implementation,
  a memo comparing shipped code to the plan. Backwards-looking ones
  may be added retroactively where useful.
- This document.

## Cross-conversation protocol

Conversations identify with tags: `wok-content-planner`,
`wok-physics-planner`, `claude-code`, and so on. New tags are coined
as new conversations begin.

Prompts from one conversation to another are copy-pasteable code
blocks containing a routing header (`[PASTE INTO: <tag>]`) and a
reply instruction (`[from: <tag>]` at the top of the response, on
its own line). Harrison is the human relay; the headers let the
orchestrator conversation identify replies without Harrison labeling
them each time.

## Decision discipline

If a decision is load-bearing for downstream work and doesn't already
live in a canonical document, write it into one. Conversational
consensus is fragile — a fresh AI session restarted from canon will
not remember anything agreed verbally. The "if it's load-bearing, it
gets written" rule protects against drift.

Be intentional. Not every decision needs explicit mention. The test
is "would a fresh orchestrator session need this to do its job
correctly, AND is it not already in another canonical doc?" If yes
to both, it lands somewhere.

## Compass

The engine exists so that Claude Code and Harrison can build games
together. The mental model: **engine creates content; game connects
content.** The engine handles content creation infrastructure —
primitives, asset pipelines, scene and terrain authoring,
deterministic simulation, cel rendering — and inherits a small set of
opinionated "only one implementation" features that emerge from its
constraints (cel shading is the visual style, 32-chunk cap, single
shadow map). Games compose that content into specific gameplay,
business logic, and story; they don't redo what the engine does.

This sharpens "primitives, not features" (high-level design): when a
design trade-off exists, choose the option that lets a future game's
session spend less context on engine concerns. Game-specific systems
being rebuilt per-game is acceptable.

**Scope:** the engine targets discrete-level AND open-world games, all
built from deterministic content. Procedural world generation is out
of scope — that's a different engine. Development setup: a single
laptop (initially a Dell XPS 14 9440) and a two-person team with AI
collaboration.

## Error handling

Crate-boundary error types use `thiserror`. Errors propagate through
`Result<T, E>` and `?`. Library crates do not use `anyhow`; the game
application may use `anyhow` internally if it wants.

Each crate defines its own error enum exposing only the failure modes
external consumers need to distinguish. Internal errors are wrapped
or converted at the boundary so external consumers see a stable
surface even when internal implementation changes.

Pattern: one `Error` enum per crate (e.g., `wok_scene::LoadError`,
`wok_content::RegistryError`), each variant carrying the context the
caller needs to handle or report the failure. Multiple narrow error
types in a single crate are fine when they're genuinely independent
(e.g., `LoadError` vs `SliceError` in wok-scene); avoid one God-enum
that mixes unrelated failure domains.

## Logging

Logging uses the `tracing` crate, structured with spans. Levels
follow tracing's conventions:

- `trace` — high-frequency or low-value detail (per-tick state,
  per-frame timing).
- `debug` — development-time visibility (chunk lifecycle events,
  asset uploads, registry mutations).
- `info` — noteworthy state changes (scene loaded, snapshot
  captured, hot reload applied).

Warnings and errors do NOT use logging. They surface through
`Result<T, E>` and propagate to the caller. If a library crate finds
itself reaching for `tracing::warn!` or `tracing::error!`, that's a
signal the failure mode should be in the error enum instead.

The game application configures the tracing subscriber and decides
what to do with events (write to file, print to console, ship to
telemetry). Libraries just emit events; they don't configure
subscribers themselves.

## Determinism contract

The Level 2 deterministic replay harness (HLD §4) requires every
engine crate to honor a determinism contract. Centralized here so
future sessions have one reference rather than reconstructing it from
scattered notes across crate plans.

**Required of every engine crate:**

- Simulation must not read wall-clock time. Time is passed as
  `dt: f32` (or similar) parameters.
- Randomness must use seeded RNG. Seeds are inputs, never implicit.
- Asset loading does not affect simulation timing. Chunk
  transformations produce identical arrays from identical inputs,
  regardless of when or where they load.
- HashMap-based serialization sorts before write. HashMap iteration
  order is nondeterministic; any output destined for disk or
  cross-client comparison must impose a stable order.

**Required of wok-physics specifically:**

- Fixed-timestep integration. Integration step parameterized by `dt`;
  never reads wall-clock.
- No parallel reductions in inner loops (collision narrow-phase,
  integration). Order of accumulation affects floating-point results.
- Position-independence holds: same actor inputs + same chunk data
  produce the same trajectory regardless of the chunk's world
  position.

**Permitted parallelism (does not break determinism):**

- Rendering inner loops. Output is not part of simulation state.
- Content processing (mesh generation, light baking). Output is
  byte-identical regardless of parallelization order when implemented
  correctly (each work unit's output is independent of others'
  intermediate states).
- Cross-chunk work in wok-content (when parallelizing `slice_chunk`
  across chunks). Per-chunk slicing is internally sequential; across
  chunks is free to parallelize.

**Testing:** the Level 2 harness exercises a scripted input sequence
over N simulation steps and compares dumped state to a stored
expected dump. Any crate breaking the contract surfaces as a Level 2
test failure. Each crate also includes its own determinism tests at
Level 1 against narrower fixtures.

## Cross-crate decisions

*(Currently empty. Entries land here as cross-crate decisions emerge
from orchestration work that don't fit naturally into the sections
above.)*
