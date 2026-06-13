# The binaries are compiled on the host (cached across CI runs by the rust cache) and copied out
# of target/, which .dockerignore re-includes. The base matches the CI runner's glibc so the
# host-built binaries run; these images only package them, never compile.
FROM ubuntu:24.04 AS website
COPY target/release/website /usr/local/bin/website
ENTRYPOINT ["website"]

FROM ubuntu:24.04 AS installer-backend
# The only image that calls a public HTTPS endpoint (the GitHub API), so it needs the system CA bundle
# that rustls' platform verifier reads; the other services only reach keycloak over plain http.
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY target/release/installer-backend /usr/local/bin/installer-backend
ENTRYPOINT ["installer-backend"]

FROM ubuntu:24.04 AS game-server
# A static busybox supplies wget for the compose healthcheck without an apt layer.
COPY --from=busybox:1.36-musl /bin/busybox /usr/bin/wget
COPY target/release/server /usr/local/bin/game-server
# Content is read from disk at boot (world::validate panics on anything broken).
COPY assets/ /assets
ENV RIFT_ASSETS_DIR=/assets
ENTRYPOINT ["game-server"]
