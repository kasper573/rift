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
- Game mechanics on the server draw randomness only from their world's `Rng` resource, seeded by the server's `rng_seed` (drawn and logged when unset), so a seeded world plays out alike from run to run. All other randomness stays unseeded: secrets such as sign-in tickets come from the OS, and clients draw as they please. Benchmarks and showcases set a seed; tests run on a random one, and must hold on any.
- Don't use #[must_use]. Only when clippy recommends it or when it's absolutely critical.
- Use serde and envy for all json/env serialization and deserialization. No custom parsing code. And use the derive macros, not the imperative APIs.
- Data crosses JSON only through its Rust type. Code that knows a field's name must hold the type that declares it: never read or write `serde_json::Value` by literal key (`value["field"]`, `.get("field")`, `json!({...})` built in place of a struct), never branch on a serde tag string, never rebuild a type's file layout by hand. An untyped path compiles against whatever the files happen to look like today, so when the type changes it silently reads nothing and writes files the loader refuses.
- Content is read and written only as its engine type, through the same code the loader uses: `FolderReader` (in the editor `Editor::read::<Row>` and `write_content`), which parses fields, held data and bundle parts, and writes them back the way the game does. Bundle part paths come from the type's `Bundle` and `PartSpec`.
- `serde_json::Value` is for data whose shape is genuinely unknown at compile time: the schema-driven forms, whose keys come from the JSON Schema they walk, and raw-JSON fallbacks. Needing a field by name there means the code needs the type instead.
- When a type can't hold what a tool needs (a blueprint's unfilled holes, a draft mid-edit), extend the engine type to hold it (as `Fill<T>` does) rather than dropping to `Value`. `just lint` refuses literal-key JSON access outside the schema-driven forms.
- Aim for single source of truth (however do not conflate this with DRY. Code duplication is allowed and is not the same thing as SSoT).
- Any and all public type names must be intuitive and not ambigious if listed alongside other public types. Do not rely on crate namespacing to disambiguate. 
- A folder may never contain only one file, with the exception of common crate root folders like `src`, `static`, `assets`, `templates`, etc. 
- Avoid the use of #[cfg]. Only reach for it when it's absolutely necessary, and keep the usage count as low as possible.

## Architecture

1. The engine crate's `src/` is organized into `core/` and `systems/`; its content is not code, but a game folder read at runtime:

`core/`:
- code that may be reused by all systems
- typically low level systems and primitives
- may not depend on high level systems
- must be abstract and pluggable: systems integrate with core, core never reaches into a system. Never create a `systems::x` that mirrors a `core::x`. If core code seems to need a system, that's a sign core isn't abstract enough — make it extensible (traits, messages, registries, callbacks) and put the game-specific glue in the relevant feature.

`systems/`:
- high level systems and compositions of core primitives
- the majority of our game mechanics goes here
- may depend on other high level systems

The content layer is a game folder (`example_game/` is the example game): folders arranged however the game likes, in which every file says what it is by its name, `<name>.<type>.json`, or a bundle folder `<name>.<type>/` holding its parts (an area's scene and grids, a tileset's image). The game's settings are `fixtures/game.config.json`, the engine's own content lives at fixed paths under `fixtures/`, and media and notes sit anywhere. A content type implements `ContentFile` (its `TABLE`, and `check` for the rules a schema can't state), a type with parts also `Bundle`, and is registered in `systems::registry()`. A reference names a file by its path in the game folder; loading resolves every path to a `ContentId`, newtyped per type (`ItemId`, `NpcId`, …) and valid only within the `Content` it was resolved against. Content files stay separate and reference each other only loosely by path, never by embedding another file's content (a file may still nest its own data). Content types live in core/* or systems/*.

Code must not depend on arbitrary game content, so the same server and client can run any game's data:
- Code never names a content file. A file that stands for a generic system or mechanic (an input action, an interface sound, a quest marker) is a `Fixture`: an enum whose variants are files every game must provide at their place under `fixtures/`, checked at load. Never make a piece of content (a character, an item, a quest) a fixture.
- When code needs content for a role, the content says which file plays it: the game's settings (`GameSettings::initial_spawn`), or the file the role belongs to (a surface's `footstep`).
- Only content files name media; another game reskins with its own files.
- Integration tests name content by its path.

2. The ui and bevy/* crates may not depend on other crates in this repo. They may depend on third party crates.

3. The ui crate is the interface the editor and the game share: its look and every general-purpose egui component (fields, chips, cards, panels, menus, dialogs, floating labels, painted patterns, wording helpers). A component that isn't about one mechanic or one editor view lives there, so the game and the editor never draw the same thing two ways; a crate's own UI code keeps only what is about its domain. The editor and the engine reach egui and its extension crates only through ui's re-exports (`ui::egui`, `ui::egui_dock`, …), so they always build against the versions ui is built on; bevy_egui stays their own dependency, as the host that runs egui inside bevy.

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


## Showcase videos

Everything you build gets shown to the user on video, from a player's point of view, against the real game server.

- The showcase is the `showcase` crate: one chapter per file in `showcase/src/chapters/`, played in the order of `CHAPTERS` (`showcase/src/chapters/mod.rs`) and narrated with `caption()` (`showcase/src/shoot.rs`). A chapter plays through the `driver` crate: real mouse and keyboard input through the game's own bindings, finding what to click through the client's probe of the world and UI (`walk_to`, `travel_to`, `talk_to`, `fight`, `click_ui`, …). Never hardcode screen positions — monsters wander and windows move. Never roam or search either: a chapter heads straight for where what it needs is (`resident_home`, `hunting_ground`, markers) and asserts it's there. When a chapter needs to see something the probe doesn't report, extend the probe (`engine/src/systems/probe.rs`).
- After every change, show it in the chapter it belongs to: extend that chapter, or add one where it fits the story. Record with `just showcase <chapter>…`, check the result with `just contact-sheet target/showcase/<NN-chapter>.mp4`, and share the video path(s) with the user when you report back. This is not optional. When a change has nothing new to show (a refactor, a fix with no visible effect), record the chapters covering what it touched to show they still work.
- Chapters show the game's mechanics, never its content. New or changed content (maps, areas, actors, busts, items, dialogue) is reviewed in one-off recordings shared with the user, which are never committed as chapters. Content that brings no new mechanic adds nothing to the showcase.
- The full showcase, `just showcase` with no chapter named → `target/showcase/showcase.mp4`, is only recorded when the user explicitly asks for it. It is every chapter stitched in order, so keeping the chapters current is what keeps it complete: never show a feature outside a chapter.

## Tests

- The suite is a set of contracts: each test states a scenario we want to support and asserts it through what a player, a designer or the loader sees and does (a mechanic's outcome, what lands in the files, what a view shows, what the loader accepts or refuses). Never through a helper, an intermediate state or the code's structure, which a refactor must be free to change.
- Tests are added economically, as entropy is the enemy here as everywhere. A scenario gets one test; its variations are cases of that test, not copies of it. A bug fix earns a test only when it reveals a scenario no test states yet; otherwise it extends the test that owns the scenario, or adds nothing. Never add tests for coverage, one per fix or per function. A test that duplicates another, pins an implementation detail or asserts something trivial is a cost, not a safeguard: delete it.
- `just test` is unit and integration tests only, each isolated and fast: a test pulls in no more code than the slice of mechanics it verifies. Never start a game server, a client, a window or the docker stack in a test, and never drive one through the `driver` crate; that is the showcase's and the e2e's alone.
- Tests never read a shipped game (`example_game/`, the editor's templates). A test declares the small game it needs in code with `testkit::Game`, only the files it reads, and loads it through the real loader; `engine_rows()` and `whole()` exist for the few tests about the whole game contract. Shipped games are checked by `just lint`.
- Mechanics run in `testkit::Sim`: a bare world holding the test's game, stepped through only the real engine systems the test adds, in the server's order, with the clock advanced by the test rather than waited on. Never reimplement or stub a system to make a test pass. When a mechanic can't be exercised without the whole server or client, it isn't modular enough: refactor it until it can.
- The Playwright suite (`e2e/`) tests the whole stack from a browser: the website, and whatever else a browser is served. Gameplay gets exactly one smoke test there, that a signed-up player appears in the game.

## Builds on this machine

- Run every cargo build or test inside a memory-capped scope so a runaway compile kills only itself, never the session that spawned it: `systemd-run --user --scope -q -p MemoryMax=22G -p MemorySwapMax=0 cargo …` (the recipes in the Justfile run plain cargo; wrap the `just` call the same way). A process that exceeds the cap dies with "Killed"; rerun it with fewer jobs (`CARGO_BUILD_JOBS=8`) rather than lifting the cap. Never run two cargo invocations at once.
- Clean up unused cargo build output: `target/` grows by tens of gigabytes across profiles and feature sets and the disk is shared with everything else here. After a task, `cargo clean -p <crate>` for crates whose artifacts are stale, or `cargo clean` outright when `du -sh target` passes 50 GB.

## Verification

Before you start work on a task run benchmarks via `just bench` and save the results to a temporary file.

After you finish the task:

- `cargo fmt` · `cargo clippy --all-targets` (no warnings) · `cargo build` · `just test` · `just e2e`
- Run benchmarks again and compare the results to the previous run. If there is a significant regression, investigate and fix it.