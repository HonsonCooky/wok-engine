Global conventions (ASCII-only output, line width, philosophy) live in ~/.claude/CLAUDE.md, deployed from
dotfiles-windowsos.

# Project: wok-engine
This repo is the wok-engine: a modular monolith Rust workspace for building 3D games. Two layers, depending downward
only: the engine crates (wok-platform substrate at the bottom, wok-* libraries above it), and applications on the
engine. Two applications exist: the wok editor (co-developed in this workspace as the reference application) and the
game (a separate downstream project, not in this workspace). The engine never depends on an application.

Design canon lives in designs/. Read it before implementing; it is the source of truth for architecture and conventions:
- high-level-design.md: approach, principles, constituent crates, dependencies, data flow, validation strategy.
- project-canon.md: workflow, error handling (thiserror at crate boundaries), logging (tracing; warnings and errors go
  through Result, not logs), the determinism contract, and cross-crate decisions.
- multiplayer-model.md: the game-layer multiplayer approach (not engine architecture).

Documents in designs/ are not hard-wrapped: one line per paragraph or list item, regardless of the 120-character rule
above. Do not rewrap them.

If code and canon disagree, surface it rather than silently reconciling. Work proceeds crate by crate from scoped
briefs; defer back to the orchestrator when a problem exceeds the brief or is cross-cutting.

Rust in this repo is hand-formatted, not via cargo fmt: the committed baseline keeps intentional single-line
constructs and diverges from default rustfmt. Do not run cargo fmt - it reflows those lines and churns untouched files.
