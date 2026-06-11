issuer := "https://auth.rift.localhost/realms/rift"
game_url := "https://game-server.rift.localhost"
ca := "target/caddy-root.crt"
compose := "docker compose -f docker/docker-compose.yaml -f docker/docker-compose.dev.yaml --profile test"

# Build the server/website artifacts the stack bind-mounts.
build:
    cargo build --release -p website -p server

# Build the artifacts and (re)deploy the local stack; compose only restarts what changed.
stack: build _net
    docker compose -f docker/docker-compose.yaml --profile test up -d --build --wait

# Ensure the external docker network the compose stack attaches to exists.
_net:
    docker network inspect rift >/dev/null 2>&1 || docker network create rift

# Tear the stack down and wipe its volumes. Keycloak only imports a realm on first boot, so the
# realm DB must be wiped for `rift-realm.json` changes (e.g. redirect URIs) to re-apply.
reset:
    {{compose}} down -v

# Watch mode: bring up the stack and run the client here under Bevy's first-class hot-reload.
# `dx serve --hot-patch` live-patches the bodies of changed `Update` systems in the running process
# (subsecond), and Bevy's file_watcher hot-reloads edited assets — so most edits apply without
# restarting the app, and dx falls back to a full rebuild for changes it can't patch (new systems,
# signatures, schedules). The client is a native desktop app, so it runs on this machine — real GPU,
# browser, audio, and loopback — connecting to the compose stack over its published ports exactly
# like a shipped client connects to production.
dev: build _up trust
    cd game/client && \
    RIFT_ASSETS="{{justfile_directory()}}/assets" \
    RIFT_CLIENT_ISSUER={{issuer}} \
    RIFT_CLIENT_GAME_URL={{game_url}} \
    RIFT_CLIENT_EXTRA_CA="{{justfile_directory()}}/{{ca}}" \
        dx serve --hot-patch --bin rift --features hotpatch --platform {{os()}}

# Bring the stack up (building images only when missing) and export the proxy's local CA, which both
# the client and the browser trust for sign-in and session minting.
_up:
    #!/usr/bin/env bash
    set -euo pipefail
    for image in $({{compose}} config --images); do
        docker image inspect "$image" >/dev/null 2>&1 || { {{compose}} build; break; }
    done
    {{compose}} up -d --wait
    {{compose}} cp reverse-proxy:/data/caddy/pki/authorities/local/root.crt {{ca}}

# Trust Caddy's local dev CA in the browser so it accepts https://*.rift.localhost during sign-in.
# The client trusts the same CA file directly (RIFT_CLIENT_EXTRA_CA), but browsers keep their own
# NSS store, so the cert has to go there too. Idempotent, and re-run by `dev` because `reset`
# rotates the CA. Needs certutil (libnss3-tools); without it the client still works but the sign-in
# tab shows a certificate warning.
trust: _up
    #!/usr/bin/env bash
    set -euo pipefail
    if ! command -v certutil >/dev/null 2>&1; then
        echo "⚠ certutil not found (install libnss3-tools) — the browser won't trust the dev CA" >&2
        exit 0
    fi
    mkdir -p "$HOME/.pki/nssdb"
    certutil -d sql:"$HOME/.pki/nssdb" -D -n rift-caddy-local 2>/dev/null || true
    certutil -d sql:"$HOME/.pki/nssdb" -A -t "C,," -n rift-caddy-local -i {{ca}}
