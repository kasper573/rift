# The binaries are compiled on the host (cached across CI runs by the rust cache) and copied out
# of target/, which .dockerignore re-includes. The base matches the CI runner's glibc so the
# host-built binaries run; these images only package them, never compile.

# Shared runtime: ubuntu plus the system CA bundle that rustls' verifier reads. Every service runs
# the pyroscope agent, whose reqwest client loads the CA store at startup and panics without one;
# the game-server additionally reaches keycloak.
FROM ubuntu:24.04 AS runtime
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

FROM runtime AS website
COPY target/release/website /usr/local/bin/website
# The wasm client bundle the website serves at /wasm (built by `just wasm` before the image).
COPY target/wasm /wasm
ENTRYPOINT ["website"]

FROM runtime AS game-server
# A static busybox supplies wget for the compose healthcheck without an apt layer.
COPY --from=busybox:1.36-musl /bin/busybox /usr/bin/wget
COPY target/release/server /usr/local/bin/game-server
ENTRYPOINT ["game-server"]
