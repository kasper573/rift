# rift

An online RPG built in rust.

I'm doing this project for fun and to teach myself more about multiplayer game development and
web development infrastructure.

## Development

One-time setup:

- [Docker](https://www.docker.com/) — runs the local stack (auth, website, game server,
  observability, HTTPS proxy)
- [Rust](https://rustup.rs/) — rustup installs the repo's pinned toolchain on first build
- The [Dioxus CLI](https://dioxuslabs.com/) for the hot-reload dev loop:
  `cargo install dioxus-cli --locked`

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
sudo cp /tmp/rift-root.crt /usr/local/share/ca-certificates/rift-root.crt && sudo update-ca-certificates
certutil -d sql:$HOME/.pki/nssdb -A -t C,, -n rift-root -i /tmp/rift-root.crt   # Chrome; needs libnss3-tools
```

`just reset` wipes the CA along with the rest of the stack's data; repeat the import after.

## Testing

```sh
cargo test -p client --test e2e
```

This plays an honest session: the real client binary against a freshly spawned server on a
private X display, driven by genuine input and asserted through screenshots. It needs `Xvfb`
(`apt install xvfb`).

## Production deployment

This repository comes with a github actions workflow that performs automatic
deployments whenever the main branch receives updates. It's a simple deploy
script designed to deploy to a single remote machine. It logs in to your remote
machine via ssh and updates or initializes the docker stack utilizing the same
docker compose file as in development but with production environment variables
provided via github action variables and secrets.

Review the workflow to see which variables and secrets you need to provide.
