# One builder compiles the wasm client and both native binaries in a single cargo invocation, so
# the shared crates (rift, world, …) and the wasm client are each built once. The website and
# game-server images then just copy their artifact out of it.
FROM rust:1-alpine AS builder
RUN apk add --no-cache musl-dev
WORKDIR /workdir
COPY . .
RUN --mount=type=cache,id=cargo-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=cargo-target,target=/workdir/target \
    cargo x wasm \
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
