#!/usr/bin/env bash
# Builds libwinevbs for the browser (wasm32-wasip1), as tools/libwinevbs/setup.sh builds it for Linux: the same
# commit and the same patches, a static library the player links in (engine/player/src/script/winevbs.rs finds its
# functions there). Runs inside the image vpi-steamrt4-arm64 (tools/steam-frame/in-image.sh), after wasi-libc.sh.
#
#   1. Clones libwinevbs at the commit we follow into vpx-build/wasm32/libwinevbs (git-ignored), on local branch
#      vpi, and applies the patches in tools/libwinevbs/patches, then the browser's own in tools/web/patches.
#   2. Builds libwinevbs.a in its build folder.
set -euo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
WORK=${VPI_FRAME_WORK:-/Volumes/storage/personal/vpi-frame}
SRC=$ROOT/vpx-build/wasm32/libwinevbs
COMMIT=e8f15a759c345f8ff535bacf2fe6045cc59f5f8c # atl: return typeinfo from GetClassInfo (0.10.2, Wine 11.20's code)
export PATH=/usr/lib/llvm-19/bin:$PATH WASI_SYSROOT=$WORK/build/wasi-libc/sysroot
[[ -f $WASI_SYSROOT/lib/wasm32-wasip1/libc.a ]] || { echo "build wasi-libc first (tools/web/wasi-libc.sh)" >&2; exit 1; }

if [[ ! -d $SRC ]]; then
   echo "Cloning libwinevbs at $COMMIT ..."
   git clone -q https://github.com/vpinball/libwinevbs.git "$SRC"
   git -C "$SRC" -c advice.detachedHead=false checkout -q "$COMMIT"
   git -C "$SRC" switch -q -c vpi
   echo 'Applying patches ...'
   git -C "$SRC" -c user.name=VPinInfinite -c user.email=340424849+VPinInfinite@users.noreply.github.com am -q "$ROOT"/tools/libwinevbs/patches/*.patch "$HERE"/patches/*.patch
else
   echo "Source already at $SRC, skipping clone and patches."
fi

cmake -S "$SRC" -B "$SRC/build" -G Ninja -DCMAKE_BUILD_TYPE=Release -DPLATFORM=wasm -DARCH=wasm32 \
   -DBUILD_SHARED=OFF -DBUILD_STATIC=ON \
   "-DCMAKE_TOOLCHAIN_FILE=$HERE/wasm32-wasip1.cmake" \
   "-DCMAKE_PROJECT_INCLUDE=$ROOT/tools/cmake/trim-paths.cmake"
cmake --build "$SRC/build"
echo "Done: $SRC/build/libwinevbs.a"
