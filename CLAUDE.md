# Global Instructions
- Only use ASCII / US Standard Keyboard characters in all output. No emojis, no Unicode symbols, no special characters
  outside the printable ASCII range (0x20-0x7E).
- Never use double dashes (--) or em dashes in prose or comments. Use a single hyphen, a colon, or rewrite the sentence. Exception: ASCII divider comments in code (e.g. // ---- name ----) are formatting, not prose, and are allowed.
- Wrap prose in documents and comments at 120 characters. This is my preferred default line width.
- Never install new programs, packages, or dependencies without asking first.
# Philosophy
These directives encode how I think about software. Follow them when making suggestions, writing code, or recommending
tools and patterns.
- When suggesting a tool, pattern, or architecture, state the problem it solves and the constraint that makes it
  appropriate. If you cannot, do not suggest it.
- Do not recommend "best practices" that cannot be traced to a concrete mechanical reason. Convention alone is not
  justification.
- Explain the underlying ideas before the API surface. Labels are for communication; ideas are for understanding.
- Prefer fewer ideas composed together over named patterns memorized from convention. A solution built from understood
  ideas is more adaptable than one copied from a template.
- If the answer feels complex, consider whether the problem is being solved at the wrong layer of abstraction. The
  answer is usually simpler than the framework.
- Do not add configuration, tooling, or abstraction unless it solves a specific, identified problem. If a default works,
  leave it alone.
- When a conventional approach exists, explain the problem it was originally designed to solve. If my context differs,
  suggest an alternative built from the relevant ideas instead.
- Question inherited assumptions. If something "has always been done this way," that is not a reason to keep doing it.
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
