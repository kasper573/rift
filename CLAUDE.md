# CLAUDE.md

## General

- Terseness above all: This repo should contain only our business logic. Anything else should be outsourced to well established crates or services. Ie. we don't want to build an ECS, a graphics engine, a ui framework, a tiling engine, audio engine, etc. We want to build a game, and the code in this repo should reflect that.
- Correctness & clarity comes before performance.
- Tests assert on contracts, never implementation details.
- No mitigation fixes or hacks. Refactoring is encouraged: Don't hunt symptoms, fix root causes.
- No paintjobs. Think longterm when adding features. Again, refactoring is encouraged: Don't just layer code on top of code without thinking about the longterm design. Entropy is the enemy.
- Build once, run everywhere. The same binary (applies to all binaries in the repo) should be able to run in any environment. If assets or environment variables are changed the runtime should work anyway. (Note that runtimes may still panic or have degraded behavior if essential assets are missing)
- No hardcoded environment defaults: Panic if an env var is missing or invalid. Makes mistakes loud and obvious and forces environments to be well and explicitly configured. Also aids with the "build once, run everywhere" principle.

## Code style

- Prioritize simplicity, stability (extensible, not brittle), readability — then performance.
- small `macro_rules!` codegen is allowed where it removes boilerplate.
- Files read consumer-first: public API at top, private helpers at the bottom.
- No inline tests: every test lives in its crate's `tests/` folder, against the public API.
- Use `Option`/`Result` and sum types over sentinels/casts. No `unsafe` without a justifying comment.
  Avoid `unwrap`/`panic!` off the test path unless an invariant is truly guaranteed.
- Newtype every float/int that carries a precise unit or id (`Seconds`, `Millis`, `NpcId`) — never
  semantic type aliases. The reader must not have to guess a unit, and the type replaces a comment.
  Plain primitives are fine only for obvious-to-everyone concepts (e.g. `health: f32`).
- Don't use #[must_use]. Only when clippy recommends it or when it's absolutely critical.
- Use serde and envy for all json/env serialization and deserialization. No custom parsing code. And use the derive macros, not the imperative APIs.

## Comments

- The default mindset should be: Do not write comments. Write code that is self explanatory.
- The only exception is: You need to explain WHY, not WHAT some code does.
- However, even then, you should consider refactoring the code so both the WHAT and the WHY becomes obvious.
- Only use comments as a final excape hatch.
- Never use comments as a way to give feedback to the prompter. This means comments should never refer to prompt specific details. Comments should be timeless and not rely on the reader being the person who prompted you to do some work.
- Don't scatter comments describing how a specific mechanism works all over the codebase. Keep it in one place, ideally at the implementation of that mechanism. A common source of this type of bad hygiene is re-explaining a mechanism in the workflow, in env files, in call sites, and finally also in the source code implementation of the mechanism.


## Verification

Before you start work on a task run benchmarks via `just bench` and save the results to a temporary file.

After you finish the task:

- `cargo fmt` · `cargo clippy --all-targets` (no warnings) · `cargo build` · `just e2e`
- Run benchmarks again and compare the results to the previous run. If there is a significant regression, investigate and fix it.