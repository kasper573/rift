# e2e

End-to-end tests that drive the deployed stack through a real browser, register an account, and
assert on the pixels the game renders. Written in [Playwright](https://playwright.dev) so the same
tests run across Chrome, Firefox, Safari (WebKit), and Edge — locally and in CI.

## Running

```sh
just e2e            # build the wasm + server, bring the stack up, run the suite
just e2e portal     # only tests whose title matches "portal"
```

Locally this runs one combination — **chrome at desktop size, headless** — for a fast loop. It needs
Google Chrome; the stack runs on `rift.lan`, which Chrome maps to loopback via a launch flag, so
there's no DNS or `/etc/hosts` setup.

CI sets `E2E_ALL_BROWSERS=1` to fan out across every browser (chrome/edge/firefox/safari) at desktop
size. (A mobile-portrait resolution is wired up but commented out in the config for now.)

## The tests

Two histogram checks:

- **spawn** — register, wait for the world, assert the frame resembles the island.
- **portal** — register, click the island's warp tile, assert the frame becomes the forest.

"Which map is on screen" is a **color-histogram intersection** against the references in `snapshots/`
(`island.png`, `forest.png`): it answers "is this the same place" while tolerating the player's
position, animation, resolution, and a browser's rendering quirks — so one reference per map matches
across every browser, with no per-browser baselines.

The portal test clicks the warp by **world tile coordinate**, not screen pixels: the client exposes a
`click_world_tile(x, y)` hook (`game/client/src/testing.rs`) that issues exactly a player's click.
That keeps the test deterministic regardless of how slowly a browser renders — no locating the player
on screen, no mouse timing.

## Browsers and hardware

The game is WebGL2, and headless WebGL on Linux differs by engine:

- **Chrome / Edge** render headless with SwiftShader (the launch flags request it explicitly).
- **Firefox / WebKit** can't do WebGL headless, so they run **headed under a virtual display**
  (`xvfb-run`) with Mesa's software GL. An all-browser run is wrapped in `xvfb-run` (the Justfile does
  this when `E2E_ALL_BROWSERS=1`).

The suite runs at `workers: '50%'`, so it parallelizes with the runner's cores; `LP_NUM_THREADS=2`
(set in `just e2e-run`) caps each headed browser's render threads so parallel workers don't thrash.
Software-rendered WebGL is CPU-bound and the branch wasm is unoptimized, so the slow engines (Firefox,
WebKit) need real cores per worker — the standard 4-core GitHub runner is not enough for the full
matrix; run it on a runner with plenty of cores and memory bandwidth.
