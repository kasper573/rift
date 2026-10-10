# rift

An online RPG built in rust.

## Development

One-time setup:

- [Docker](https://www.docker.com/) — runs the local stack (auth, website, game server,
  observability, HTTPS proxy)
- [Rust](https://rustup.rs/) — rustup installs the repo's pinned toolchain on first build
- `wasm-bindgen-cli` for building the wasm client:
  `cargo install wasm-bindgen-cli --locked`
- `cargo-watch` for the dev loop: `cargo install cargo-watch --locked`
- To run the end-to-end suite (`just e2e`): [Node.js](https://nodejs.org/), Google Chrome and `xvfb`.
  The suite is [Playwright](https://playwright.dev) (in [`e2e/`](e2e)); `just e2e` installs its npm
  deps on first run and drives Chrome on a virtual display through a real sign-in, so it's
  zero-config and never touches your desktop. CI fans out across Chrome, Firefox and Safari.
- To record demo videos: [ffmpeg](https://ffmpeg.org/) and a GPU (demos fall back to slow software
  rendering without one). The ui gallery recordings also need `xvfb` and `xdotool`.

Then:

```sh
just dev
```

This deploys the stack (website serving the baked wasm) and rebuilds the wasm on change. There is
no wasm hot reload: sign in at the printed URL and reload the page to pick up rebuilds.
`just stack` deploys the stack alone; `just reset` tears it down and wipes its data.

To see the game in action without playing it, record its demo chapters ([`e2e/demos/`](e2e/demos)):

```sh
just demo combat   # one chapter: target/demo/04-combat.webm
just showcase      # all of them, stitched: target/demo/showcase.webm
just gallery-demo "toasts (sonner)" 8 "mousemove 800 523 click 1"   # a ui component on its own
```

The website lives at <https://rift.localhost> and Grafana at <https://grafana.rift.localhost>.

## Release and deploy

Pushing to `main` builds the wasm client and stack images, and deploys them to production. The
website serves the embedded-wasm `/play` endpoint.
