#!/usr/bin/env bash
# Builds libwinevbs for Linux (arm64, for the Steam Frame), as setup.ps1 builds it for Windows: the same commit and
# the same patches, with CMake as VPX standalone's own Linux build runs it (-DPLATFORM=linux). Runs inside the image
# vpi-steamrt4-arm64 (tools/steam-frame/in-image.sh), which has the compiler, CMake and Bison.
#
#   1. Clones libwinevbs at the commit we follow into vpx-build/linux-aarch64/libwinevbs (git-ignored), on local
#      branch vpi, and applies the patches in ./patches.
#   2. Builds libwinevbs.so in its build folder.
#
# The player finds the result there in a development build (engine/player/src/script/winevbs.rs); a package carries
# it beside the player. Safe to re-run: an existing copy is rebuilt, not cloned again.
set -euo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
SRC=$ROOT/vpx-build/linux-aarch64/libwinevbs
COMMIT=e8f15a759c345f8ff535bacf2fe6045cc59f5f8c # atl: return typeinfo from GetClassInfo (0.10.2, Wine 11.20's code)

if [[ ! -d $SRC ]]; then
   echo "Cloning libwinevbs at $COMMIT ..."
   git clone -q https://github.com/vpinball/libwinevbs.git "$SRC"
   git -C "$SRC" -c advice.detachedHead=false checkout -q "$COMMIT"
   git -C "$SRC" switch -q -c vpi
   echo 'Applying patches ...'
   git -C "$SRC" -c user.name=VPinInfinite -c user.email=340424849+VPinInfinite@users.noreply.github.com am -q "$HERE"/patches/*.patch
else
   echo "Source already at $SRC, skipping clone and patches."
fi

cmake -S "$SRC" -B "$SRC/build" -DCMAKE_BUILD_TYPE=Release -DPLATFORM=linux -DARCH=aarch64 \
   "-DCMAKE_PROJECT_INCLUDE=$ROOT/tools/cmake/trim-paths.cmake"
cmake --build "$SRC/build" -- -j"$(nproc)"
echo "Done: $SRC/build/libwinevbs.so"
