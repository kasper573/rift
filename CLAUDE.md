# CLAUDE.md

## General

- Terseness above all: This repo should contain only our business logic. Anything else should be outsourced to well established crates or services. Ie. we don't want to build an ECS, a graphics engine, a ui framework, a tiling engine, audio engine, etc. We want to build a game, and the code in this repo should reflect that.
- Correctness & clarity comes before performance.
- Tests assert on contracts, never implementation details.
- No mitigation fixes or hacks. Refactoring is encouraged: Don't hunt symptoms, fix root causes.
- Content is data in `assets/`: dev and servers read it from disk (hot-reloadable in the dev client); shipped clients embed it.

## Code style

- Prioritize simplicity, stability (extensible, not brittle), readability — then performance.
- small `macro_rules!` codegen is allowed where it removes boilerplate.
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

- `cargo fmt` · `cargo clippy --all-targets` (no warnings) · `cargo build`
- The E2E test green, locally and in CI: `cargo test -p client --test e2e` (needs `Xvfb`)
