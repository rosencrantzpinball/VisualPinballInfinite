#!/usr/bin/env bash
# Builds the Khronos OpenXR loader (libopenxr_loader.so.1, Apache-2.0) for Linux arm64 from the OpenXR SDK at a
# release tag, as VPX's Linux builds do, into vpx-build/linux-aarch64/openxr. The player loads it at run time
# for VR (engine/player/src/xr); a package carries it beside the player. Runs inside the image
# vpi-steamrt4-arm64 (tools/steam-frame/in-image.sh). Safe to re-run.
set -euo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
OUT=$ROOT/vpx-build/linux-aarch64/openxr
SRC=$OUT/OpenXR-SDK
TAG=release-1.1.65

if [[ ! -d $SRC ]]; then
   git clone -q --depth 1 --branch "$TAG" https://github.com/KhronosGroup/OpenXR-SDK.git "$SRC"
fi
cmake -S "$SRC" -B "$OUT/build" -G Ninja -DCMAKE_BUILD_TYPE=Release -DBUILD_TESTS=OFF -DDYNAMIC_LOADER=ON \
   -DBUILD_WITH_SYSTEM_JSONCPP=OFF "-DCMAKE_PROJECT_INCLUDE=$ROOT/tools/cmake/trim-paths.cmake"
cmake --build "$OUT/build"
cp -a "$OUT"/build/src/loader/libopenxr_loader.so* "$OUT/"
echo "Done: $(ls "$OUT"/libopenxr_loader.so.*.*)"
