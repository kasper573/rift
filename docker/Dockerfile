# The website and game-server binaries are compiled on the host — cached across CI runs by the
# Rust cache — and staged into docker/stage by `cargo x package`. These images only package them,
# so a code change that doesn't touch dependencies never recompiles anything inside Docker.
FROM alpine:3.21 AS website
COPY docker/stage/website /usr/local/bin/website
COPY docker/stage/game/ /game/
ENV RIFT_GAME_CLIENT_WASM=/game/client.wasm
ENV RIFT_MQ_JS_BUNDLE=/game/mq_js_bundle.js
ENTRYPOINT ["website"]

FROM alpine:3.21 AS game-server
COPY docker/stage/game-server /usr/local/bin/game-server
ENTRYPOINT ["game-server"]
