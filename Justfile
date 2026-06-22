set shell := ["bash", "-c"]

compose_file := "docker/docker-compose.yaml"
compose := "docker compose -f " + compose_file + " --profile test"

lint:
    cargo fmt --check
    cargo clippy --release --all-targets -- -D warnings
    cargo clippy --release -p world --no-default-features -- -D warnings
    cargo clippy --release -p client --target wasm32-unknown-unknown -- -D warnings

test:
    cargo test --release -p world

bench:
    cargo run --release -p bench

build:
    cargo build --release -p website -p server
    cargo run --release -p world --bin kc-roles > docker/keycloak/roles.conf

# The browser client: a wasm cdylib post-processed by wasm-bindgen into the bundle the website serves
# (and bakes into its image). Assets are embedded in the binary, so there's nothing else to ship. The
# dev loop builds it plain and fast; the deploy alone re-runs this with LTO + size opt-level + strip
# (env, see ci.yml) to roughly halve the bundle for mobile Safari's per-tab memory budget.
wasm:
    cargo build --release -p client --target wasm32-unknown-unknown
    wasm-bindgen --target web --no-typescript --out-name rift --out-dir target/wasm \
      target/wasm32-unknown-unknown/release/rift.wasm

stack: build wasm stack-up

stack-up:
    docker network inspect rift >/dev/null 2>&1 || docker network create rift
    {{compose}} up -d --build --wait

prewarm:
    {{compose}} pull --ignore-buildable || true
    {{compose}} build keycloak reverse-proxy

# No service names: compose's build sections are the single source of truth for what we publish.
push-images:
    docker compose -f {{compose_file}} --profile prod build
    docker compose -f {{compose_file}} --profile prod push

logs:
    {{compose}} logs --no-color

kc-provision:
    cargo run --release -p world --bin kc-roles > docker/keycloak/roles.conf
    {{compose}} up -d --build keycloak

# Bring the stack up (website serving the baked wasm), then rebuild the wasm and refresh just the
# website image on every change. There's no wasm hot reload: sign in at the printed URL and reload
# the page to pick up a rebuild.
dev: stack
    #!/usr/bin/env bash
    set -euo pipefail
    command -v cargo-watch >/dev/null || { echo "just dev needs cargo-watch: cargo install cargo-watch --locked"; exit 1; }
    set -a; source docker/.env.test; set +a
    echo "sign in and play at https://${RIFT_DOMAIN}/play"
    cargo watch -w game/client -w game/world -w ui \
      -s 'just wasm && {{compose}} up -d --build --wait rift-website'

# `down -v` wipes volumes: keycloak only imports its realm on first boot, so realm changes
# (e.g. redirect URIs) don't apply until the DB is gone.
reset:
    {{compose}} down -v

# An optional FILTER runs only matching tests (e.g. `just e2e portal`); omit it to run the whole
# suite. Either way it's the one command — never reach for cargo/chromedriver by hand.
e2e filter="": build wasm e2e-stack-up (e2e-run filter)

# The e2e walks an idle player across the island to a portal; with NPCs present they attack it (and a
# move click can land on one as an attack), so the e2e stack alone starts areas empty of NPCs. Every
# other stack — `just dev`, production — keeps them.
[private]
e2e-stack-up:
    RIFT_GAME_SERVER_SPAWN_NPCS=false just stack-up

e2e-build:
    cargo test --release -p e2e --no-run

# Drives the live stack through a headless browser via chromedriver, so `just e2e` behaves the same
# here and in CI. `--test-threads=1`: the tests register fresh accounts against the one shared
# Keycloak, so they run one at a time. Override RIFT_E2E_URL to point the suite at a deployment.
e2e-run filter="": e2e-build
    #!/usr/bin/env bash
    set -euo pipefail
    set -a; source docker/.env.test; set +a
    command -v chromedriver >/dev/null || { echo "just e2e needs chromedriver: install the e2e system packages (see README)"; exit 1; }

    chromedriver --port=9515 >/dev/null 2>&1 &
    driver=$!
    trap 'kill "$driver" 2>/dev/null || true' EXIT
    for _ in $(seq 1 30); do
      curl -fsS http://127.0.0.1:9515/status >/dev/null 2>&1 && break
      sleep 0.5
    done

    cargo test --release -p e2e -- --ignored --nocapture --test-threads=1 {{filter}}
