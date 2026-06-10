# CLAUDE.md

## Mission: tersify

Shrink this repo to the minimum first-party code that produces an identical player experience.

- INVARIANT: player-identical experience — gameplay, graphics, audio, feel, and perceptible
  performance (load times, framerate, latency). Unsure if something is player-perceivable? It is.
- End state: exactly four first-party crates — `game/server`, `game/client`, `game/world`,
  `website` — plus assets (convert freely), a terse E2E suite, and minimal infra config.
- Caddy, Keycloak, and the LGTM stack stay (reconfigure freely).
- Everything an established, actively maintained crate can do, that crate does. Only genuine
  game logic and content earns a place here. Prefer one ecosystem's idiomatic companions.
- Metaprogramming lives inside Rust: macros fine; zero build.rs, zero codegen scripts.
- Depend, don't vendor.
- Verification is E2E plus real play, nothing else. Delete internal unit/integration tests.
- Strangle, don't big-bang: E2E green before every commit; commit messages name what was
  deleted and the first-party LOC delta (tokei, excluding assets).

## General

- Correctness & clarity comes before performance.
- Tests assert on contracts, never implementation details.
- No mitigation fixes or hacks. Refactoring is encouraged: Don't hunt symptoms, fix root causes.
- Content is data bundled into the runtime.

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
- The E2E suite green, locally and in CI:

  ```sh
  cargo build --release -p website -p server && cargo wasm
  docker compose -f docker/docker-compose.yaml --profile test up -d --build --wait
  cargo test -p server --features stack -- --test-threads=1
  ```
