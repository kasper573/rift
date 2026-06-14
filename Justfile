# Recipes run under bash so they behave identically on the Linux/macOS/Windows CI runners.
set shell := ["bash", "-c"]

compose_file := "docker/docker-compose.yaml"
compose := "docker compose -f " + compose_file + " --profile test"

# Lint everything CI lints: format, the workspace, and the installer's feature-gated targets.
lint:
    cargo fmt --check
    cargo clippy --all-targets -- -D warnings
    cargo clippy -p installer --all-features --all-targets -- -D warnings

# The non-e2e test suite: the workspace plus the installer's feature-gated tests.
test:
    cargo test --release
    cargo test -p installer --all-features

# Build the server/website/installer-backend artifacts and the keycloak provisioning input the stack images package.
build:
    cargo build --release -p website -p server
    cargo build --release -p installer --features backend --bin installer-backend
    cargo run --release -p world --bin kc-roles > docker/keycloak/roles.conf

# Build the shipping client and the installer GUI (the artifacts the release packages per OS).
build-client:
    cargo build --release -p client -p installer --features installer/frontend

# Build the artifacts and (re)deploy the local stack; compose only restarts what changed.
stack: build
    docker network inspect rift >/dev/null 2>&1 || docker network create rift
    {{compose}} up -d --build --wait

# Build and push the prod stack images. No service names: compose's build sections are the source
# of truth for what we publish, so adding/removing/renaming a service needs no change here.
push-images:
    docker compose -f {{compose_file}} --profile prod build
    docker compose -f {{compose_file}} --profile prod push

# Print the prod .env packaged beside the shipped installer (read by the client it launches). The
# domain -> URL convention lives here rather than being restated in the release workflow.
installer-env domain exe="rift":
    @echo 'RIFT_CLIENT_ISSUER=https://auth.{{domain}}/realms/rift'
    @echo 'RIFT_CLIENT_GAME_SERVER_URL=https://game-server.{{domain}}'
    @echo 'RIFT_ASSETS_DIR=assets'
    @echo 'RIFT_INSTALLER_METADATA_URL=https://installer.{{domain}}'
    @echo 'RIFT_CLIENT_EXECUTABLE={{exe}}'

# Copy the stack's first-boot CA out of the running reverse-proxy (the service name and in-container
# path live here, not in the workflow or the README). Waits for caddy to mint it.
ca-cert out:
    #!/usr/bin/env bash
    set -euo pipefail
    for _ in $(seq 1 60); do
      {{compose}} cp reverse-proxy:/data/caddy/pki/authorities/local/root.crt "{{out}}" 2>/dev/null && exit 0
      sleep 2
    done
    echo "reverse-proxy CA not available" >&2
    exit 1

# Dump all stack container logs (CI failure diagnostics).
logs:
    {{compose}} logs --no-color

# Re-sync keycloak's roles and groups from source (game/world): rebuild the keycloak image and
# restart it — its entrypoint re-provisions on every start, the same mechanism production uses.
kc-provision:
    cargo run --release -p world --bin kc-roles > docker/keycloak/roles.conf
    {{compose}} up -d --build keycloak

# `dx serve --hot-patch` live-patches the bodies of changed systems in the running process and
# falls back to a full rebuild for changes it can't patch; Bevy's file_watcher hot-reloads
# edited assets.
# Run the client here, on top of the stack, under hot-reload.
dev: stack
    @command -v dx >/dev/null || { echo "just dev needs the Dioxus CLI: cargo install dioxus-cli --locked"; exit 1; }
    cd game/client && \
    RIFT_ASSETS_DIR="{{justfile_directory()}}/assets" \
    RIFT_CLIENT_ISSUER=https://auth.rift.localhost/realms/rift \
    RIFT_CLIENT_GAME_SERVER_URL=https://game-server.rift.localhost \
        dx serve --hot-patch --package client --bin rift --features hotpatch --platform desktop

# Keycloak only imports a realm on first boot, so the realm DB must be wiped for
# `rift-realm.json` changes (e.g. redirect URIs) to re-apply.
# Tear the stack down and wipe its volumes.
reset:
    {{compose}} down -v

# The E2E test against the shipping client: it drives a real gameplay session on the screen —
# including registering a fresh account through the browser against the stack's keycloak — and
# asserts on rendered pixels, so it runs against the desktop you're on and needs the stack up
# and its CA trusted (see the README). The client/server binaries are passed by path so the test
# never rebuilds them. CI runs the same test on linux (and supplies its own headless display).
e2e: stack
    cargo build --release -p client
    RIFT_E2E_CLIENT="{{justfile_directory()}}/target/release/rift" \
    RIFT_E2E_SERVER="{{justfile_directory()}}/target/release/server" \
        cargo test -p e2e -- --ignored --nocapture
