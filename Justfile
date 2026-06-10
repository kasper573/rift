# Build the release artifacts that the stack packages and the dev loop bind-mounts.
build:
    cargo build --release -p website -p server
    cargo wasm

# Build the artifacts and (re)deploy the local stack; compose only restarts what changed.
stack: build
    docker compose -f docker/docker-compose.yaml --profile test up -d --build --wait

# Watch mode: the stack runs with target/ bind-mounted, so every save rebuilds and restarts
# only the services whose binary changed; a new client.wasm is picked up by a browser refresh
# alone. Images are only built when missing — the running loop never needs the registry.
dev: build
    #!/usr/bin/env bash
    set -euo pipefail
    compose="docker compose -f docker/docker-compose.yaml -f docker/docker-compose.dev.yaml --profile test"
    for image in $($compose config --images); do
        docker image inspect "$image" >/dev/null 2>&1 || { $compose build; break; }
    done
    $compose up -d --wait
    watchexec --clear --on-busy-update queue -- just _dev-cycle

# One cycle: rebuild, then restart every service whose container runs a process older than its
# binary on disk. Comparing against the container start (not this cycle's work) makes cycles
# self-healing: an interrupted cycle or a stale bring-up is corrected by the next run.
_dev-cycle:
    #!/usr/bin/env bash
    set -euo pipefail
    compose="docker compose -f docker/docker-compose.yaml -f docker/docker-compose.dev.yaml --profile test"
    cargo build --release -p website -p server & native=$!
    cargo wasm & wasm=$!
    status=0
    wait "$native" || status=$?
    wait "$wasm" || status=$?
    [ "$status" -eq 0 ] || exit "$status"
    stale=()
    for pair in "rift-game-server target/release/server" "rift-website target/release/website"; do
        set -- $pair
        container=$($compose ps -q "$1")
        if [ -n "$container" ]; then
            started=$(docker inspect "$container" --format '{{{{.State.StartedAt}}')
            awk -v binary="$(stat -c %.Y "$2")" -v started="$(date -d "$started" +%s.%N)" \
                'BEGIN { exit !(binary > started) }' || continue
        fi
        stale+=("$1")
    done
    if [ "${#stale[@]}" -gt 0 ]; then
        $compose up -d "${stale[@]}"
        $compose restart "${stale[@]}"
    fi
