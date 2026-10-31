# The image CEF is built in for the Steam Frame (Linux arm64), natively on arm64: the player's image
# (vpi-steamrt4-arm64: Valve's Steam Runtime 4 SDK, so libcef links against the glibc the Frame's runtime has, and
# Rust 1.98) plus what Chromium's build needs that the SDK lacks, from Debian's own packages: GTK 3's and AT-SPI's
# headers, Mesa's dri.pc (where Mesa's drivers are, for the GPU process's sandbox), lld to link LLVM with, and Go 1.24
# to bootstrap the Go that builds TypeScript's native compiler. Clang, gn, bindgen, Node.js, Rust, Go 1.26 and tsgo
# are built from source (build-toolchain.sh): Chromium's own prebuilt toolchain is never fetched. Built by image.sh
# beside this file. Sources and every build output stay outside, bind-mounted.
FROM vpi-steamrt4-arm64:latest
SHELL ["/bin/bash", "-o", "pipefail", "-c"]

RUN apt-get update -y \
    && apt-get install -y --no-install-recommends libgtk-3-dev libatspi2.0-dev mesa-common-dev lld-19 golang-1.24-go \
    && rm -rf /var/lib/apt/lists/* \
    && pkg-config --variable=dridriverdir dri && ld.lld-19 --version && /usr/lib/go-1.24/bin/go version
