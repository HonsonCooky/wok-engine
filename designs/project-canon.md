# Project Canon

Workflow rules, working principles, and cross-crate decisions that
don't fit in the HLD or multiplayer model.

## Roles

**Orchestrator/designer** - Harrison plus a design-lead AI
conversation (typically Claude). Source of truth for the engine's
design. Owns the canonical documents (HLD, multiplayer model, this
canon). Drafts implementation handoff briefs directly.

**Implementer (Claude Code)** - separate sessions. Takes a brief
from the orchestrator and produces working code in checkpoint
cadence. Defers back when problems exceed the brief or when an
issue is cross-cutting.

There is no per-crate planner role. Orchestrator drafts briefs for
CC directly, scoped tightly enough that a single CC session can
absorb and execute them.

## Artifacts

- `high-level-design.md` - engine philosophy, principles, crates,
  dependency graph, data flow, validation.
- `multiplayer-model.md` - parallel-paths approach for games that
  opt into multiplayer. Game-design doc.
- This document - workflow, working principles, cross-crate
  decisions.

Handoff briefs and plan-vs-reality reflections live in the
conversation that produced them; not canonical.

## Handoff briefs

Orchestrator -> CC handoffs are scoped task assignments sized for
one CC session. A brief contains:

- **Scope** - the specific chunk of work; bounded for absorption
  without external context.
- **References** - pointers to canonical documents the brief draws
  from (typically HLD + this canon).
- **Outcome criteria** - what "done" looks like: tests passing,
  public API implemented, file structure created, expected behavior
  demonstrated.
- **Out-of-scope notes** - explicit boundaries on what NOT to
  build. CC interprets silence as license; explicit non-scope
  prevents this.

Harrison relays: orchestrator drafts here -> Harrison copies to CC
session -> CC executes and reports -> Harrison relays results back.

## Decision discipline

Load-bearing decisions go into canonical documents. Conversational
consensus is fragile; a fresh AI session will not remember anything
agreed verbally.

Test: would a fresh orchestrator session need this to do its job
correctly, AND is it not already in another canonical doc? If yes
to both, write it down.

## Working method

Design happens crate by crate; the HLD is a living anchor. HLD
updates only when *earned by discovery* (a forbidden edge was
missed; a primitive needs a type a crate doesn't expose), not by
preference. Preference-driven changes are churn.

For each design area, two lists: **locked decisions** and
**explicitly left-to-game**. Scheduling deferrals (e.g., wok-anim
deferred until placeholder animation needs more than transform
tweens) are locked decisions about scope. Revisit criteria attach
to locked decisions, not a third category.

## Compass

Mental model: **engine creates content; game connects content.** The
engine handles content creation infrastructure (primitives, asset
pipelines, scene and terrain authoring, deterministic math, cel
rendering) and inherits a small set of opinionated "only one
implementation" features (cel shading, 32-chunk cap, single shadow
map). Games compose content into specific gameplay, business logic,
story; they own entity state and simulation loops.

Sharpens HLD principle #5: when a design trade-off exists, choose
the option that lets a future game's session spend less context on
engine concerns. Game-specific systems being rebuilt per-game is
acceptable.

**Scope:** discrete-level AND open-world games, all from
deterministic content. Procedural world generation is out of scope.
Development: single laptop (initially a Dell XPS 14 9440),
two-person team with AI collaboration.

## Error handling

Crate-boundary error types use `thiserror`. Errors propagate through
`Result<T, E>` and `?`. Library crates do not use `anyhow`; the game
application may use `anyhow` internally.

Each crate defines its own error enum exposing only the failure
modes external consumers need to distinguish. Internal errors are
wrapped at the boundary so external consumers see a stable surface.

One `Error` enum per crate by default (e.g., `wok_scene::LoadError`,
`wok_mesh::MeshError`). Multiple narrow types in a single
crate are fine when genuinely independent (e.g., `LoadError` vs
`SliceError`); avoid one God-enum mixing unrelated failure domains.

## Logging

Logging uses `tracing`, structured with spans. Levels:

- `trace` - high-frequency / low-value detail (per-tick state,
  per-frame timing).
- `debug` - development-time visibility (chunk lifecycle, asset
  uploads, content scans).
- `info` - noteworthy state changes (scene loaded, chunk
  transitioned to loaded, hot reload applied).

Warnings and errors do NOT use logging. They surface through
`Result<T, E>`. If a library crate reaches for `tracing::warn!` or
`tracing::error!`, the failure mode should be in the error enum
instead.

The game application configures the tracing subscriber. Libraries
just emit events.

## Determinism contract

Required for HLD Section 4 Level 2 deterministic replay.

**Every engine crate:**

- No wall-clock reads. Time passed as `dt: f32` (or similar)
  parameters.
- Seeded RNG only. Seeds are inputs, never implicit.
- Asset loading does not affect simulation timing. Chunk
  transformations produce identical arrays from identical inputs.
- HashMap-based serialization sorts before write.

**wok-physics specifically:**

- Integration and collision functions are deterministic given the
  same inputs, `dt`, and build: identical input sequences reproduce
  bitwise. This is the property the Level 2 replay harness relies on.
- No parallel reductions in collision narrow-phase inner loops
  (order affects floating-point results).
- Position-independence: simulation runs in chunk-local coordinates,
  so the same actor inputs and chunk data give the same collision
  result regardless of the chunk's world position. In floating point
  that is exact for the qualitative result (contact axis and
  direction) and holds to float precision for magnitudes; chunk-local
  coordinates are what keep the error bounded. It is not bitwise
  across world positions, and we do not adopt fixed-point to make it
  so.

The game's simulation loop composes these primitives on a fixed
timestep to produce deterministic gameplay.

**Permitted parallelism:**

- Rendering inner loops (output not part of simulation state).
- Content processing (mesh generation, light baking - outputs
  byte-identical regardless of parallelization order when each work
  unit's output is independent of others' intermediate states).
- Cross-chunk work in wok-content (per-chunk transformation is
  internally sequential; across chunks is free to parallelize).

**Testing:** Level 2 harness (game-owned) exercises a scripted
input sequence over N simulation steps and compares dumped state to
a stored expected dump. Each crate also includes its own
determinism tests at Level 1.

## Cross-crate decisions

*(Currently empty. Entries land here when cross-crate decisions
emerge that don't fit elsewhere.)*
