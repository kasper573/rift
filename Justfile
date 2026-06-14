# Recipes run under bash so they behave identically on the Linux/macOS/Windows CI runners.
set shell := ["bash", "-c"]

compose_file := "docker/docker-compose.yaml"
compose := "docker compose -f " + compose_file + " --profile test"

# Lint everything CI lints: format, the workspace, the installer's feature-gated targets, and the
# `world` contract layer on its own (no `host`) — the client's view, so simulation can't leak into it.
lint:
    cargo fmt --check
    cargo clippy --all-targets -- -D warnings
    cargo clippy -p installer --all-features --all-targets -- -D warnings
    cargo clippy -p world --no-default-features -- -D warnings

# The non-e2e test suite. Only world and installer carry tests; the binary crates (client, server,
# website) are compile-checked by `lint` and built by `build`/`e2e`, so running the whole workspace
# here only recompiled the heavy crates in test config for nothing. Debug build — tests assert on
# contracts, and debug keeps the overflow/debug assertions on.
test:
    cargo test -p world
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

# Pull the external images and build the slow infra images (keycloak's kc.sh build, caddy's xcaddy
# build) — no app binaries or roles.conf needed. A caller can run this while the workspace compiles;
# `just stack` then reuses the freshly built layers and only has the app images left to build.
prewarm:
    {{compose}} pull --ignore-buildable || true
    {{compose}} build keycloak reverse-proxy

# Build and push the prod stack images. No service names: compose's build sections are the source
# of truth for what we publish, so adding/removing/renaming a service needs no change here.
push-images:
    docker compose -f {{compose_file}} --profile prod build
    docker compose -f {{compose_file}} --profile prod push

# Print the prod .env packaged beside the shipped installer (read by the client it launches). The
# external client can't read the cluster's interpolated env, so the domain -> URL convention lives
# here; the realm/audience is derived from its one owner (docker/.env.shared) so it can't drift.
# Runs on every release runner including Windows, so it stays a plain `set shell` recipe (one
# bash -c) rather than a `#!/usr/bin/env bash` shebang, which just can't reliably launch on Windows.
installer-env domain exe="rift":
    @set -euo pipefail; \
      audience=$(grep -E '^RIFT_AUTH_AUDIENCE=' docker/.env.shared | cut -d= -f2); \
      echo "RIFT_CLIENT_ISSUER=https://auth.{{domain}}/realms/${audience}"; \
      echo "RIFT_CLIENT_GAME_SERVER_URL=https://game-server.{{domain}}"; \
      echo "RIFT_CLIENT_OIDC_CLIENT_ID=${audience}"; \
      echo "RIFT_ASSETS_DIR=assets"; \
      echo "RIFT_INSTALLER_METADATA_URL=https://installer.{{domain}}"; \
      echo "RIFT_CLIENT_EXECUTABLE={{exe}}"

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
# Run the client here, on top of the stack, under hot-reload. The client endpoints come from the
# same `installer-env` the release packages, pointed at the test domain, so dev and prod can't drift.
dev: stack
    #!/usr/bin/env bash
    set -euo pipefail
    command -v dx >/dev/null || { echo "just dev needs the Dioxus CLI: cargo install dioxus-cli --locked"; exit 1; }
    domain=$(grep -E '^RIFT_DOMAIN=' docker/.env.test | cut -d= -f2)
    client_env=$(just installer-env "$domain")
    cd game/client
    env $client_env RIFT_ASSETS_DIR="{{justfile_directory()}}/assets" \
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
e2e: stack e2e-run

# Run the e2e test against an already-running stack. CI brings the stack up as its own step (so it
# can trust the CA in between), then calls this; `just e2e` is the all-in-one for local use.
e2e-run:
    cargo build --release -p client
    RIFT_E2E_CLIENT="{{justfile_directory()}}/target/release/rift" \
    RIFT_E2E_SERVER="{{justfile_directory()}}/target/release/server" \
        cargo test -p e2e -- --ignored --nocapture
