# Shared settings for building CEF for the Steam Frame (sourced by the scripts beside this file).
#
# CEF 154.0.34, the version the `cef` crate 154.5.0 binds, on Chromium 154.0.8037.98, built natively on arm64
# from Chromium's release source tarball (as Linux distributions build Chromium) with clang built from the LLVM
# commit Chromium 154 pins. Everything stays under CEF_WORK on the data volume: the downloads (dl/), the sources
# (src/), the toolchain built (toolchain/), its build (build/) and the logs (logs/).
source "$(dirname "${BASH_SOURCE[0]}")/../env.sh"
CEF_WORK=${VPI_CEF_WORK:-$WORK/cef}
CEF_IMAGE=vpi-cef-build-arm64
CEF_VERSION=154.0.34+g14c5a08+chromium-154.0.8037.98
CEF_COMMIT=14c5a089e8452874cb1a556dc3fde26ed2d87723
CHROMIUM_VERSION=154.0.8037.98
# Chromium 154's tools/clang/scripts/update.py: CLANG_REVISION llvmorg-24-init-3796-g20e97c4b, RELEASE_VERSION 24
LLVM_COMMIT=20e97c4b113b79586c29ac57738548939a34d22e
CLANG_VERSION=24
# the Go that builds tsgo and runs Dawn's generators (Chromium 154's need Go 1.26), and the Go it's bootstrapped from
GO=go1.26.9
GO_BOOTSTRAP=go1.25.13
CHROMIUM_SRC=$CEF_WORK/src/chromium/src

# Runs a command in the CEF build image, with WORK and the repository mounted at their own paths (the container
# named CEF_CONTAINER when that's set, for watch-disk.sh to stop). Temporary files go to the data volume too, and
# Docker keeps no log of the output (it still comes here): in the container's /tmp or Docker's log they'd grow
# OrbStack's disk image, which is on the Mac's system disk.
in_cef_image() {
   mkdir -p "$CEF_WORK/tmp"
   docker run --rm --init ${DOCKER_TTY:-} --log-driver none --platform linux/arm64 ${CEF_CONTAINER:+--name "$CEF_CONTAINER"} \
      -v "$WORK:$WORK" -v "$REPO:$REPO" \
      -e CARGO_HOME="$WORK/cargo" -e HOME="$CEF_WORK/home" -e TMPDIR="$CEF_WORK/tmp" \
      -w "$CEF_WORK" "$CEF_IMAGE" "$@"
}
