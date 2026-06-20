set shell := ["bash", "-c"]

compose_file := "docker/docker-compose.yaml"
compose := "docker compose -f " + compose_file + " --profile test"

lint:
    cargo fmt --check
    cargo clippy --release --all-targets -- -D warnings
    cargo clippy --release -p installer --all-features --all-targets -- -D warnings
    cargo clippy --release -p world --no-default-features -- -D warnings

test:
    cargo test --release -p world
    cargo test --release -p installer --all-features

bench:
    cargo run --release -p bench

build:
    cargo build --release -p website -p server
    cargo build --release -p installer --features backend --bin installer-backend
    cargo run --release -p world --bin kc-roles > docker/keycloak/roles.conf

build-client:
    cargo build --release -p client -p installer --features installer/frontend

stack: build stack-up

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

# No shebang: this runs on the Windows release runner, which can't launch `#!/usr/bin/env bash`.
installer-env domain exe="rift":
    @set -euo pipefail; \
      audience=$(grep -E '^RIFT_AUTH_AUDIENCE=' docker/.env.shared | cut -d= -f2); \
      echo "RIFT_CLIENT_ISSUER=https://auth.{{domain}}/realms/${audience}"; \
      echo "RIFT_CLIENT_GAME_SERVER_URL=https://game-server.{{domain}}"; \
      echo "RIFT_CLIENT_OIDC_CLIENT_ID=${audience}"; \
      echo "RIFT_ASSETS_DIR=assets"; \
      echo "RIFT_INSTALLER_METADATA_URL=https://installer.{{domain}}"; \
      echo "RIFT_CLIENT_EXECUTABLE={{exe}}"

ca-cert out:
    #!/usr/bin/env bash
    set -euo pipefail
    for _ in $(seq 1 60); do
      {{compose}} cp reverse-proxy:/data/caddy/pki/authorities/local/root.crt "{{out}}" 2>/dev/null && exit 0
      sleep 2
    done
    echo "reverse-proxy CA not available" >&2
    exit 1

logs:
    {{compose}} logs --no-color

kc-provision:
    cargo run --release -p world --bin kc-roles > docker/keycloak/roles.conf
    {{compose}} up -d --build keycloak

dev: stack
    #!/usr/bin/env bash
    set -euo pipefail
    command -v dx >/dev/null || { echo "just dev needs the Dioxus CLI: cargo install dioxus-cli --locked"; exit 1; }
    domain=$(grep -E '^RIFT_DOMAIN=' docker/.env.test | cut -d= -f2)
    client_env=$(just installer-env "$domain")
    cd game/client
    env $client_env RIFT_ASSETS_DIR="{{justfile_directory()}}/assets" \
        dx serve --hot-patch --package client --bin rift --features hotpatch --platform desktop

# `down -v` wipes volumes: keycloak only imports its realm on first boot, so realm changes
# (e.g. redirect URIs) don't apply until the DB is gone.
reset:
    {{compose}} down -v

e2e: stack e2e-run

e2e-build:
    cargo build --release -p client -p server
    cargo test --release -p e2e --no-run

# `--test-threads=1`: every e2e drives one shared display, injects OS input and finds the client
# window by title, so the tests must run one at a time.
e2e-run: e2e-build
    RIFT_E2E_CLIENT="{{justfile_directory()}}/target/release/rift" \
    RIFT_E2E_SERVER="{{justfile_directory()}}/target/release/server" \
        cargo test --release -p e2e -- --ignored --nocapture --test-threads=1

# The `ui` component showcase: drives every component's states with real input on a real display.
# Needs an unlocked desktop session (it injects OS input). Build the gallery first, then drive it.
gallery:
    cargo build -p client --bin gallery
    cargo test -p e2e --test gallery --no-run
    RIFT_GALLERY="{{justfile_directory()}}/target/debug/gallery" \
        cargo test -p e2e --test gallery -- --ignored --nocapture
