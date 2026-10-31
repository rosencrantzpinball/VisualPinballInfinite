#!/usr/bin/env bash
# Builds libpinmame for Linux (arm64, for the Steam Frame), as setup.ps1 builds pinmame64.dll for Windows: PinMAME
# at the commit VPX build 5436 uses, with the same patches, the library's own CMakeLists.txt at the top as VPX's
# build scripts use it (-DPLATFORM=linux -DARCH=aarch64), shared library only. Runs inside the image
# vpi-steamrt4-arm64 (tools/steam-frame/in-image.sh), which has the compiler and CMake.
#
#   1. Clones PinMAME at that commit into vpx-build/linux-aarch64/pinmame-5436 (git-ignored), on local branch vpi,
#      and applies the patches in ./patches (keeping the carriage returns of the files PinMAME keeps in CRLF).
#   2. Builds libpinmame.so in its build folder, with tools/cmake/trim-paths.cmake.
#
# The PinMAME host finds the result there in a development build (engine/pinmame-host/src/main.rs); a package
# carries it beside the host. Safe to re-run: an existing copy is rebuilt, not cloned again.
set -euo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
SRC=$ROOT/vpx-build/linux-aarch64/pinmame-5436
COMMIT=23321ec7de6dfd563a1c64153bc75f26e5059b9f # VPX 5436's PINMAME_SHA (platforms/config.sh)

if [[ ! -d $SRC ]]; then
   echo "Cloning PinMAME at $COMMIT ..."
   git clone -q --filter=blob:none --no-checkout "${PINMAME_MIRROR:-https://github.com/vpinball/pinmame.git}" "$SRC"
   git -C "$SRC" -c advice.detachedHead=false checkout -q "$COMMIT"
   git -C "$SRC" switch -q -c vpi
   echo 'Applying patches ...'
   git -C "$SRC" -c user.name=VPinInfinite -c user.email=340424849+VPinInfinite@users.noreply.github.com am -q --keep-cr "$HERE"/patches/*.patch
else
   echo "Source already at $SRC, skipping clone and patches."
fi

cp "$SRC/cmake/libpinmame/CMakeLists.txt" "$SRC/CMakeLists.txt"
cmake -S "$SRC" -B "$SRC/build" -DCMAKE_BUILD_TYPE=Release -DPLATFORM=linux -DARCH=aarch64 -DBUILD_SHARED=ON -DBUILD_STATIC=OFF \
   "-DCMAKE_PROJECT_INCLUDE=$ROOT/tools/cmake/trim-paths.cmake"
cmake --build "$SRC/build" --target pinmame_shared -- -j"$(nproc)"
echo "Done: $(ls "$SRC"/build/libpinmame.so* | head -1)"
