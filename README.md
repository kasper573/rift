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
- To run the end-to-end suite (`just e2e`): chromedriver and Google Chrome — the `all` set in
  [`.github/actions/setup`](.github/actions/setup/action.yml) is the canonical list. `just e2e` is
  then zero-config: it drives a headless browser through a real sign-in, so it never touches your
  desktop and needs no display or browser flags.

Then:

```sh
just dev
```

This deploys the stack (website serving the baked wasm) and rebuilds the wasm on change. There is
no wasm hot reload: sign in at the printed URL and reload the page to pick up rebuilds.
`just stack` deploys the stack alone; `just reset` tears it down and wipes its data.

The website lives at <https://rift.localhost> and Grafana at <https://grafana.rift.localhost>.

## Release and deploy

Pushing to `main` builds the wasm client and stack images, and deploys them to production. The
website serves the embedded-wasm `/play` endpoint.
