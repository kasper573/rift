# rift

An online RPG built in rust.

## Development

One-time setup:

- [Docker](https://www.docker.com/) — runs the local stack (auth, website, game server,
  observability, HTTPS proxy)
- [Rust](https://rustup.rs/) — rustup installs the repo's pinned toolchain on first build
- The [Dioxus CLI](https://dioxuslabs.com/) for the hot-reload dev loop:
  `cargo install dioxus-cli --locked`
- To run the end-to-end suite (`just e2e`): Google Chrome plus the e2e system packages (a virtual
  X server, a window manager and the capture/render libraries) — the `all` set in
  [`.github/actions/setup`](.github/actions/setup/action.yml) is the canonical list. `just e2e` is
  then zero-config: it drives the released client through a real sign-in on its own throwaway
  headless display, so it never touches your desktop and needs no display or browser flags.

Then:

```sh
just dev
```

This deploys the stack to docker and runs the native client on top of it under hot-reload:
edited systems patch into the running game, and edited `assets/` content reloads in place.
`just stack` redeploys the stack alone; `just reset` tears it down and wipes its data.

The website lives at <https://rift.localhost> and Grafana at <https://grafana.rift.localhost>.

### Trust the local certificate authority (once per machine)

Caddy terminates HTTPS for `*.rift.localhost` with a local CA it manages itself. The OS trust
store covers the game client and most tools, but Chrome keeps its own store, so both need a
one-time import. With the stack running:

```sh
docker compose -f docker/docker-compose.yaml cp reverse-proxy:/data/caddy/pki/authorities/local/root.crt /tmp/rift-root.crt
sudo install -m 644 /tmp/rift-root.crt /usr/local/share/ca-certificates/rift-root.crt && sudo update-ca-certificates --fresh
certutil -d sql:$HOME/.pki/nssdb -A -t C,, -n rift-root -i /tmp/rift-root.crt   # Chrome; needs libnss3-tools
```

`install -m 644` matters: the exported file is mode 600, and a straight `cp` leaves a root-only
cert that TLS clients scanning `/etc/ssl/certs` warn about on every connection.

`just reset` wipes the CA along with the rest of the stack's data. Repeat the import after, and
remove any previously trusted copy first — a stale root with the same name fails signature
checks (`BadSignature`) rather than falling through to the fresh one.

## Release and deploy

Binaries are built for all supported platforms and published as github releases automatically on push to `main`, and deploys the latest release to the production environment.
