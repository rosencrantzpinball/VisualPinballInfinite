# The image the player is built in for the Steam Frame (Linux arm64): Valve's Steam Runtime 4 SDK for arm64 with
# VPX's build dependencies, Xvfb and Python (the local image vpinball-steamrt4-arm64, never changed here), plus:
#  - Mesa's Vulkan drivers, for lavapipe: the player's flat test runs on Xvfb draw through it;
#  - rustup with the workspace's Rust (1.98), minimal profile, and its wasm32-wasip1 target for the browser build;
#  - DXC, built from source by build-dxc.sh: the player's HLSL shaders become SPIR-V at build time.
# Built by image.sh. Sources, Cargo's home and every build output stay outside, bind-mounted.
FROM vpinball-steamrt4-arm64:latest AS rust
SHELL ["/bin/bash", "-o", "pipefail", "-c"]

RUN apt-get update -y \
    && apt-get install -y --no-install-recommends mesa-vulkan-drivers \
    && rm -rf /var/lib/apt/lists/*

ENV RUSTUP_HOME=/opt/rust/rustup \
    PATH=/opt/rust/cargo/bin:/opt/dxc/bin:$PATH
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs -o /tmp/rustup-init.sh \
    && CARGO_HOME=/opt/rust/cargo sh /tmp/rustup-init.sh -y --profile minimal --default-toolchain 1.98 --no-modify-path \
    && rm /tmp/rustup-init.sh \
    && rustc --version && cargo --version
# the same Rust's WebAssembly target, for the browser build (tools/web)
RUN CARGO_HOME=/opt/rust/cargo rustup target add wasm32-wasip1

FROM rust
COPY dxc-install/ /opt/dxc/
RUN dxc --version
