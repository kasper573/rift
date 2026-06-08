# CLAUDE.md

## General

- Dependencies: std only by default; exceptions must be approved.
- Correctness & clarity comes before performance.
- Tests assert on contracts, never implementation details.
- No mitigation fixes or hacks. Refactoring is encouraged: Don't hunt symptoms, fix root causes.
- Content is data bundled into the runtime.

## Architecture

- Server-authoritative: the server owns all state and the client sends intents and renders replicated state.
- `<projectRoot>/app`: Deployable artifacts
- `<projectRoot>/app/game`: The actual game. Contain all content and business logic.
- `<projectRoot>/lib/`: Reusable libraries. Entirely decoupled from /app/.

## Code style

- Prioritize simplicity, stability (extensible, not brittle), readability — then performance.
- small `macro_rules!` codegen is allowed where it removes boilerplate (see rift's `wire!`).
- Files read consumer-first: public API at top, private helpers at the bottom.
- No comments by default — code must be self-explanatory. A comment is only allowed for a
  non-obvious why (an external gotcha, a constraint the code can't express), never what/how.
- Never write comments that refer to prompt specific details as a way to communicate with the reviewer. Comments should be timeless and not rely on the reader being the person who prompted you to do some work.
- No inline tests: every test lives in its crate's `tests/` folder, against the public API.
- Use `Option`/`Result` and sum types over sentinels/casts. No `unsafe` without a justifying comment.
  Avoid `unwrap`/`panic!` off the test path unless an invariant is truly guaranteed.
- Newtype every float/int that carries a precise unit or id (`Seconds`, `Millis`, `NpcId`) — never
  semantic type aliases. The reader must not have to guess a unit, and the type replaces a comment.
  Plain primitives are fine only for obvious-to-everyone concepts (e.g. `health: f32`).
- Don't use #[must_use]. Only when clippy recommends it or when it's absolutely critical.

## Verify before done

- `cargo fmt` · `cargo clippy --all-targets` (no warnings) · `cargo test`
- `cargo build`
- Run benchmarks before and after your change. Compare and detect and fix performance regressions.
