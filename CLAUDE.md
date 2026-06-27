# CLAUDE.md

## General

- Terseness above all: This repo should contain only our business logic. Anything else should be outsourced to well established crates or services. Ie. we don't want to build an ECS, a graphics engine, a ui framework, a tiling engine, audio engine, etc. We want to build a game, and the code in this repo should reflect that.
- Correctness & clarity comes before performance.
- Tests assert on contracts, never implementation details.
- No mitigation fixes or hacks. Refactoring is encouraged: Don't hunt symptoms, fix root causes.
- No paintjobs. Think longterm when adding features. Again, refactoring is encouraged: Don't just layer code on top of code without thinking about the longterm design. Entropy is the enemy.
- Build once, run everywhere. The same binary (applies to all binaries in the repo) should be able to run in any environment. If assets or environment variables are changed the runtime should work anyway. (Note that runtimes may still panic or have degraded behavior if essential assets are missing)
- No hardcoded environment defaults: Panic if an env var is missing or invalid. Makes mistakes loud and obvious and forces environments to be well and explicitly configured. Also aids with the "build once, run everywhere" principle.
- While we currently deploy only to web for the forseeable future, the deploy target must still be abstracted away. Do not hard couple the codebase with any specific platform or environment. Ideally you only use abstractions provided by bevy and don't have to worry about this. But if you physically cannot avoid platform specific code, you must encapsulate it behind a single platform adapter so that it's easy to swap out the implementation for a different platform in the future.

## Code style

- Prioritize simplicity, stability (extensible, not brittle), readability — then performance.
- small, simple `macro_rules!` codegen is allowed to reduce boilerplate, but complex macros are entirely forbidden.
- Files read consumer-first: public API at top, private helpers at the bottom.
- No inline tests: every test lives in its crate's `tests/` folder, against the public API.
- Use `Option`/`Result` and sum types over sentinels/casts. No `unsafe` without a justifying comment.
  Avoid `unwrap`/`panic!` off the test path unless an invariant is truly guaranteed.
- Newtype every float/int that carries a precise unit or id (`Seconds`, `Millis`, `NpcId`) — never
  semantic type aliases. The reader must not have to guess a unit, and the type replaces a comment.
  Plain primitives are fine only for obvious-to-everyone concepts (e.g. `health: f32`).
- Don't use #[must_use]. Only when clippy recommends it or when it's absolutely critical.
- Use serde and envy for all json/env serialization and deserialization. No custom parsing code. And use the derive macros, not the imperative APIs.
- Aim for single source of truth (however do not conflate this with DRY. Code duplication is allowed and is not the same thing as SSoT).
- Any and all public type names must be intuitive and not ambigious if listed alongside other public types. Do not rely on crate namespacing to disambiguate. 

## Architecture

1. Each game crate's `src/` is organized into `core/`, `systems/`:

`core/`:
- code that may be reused by all systems
- typically low level systems and primitives
- may not depend on high level systems (not its own crate's `systems`, nor another crate's — e.g. the client's `core` must not touch `world::systems`)
- must be abstract and pluggable: systems integrate with core, core never reaches into a system. Never create a `systems::x` that mirrors a `core::x`. If core code seems to need a system, that's a sign core isn't abstract enough — make it extensible (traits, messages, registries, callbacks) and put the game-specific glue in the relevant feature.

`systems/`:
- high level systems and compositions of core primitives
- the majority of our game content and mechanics goes here
- may depend on other high level systems

2. The ui and bevy/* crates may not depend on other crates in this repo. They may depend on third party crates.

3. `world`'s `src/data/` is the content layer: one normalized table per file, each built with the `table!` macro the single source of truth for a table's `enum Id`, its `TABLE`, and `Id::get()`. Tables stay separate and reference each other only loosely by `data::*::Id`, never by embedding another table's rows (a row may still nest its own data). Table row structs live in core/* or systems/*, while `data/` only declares the rows. The idea is that the content layer can be swapped for a runtime loaded format in the future without too much hassle.

## Comments

- The default mindset should be: Do not write comments. Write code that is self explanatory.
- The only exception is: You need to explain WHY, not WHAT some code does. However, even then, you should consider refactoring the code so both the WHAT and the WHY becomes obvious. Only use comments as a final excape hatch.
- Never use comments as a way to give feedback to the prompter. This means comments should never refer to prompt specific details. Comments should be timeless and not rely on the reader being the person who prompted you to do some work.
- Don't scatter duplicate comments describing how a specific mechanism works all over the codebase. Keep it in one place, ideally at the implementation of that mechanism. A common source of this type of bad hygiene is re-explaining a mechanism in the workflow, in env files, in call sites, and finally also in the source code implementation of the mechanism.


## Verification

Before you start work on a task run benchmarks via `just bench` and save the results to a temporary file.

After you finish the task:

- `cargo fmt` · `cargo clippy --all-targets` (no warnings) · `cargo build` · `just e2e`
- Run benchmarks again and compare the results to the previous run. If there is a significant regression, investigate and fix it.