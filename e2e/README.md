# e2e

End-to-end tests that drive the deployed stack through a real browser, register an account, and
assert on the pixels the game renders. Written in [Playwright](https://playwright.dev) so the same
tests run across Chrome, Firefox, Safari (WebKit), and Edge — locally and in CI.

## Running

The whole suite is one command from the repo root:

```sh
just e2e            # build the wasm + server, bring the stack up, run the tests
just e2e portal     # only tests whose title matches "portal"
```

Locally this runs a single combination — **chrome at desktop size, headless** — for a fast,
deterministic loop. It needs Google Chrome installed (Playwright drives the system browser) and
nothing else: the stack runs on `rift.lan`, which Chrome maps to loopback via a launch flag, so
there's no DNS or `/etc/hosts` setup.

CI sets `E2E_ALL_BROWSERS=1` to fan out across every browser × resolution
(chrome/edge/firefox/safari × desktop/landscape/portrait).

## Why the browsers run differently

The game is WebGL2, and headless WebGL on Linux differs by engine:

- **Chrome / Edge** render headless with SwiftShader (software WebGL2). Chrome 137+ dropped the
  automatic SwiftShader fallback, so the launch flags request it explicitly.
- **Firefox / WebKit** can't do WebGL headless on Linux, so they run **headed against a virtual
  display** (`xvfb-run`, preinstalled on the CI runners) with Mesa's software GL.

Because of WebKit/Firefox, an all-browser run must be wrapped in `xvfb-run` — the Justfile does this
when `E2E_ALL_BROWSERS=1`. A local chrome-only run needs none of it.

## Parallelism and hardware

The suite runs at `workers: '50%'` — half the runner's cores — so it parallelizes further as you give
it bigger hardware, with no config change. `just e2e-run` sets `LP_NUM_THREADS=2` so each headed
Firefox/WebKit caps its Mesa render threads; paired with 50% workers that keeps total render threads
near the core count instead of every browser grabbing them all and thrashing.

Software-rendered WebGL is CPU-bound, and the client wasm is large and unoptimized on branch builds,
so the slow engines (Firefox, WebKit especially) need **real cores per worker** to finish in time.
The standard 4-core GitHub-hosted runner is not enough for the full matrix; run it on a powerful
runner (a self-hosted machine, or a managed/large runner with plenty of cores and memory bandwidth)
and point the job's `runs-on` at it. Chrome and Edge are cheap enough to pass on modest hardware.

## How the assertions work

The tests resist cross-contamination by construction: each registers its own throwaway account, so
every run starts from a clean island spawn, and they aim with well-known canvas coordinates relative
to the player (the camera centers the player) rather than fragile DOM or template matching.

"Which map is on screen" is decided by a **color-histogram intersection** against the reference
snapshots in `snapshots/` (`island.png`, `forest.png`). It answers "is this the same place" while
tolerating the player's position, animation, resolution, and a browser's rendering quirks — so one
reference image per map matches across every browser, with no per-browser baselines to maintain.
