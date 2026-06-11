FROM rust:1-bookworm

# The client renders a Bevy window on the host's X server: the X11 client libs winit dlopens, a
# Vulkan loader with a software driver (lavapipe) so it renders without GPU passthrough, and Xvfb
# (with xauth, which xvfb-run needs) so the smoke test can run against a virtual display when none
# is forwarded. socat bridges loopback to the stack containers for sign-in and netcode (see `just`).
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        socat libasound2-dev \
        libx11-6 libxcursor1 libxrandr2 libxi6 libxinerama1 libxkbcommon0 libxkbcommon-x11-0 \
        libvulkan1 mesa-vulkan-drivers \
        xvfb xauth \
    && rm -rf /var/lib/apt/lists/*

COPY rust-toolchain.toml /tmp/toolchain/rust-toolchain.toml
# The root-run toolchain install creates root-owned files inside the rustup/cargo homes;
# reopen them for the dev user, as the base image does for its own installs.
RUN cd /tmp/toolchain && rustup show && rm -r /tmp/toolchain \
    && chmod -R a+rwX "$RUSTUP_HOME" "$CARGO_HOME"
# Login shells reset PATH via /etc/profile, hiding the image's cargo location.
RUN echo 'export PATH=/usr/local/cargo/bin:$PATH' > /etc/profile.d/cargo.sh

# The dev loop: `just dev` (see Justfile).
RUN cargo install --locked just watchexec-cli \
    && chmod -R a+rwX "$CARGO_HOME"

RUN useradd -m dev
