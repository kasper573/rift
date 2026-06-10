FROM rust:1-bookworm

RUN apt-get update \
    && apt-get install -y --no-install-recommends socat libasound2-dev \
    && rm -rf /var/lib/apt/lists/*

COPY rust-toolchain.toml /tmp/toolchain/rust-toolchain.toml
# The root-run toolchain install creates root-owned files inside the rustup/cargo homes;
# reopen them for the dev user, as the base image does for its own installs.
RUN cd /tmp/toolchain && rustup show && rm -r /tmp/toolchain \
    && chmod -R a+w "$RUSTUP_HOME" "$CARGO_HOME"
# Login shells reset PATH via /etc/profile, hiding the image's cargo location.
RUN echo 'export PATH=/usr/local/cargo/bin:$PATH' > /etc/profile.d/cargo.sh

RUN useradd -m dev
