compose := "docker compose -f docker/docker-compose.yaml --profile test"

# Build the server/website artifacts and the keycloak provisioning input the stack images package.
build:
    cargo build --release -p website -p server
    cargo run --release -p world --bin kc-roles > docker/keycloak/roles.conf

# Build the artifacts and (re)deploy the local stack; compose only restarts what changed.
stack: build
    docker network inspect rift >/dev/null 2>&1 || docker network create rift
    {{compose}} up -d --build --wait

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
    RIFT_ASSETS="{{justfile_directory()}}/assets" \
    RIFT_CLIENT_ISSUER=https://auth.rift.localhost/realms/rift \
    RIFT_CLIENT_GAME_SERVER_URL=https://game-server.rift.localhost \
        dx serve --hot-patch --bin rift --features hotpatch --platform desktop

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
    cargo build --release --features dist -p client
    RIFT_E2E_CLIENT="{{justfile_directory()}}/target/release/rift" \
    RIFT_E2E_SERVER="{{justfile_directory()}}/target/release/server" \
        cargo test -p e2e -- --ignored --nocapture
