# The binaries are compiled on the host (cached across CI runs by the rust cache) and copied out
# of target/, which .dockerignore re-includes. The base matches the CI runner's glibc so the
# host-built binaries run; these images only package them, never compile.

# Shared runtime: ubuntu plus the system CA bundle that rustls' verifier reads. Every service runs
# the pyroscope agent, whose reqwest client loads the CA store at startup and panics without one;
# the installer also calls the GitHub API and the game-server reaches keycloak.
FROM ubuntu:24.04 AS runtime
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

FROM runtime AS website
COPY target/release/website /usr/local/bin/website
ENTRYPOINT ["website"]

FROM runtime AS installer-backend
COPY target/release/installer-backend /usr/local/bin/installer-backend
ENTRYPOINT ["installer-backend"]

FROM runtime AS game-server
# A static busybox supplies wget for the compose healthcheck without an apt layer.
COPY --from=busybox:1.36-musl /bin/busybox /usr/bin/wget
COPY target/release/server /usr/local/bin/game-server
# Content is read from disk at boot (world::validate panics on anything broken).
COPY assets/ /assets
ENV RIFT_ASSETS_DIR=/assets
ENTRYPOINT ["game-server"]
