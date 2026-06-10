# Build the release artifacts that the stack packages and the dev loop bind-mounts.
build:
    cargo build --release -p website -p server
    cargo wasm

# Build the artifacts and (re)deploy the local stack; compose only restarts what changed.
stack: build
    docker compose -f docker/docker-compose.yaml --profile test up -d --build --wait

# Watch mode: the stack runs with target/ bind-mounted, so every save rebuilds and restarts
# only the services whose binary changed; a new client.wasm just needs a browser refresh.
dev: build
    docker compose -f docker/docker-compose.yaml -f docker/docker-compose.dev.yaml --profile test up -d --build --wait
    watchexec --clear --on-busy-update queue -- just _dev-cycle

_dev-cycle:
    #!/usr/bin/env bash
    set -euo pipefail
    mtime() { stat -c %Y "$1" 2>/dev/null || echo 0; }
    server_before=$(mtime target/release/server)
    website_before=$(mtime target/release/website)
    cargo build --release -p website -p server & native=$!
    cargo wasm & wasm=$!
    status=0
    wait "$native" || status=$?
    wait "$wasm" || status=$?
    [ "$status" -eq 0 ] || exit "$status"
    changed=()
    if [ "$(mtime target/release/server)" != "$server_before" ]; then changed+=(rift-game-server); fi
    if [ "$(mtime target/release/website)" != "$website_before" ]; then changed+=(rift-website); fi
    if [ "${#changed[@]}" -gt 0 ]; then
        docker compose -f docker/docker-compose.yaml -f docker/docker-compose.dev.yaml --profile test restart "${changed[@]}"
    fi
