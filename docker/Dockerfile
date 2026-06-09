# cargo-chef builds the dependency graph as its own cached layer, so a code change recompiles only
# the workspace crates, not the ~170 dependencies. One builder still produces the wasm client and
# both native binaries; the website and game-server images copy their artifact out of it.
FROM rust:1-alpine AS chef
RUN apk add --no-cache musl-dev \
    && cargo install cargo-chef --locked
WORKDIR /workdir

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /workdir/recipe.json recipe.json
# Scoped to the shipped binaries: the client is built only for wasm below, so its native-only
# dependencies (ALSA, macroquad) must not be cooked into this glibc-free image.
RUN cargo chef cook --release -p website -p server --recipe-path recipe.json
COPY . .
RUN cargo x wasm \
    && cargo build --release -p website -p server \
    && mkdir -p /out/game \
    && cp target/game-client/client.wasm target/game-client/mq_js_bundle.js /out/game/ \
    && cp target/release/website /out/website \
    && cp target/release/server /out/game-server

FROM alpine:3.21 AS website
COPY --from=builder /out/website /usr/local/bin/website
COPY --from=builder /out/game/ /game/
ENV RIFT_GAME_CLIENT_WASM=/game/client.wasm
ENV RIFT_MQ_JS_BUNDLE=/game/mq_js_bundle.js
ENTRYPOINT ["website"]

FROM alpine:3.21 AS game-server
COPY --from=builder /out/game-server /usr/local/bin/game-server
ENTRYPOINT ["game-server"]
