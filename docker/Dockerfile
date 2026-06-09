# The website and game-server binaries are compiled on the host (cached across CI runs by the
# rust cache) and staged into docker/stage by `cargo x package`. The base matches the CI runner's
# glibc so the host-built binaries run; these images only package them, never compile.
FROM ubuntu:24.04 AS website
COPY docker/stage/website /usr/local/bin/website
COPY docker/stage/game/ /game/
ENV RIFT_GAME_CLIENT_WASM=/game/client.wasm
ENV RIFT_MQ_JS_BUNDLE=/game/mq_js_bundle.js
ENTRYPOINT ["website"]

FROM ubuntu:24.04 AS game-server
# A static busybox supplies wget for the compose healthcheck without an apt layer.
COPY --from=busybox:1.36-musl /bin/busybox /usr/bin/wget
COPY docker/stage/game-server /usr/local/bin/game-server
ENTRYPOINT ["game-server"]
