# The binaries are compiled on the host (cached across CI runs by the rust cache) and copied out
# of target/, which .dockerignore re-includes. The base matches the CI runner's glibc so the
# host-built binaries run; these images only package them, never compile.
FROM ubuntu:24.04 AS website
COPY target/release/website /usr/local/bin/website
ENTRYPOINT ["website"]

FROM ubuntu:24.04 AS game-server
# A static busybox supplies wget for the compose healthcheck without an apt layer.
COPY --from=busybox:1.36-musl /bin/busybox /usr/bin/wget
COPY target/release/server /usr/local/bin/game-server
# Content is read from disk at boot (world::validate panics on anything broken).
COPY assets/ /assets
ENV RIFT_ASSETS_DIR=/assets
ENTRYPOINT ["game-server"]
