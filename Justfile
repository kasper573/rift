set shell := ["bash", "-c"]

compose_file := "docker/docker-compose.yaml"
compose := "docker compose -f " + compose_file + " --profile test"

lint:
    cargo fmt --check
    cargo run --quiet -p lint
    cargo clippy --release --all-targets -- -D warnings
    cargo clippy --release -p client --target wasm32-unknown-unknown -- -D warnings

test:
    cargo test --release -p world

# Find the highest area count sustained within the tick budget. Players are congested (all on the
# spawn tile, one shared view) by default; `just bench dist` spreads them for distinct views.
bench mode="":
    cargo run --release -p bench -- {{mode}}

# Rasterize a whole map to an image to preview it (e.g. `just render island island.png`). Defaults
# the output to `<map>.png`.
render map out="":
    cargo run -p bevy_tiled --bin render -- {{map}} {{out}}

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

# --profile dev adds the grafana/loki/tempo/... stack (opt-in; see the e2e-stack-up note).
stack-up:
    docker network inspect rift >/dev/null 2>&1 || docker network create rift
    {{compose}} --profile dev up -d --build --wait

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
    cargo watch -w game/client -w game/world -w ui -w bevy \
      -s 'just wasm && {{compose}} up -d --build --wait rift-website'

# `down -v` wipes volumes: keycloak only imports its realm on first boot, so realm changes
# (e.g. redirect URIs) don't apply until the DB is gone.
reset:
    {{compose}} down -v

# Build, bring up a fresh stack, and run the Playwright suite (e2e/). FILTER limits which tests run.
e2e filter="": build wasm e2e-stack-up (e2e-run filter)

# The "test" profile leaves out the observability stack (it's opt-in via the "dev" profile, which
# only stack-up activates) — the e2e doesn't exercise it, and pulling/starting it is the bulk of the
# stack's startup. Everything else comes up, so new game services are picked up automatically.
[private]
e2e-stack-up:
    docker network inspect rift >/dev/null 2>&1 || docker network create rift
    {{compose}} up -d --build --wait

e2e-run filter="":
    #!/usr/bin/env bash
    set -euo pipefail
    set -a; source docker/.env.test; set +a
    cd e2e
    [ -d node_modules ] || npm ci
    export LP_NUM_THREADS=2 # bound each headed browser's Mesa threads so parallel workers don't thrash
    if [ "${E2E_ALL_BROWSERS:-}" = "1" ]; then
      xvfb-run -a npx playwright test {{filter}}
    else
      npx playwright test {{filter}}
    fi
