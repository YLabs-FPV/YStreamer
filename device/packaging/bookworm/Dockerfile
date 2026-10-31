# Builds YStreamer for Debian 12 "bookworm", which the Radxa ROCK 3A runs
# (Radxa's rsdk images). The release build is for Raspberry Pi OS 13
# "trixie"; its package wants a newer glibc and GStreamer than bookworm has,
# so the ROCK 3A gets its own, from this image. See build.sh next to it
FROM rust:1-slim-bookworm

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates git build-essential dpkg-dev pkg-config \
    libglib2.0-dev libgstreamer1.0-dev \
    libgstreamer-plugins-base1.0-dev libgstrtspserver-1.0-dev \
    && rm -rf /var/lib/apt/lists/*

# What device/backend/rust-toolchain.toml and mise.toml pin, so the build is
# the same as the trixie one apart from the system libraries
RUN rustup toolchain install 1.95.0 --profile minimal \
    --component rustfmt,clippy,rust-src \
    && cargo install cargo-deb --version 3.8.0 --locked \
    && rm -rf /usr/local/cargo/registry
