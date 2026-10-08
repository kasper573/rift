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
- A folder may never contain only one file, with the exception of common crate root folders like `src`, `static`, `assets`, `templates`, etc. 
- Avoid the use of #[cfg]. Only reach for it when it's absolutely necessary, and keep the usage count as low as possible.

## Architecture

1. The game crates `src/` is organized into `core/`, `systems/`, `data/`:

`core/`:
- code that may be reused by all systems
- typically low level systems and primitives
- may not depend on high level systems
- must be abstract and pluggable: systems integrate with core, core never reaches into a system. Never create a `systems::x` that mirrors a `core::x`. If core code seems to need a system, that's a sign core isn't abstract enough — make it extensible (traits, messages, registries, callbacks) and put the game-specific glue in the relevant feature.

`systems/`:
- high level systems and compositions of core primitives
- the majority of our game mechanics goes here
- may depend on other high level systems

`src/data/`: is the content layer: one normalized table per file, each built with the `table!` macro the single source of truth for a table's `Id`, its `TABLE`, and `Id::get()`. Tables stay separate and reference each other only loosely by `data::*::Id`, never by embedding another table's rows (a row may still nest its own data). Table row structs live in core/* or systems/*, while `data/` only declares the rows. The idea is that the content layer can be swapped for a runtime loaded format in the future without too much hassle.

Should contain no business logic, only data.

Code outside `data/` must not depend on arbitrary game content, so the same server and client can run any game's data. `table!` enforces this:
- A table's `Id` type, `TABLE`, `Id::VARIANTS` and `Id::get()` are usable everywhere, but its rows are hidden: only `data/` can name them (`data::npc::Id::Mara`).
- Mark a row `#[expose]` when it stands for a generic system or mechanic (an input action, an interface sound, a quest marker), or put `#![expose]` at the top of a table whose rows all do. Never expose a piece of content (a character, an item, a quest). Exposed rows are the game's core data requirements: every game's data must provide them.
- When code needs content for a role, `data/` exports a designation instead (`data::area::SPAWN_ID`).
- Integration tests look hidden rows up by name.
- Generic assets of systems (cursors, window icons) may be referenced directly; another game reskins them by swapping asset files.

2. The ui and bevy/* crates may not depend on other crates in this repo. They may depend on third party crates.

## Comments

- The default mindset should be: Do not write comments. Write code that is self explanatory.
- The only exception is: You need to explain WHY, not WHAT some code does. However, even then, you should consider refactoring the code so both the WHAT and the WHY becomes obvious. Only use comments as a final excape hatch.
- Never use comments as a way to give feedback to the prompter. This means comments should never refer to prompt specific details. Comments should be timeless and not rely on the reader being the person who prompted you to do some work.
- Don't scatter duplicate comments describing how a specific mechanism works all over the codebase. Keep it in one place, ideally at the implementation of that mechanism. A common source of this type of bad hygiene is re-explaining a mechanism in the workflow, in env files, in call sites, and finally also in the source code implementation of the mechanism.

## Content creation guidelines

- No omniscient writing. Content only shows the player what their character could know at that point. A locked option the player has no reason to know about yet stays hidden until it unlocks, and a choice never spells out consequences the character couldn't foresee. The systems allow revealing more; the content chooses not to.
- Show a locked or priced option only when the scene sets it up, so the NPC's words match what the options reveal. A trader laying out offers that each come at a price is the typical case: "I've got a few options for you, but each comes with its own set of challenges. Pick your poison:".
- The player character has no bust and no voice, and never speaks a line. The player speaks only by picking dialogue choices, so anything their character says belongs in a choice's label. Busts and voices belong to NPCs, and a scene may stage several NPCs at once.
- Audio is balanced for the default settings, where every volume slider sits at 50% and plays content exactly at its authored level. At those defaults every area must sound just right: music, ambience, effects and voice each fit into the mix without muddling over each other. Voice is the anchor (babble at about -23 LUFS over its loudest 100 ms). Music sits under it (about -30 LUFS integrated), ambience under the music (about -33 to -37 LUFS), and effects by importance: combat and key moments level with the voice, items and interface below it, footsteps lowest. No sound may peak above -12 dBFS at the defaults, so players can turn any slider up to 100% (+6 dB) or down without the mix clipping.


## Demo videos

Everything you build gets shown to the user on video, from a player's point of view, against the real stack.

- The showcase is `e2e/demos/`: one chapter per file, played in file-name order. A chapter is a Playwright test built with `chapter()` and narrated with `caption()` (`e2e/helpers/demo.ts`). It plays the game with real mouse and keyboard input, finding what to click through the client's probe of the world and UI (`e2e/helpers/game.ts`: `travelTo`, `walkTo`, `pickUp`, `clickUi`, …). Never hardcode screen positions — monsters wander and windows move. When a chapter needs to see something the probe doesn't report, extend the probe (`game/src/bin/client/probe.rs`).
- After every change, show it in the chapter it belongs to: extend that chapter, or add a numbered file where it fits the story. Record with `just demo <chapter>`, check the result with `just contact-sheet target/demo/<chapter>.webm`, and share the video path(s) with the user when you report back. This is not optional. When a change has nothing new to show (a refactor, a fix with no visible effect), record the chapters covering what it touched to show they still work.
- Chapters show the game's mechanics, never its content. New or changed content (maps, areas, actors, busts, items, dialogue) is reviewed in one-off recordings shared with the user, which are never committed as chapters. Content that brings no new mechanic adds nothing to the showcase.
- A change to the `ui` crate on its own is shown in its component gallery: `just gallery-demo "<scene>" [seconds] [xdotool input]` records `target/demo/gallery-<scene>.webm`.
- The full showcase, `just showcase` → `target/demo/showcase.webm`, is only recorded when the user explicitly asks for it. It is every chapter stitched in order, so keeping the chapters current is what keeps it complete: never demo a feature outside a chapter.

## Verification

Before you start work on a task run benchmarks via `just bench` and save the results to a temporary file.

After you finish the task:

- `cargo fmt` · `cargo clippy --all-targets` (no warnings) · `cargo build` · `just e2e`
- Run benchmarks again and compare the results to the previous run. If there is a significant regression, investigate and fix it.