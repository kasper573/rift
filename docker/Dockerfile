# The miniquad JS loader ships inside the macroquad crate version pinned by Cargo.lock; its own
# net plugin is broken (minification stripped its helper globals), so the client's working
# replacement rides along in the same file.
FROM alpine:3.21 AS game-client-bundle
WORKDIR /bundle
COPY Cargo.lock .
COPY app/game/client/js/ js/
RUN version=$(grep -A1 '^name = "macroquad"$' Cargo.lock | tail -1 | cut -d'"' -f2) \
 && wget -qO- "https://static.crates.io/crates/macroquad/macroquad-${version}.crate" | tar xz \
 && awk 1 "macroquad-${version}/js/mq_js_bundle.js" js/rift_ws.js js/rift_audio.js > mq_js_bundle.js

# The binaries are compiled on the host (cached across CI runs by the rust cache) and copied out
# of target/, which .dockerignore re-includes. The base matches the CI runner's glibc so the
# host-built binaries run; these images only package them, never compile.
FROM ubuntu:24.04 AS website
COPY target/release/website /usr/local/bin/website
COPY target/wasm32-unknown-unknown/release/client.wasm /game/client.wasm
COPY --from=game-client-bundle /bundle/mq_js_bundle.js /game/mq_js_bundle.js
ENV RIFT_GAME_CLIENT_WASM=/game/client.wasm
ENV RIFT_MQ_JS_BUNDLE=/game/mq_js_bundle.js
ENTRYPOINT ["website"]

FROM ubuntu:24.04 AS game-server
# A static busybox supplies wget for the compose healthcheck without an apt layer.
COPY --from=busybox:1.36-musl /bin/busybox /usr/bin/wget
COPY target/release/server /usr/local/bin/game-server
ENTRYPOINT ["game-server"]
