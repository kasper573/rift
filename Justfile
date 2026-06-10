# Build the artifacts and (re)deploy the local stack; compose only restarts what changed.
stack:
    cargo build --release -p website -p server
    cargo wasm
    docker compose -f docker/docker-compose.yaml --profile test up -d --build --wait

# Watch mode: every save rebuilds and redeploys, honoring .gitignore.
dev:
    watchexec --clear --on-busy-update queue -- just stack
