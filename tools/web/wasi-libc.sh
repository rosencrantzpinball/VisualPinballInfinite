#!/usr/bin/env bash
# Builds wasi-libc from source for the browser build's C (libwinevbs): its headers and libraries, into
# WORK/build/wasi-libc/sysroot. Runs inside the image vpi-steamrt4-arm64 (tools/steam-frame/in-image.sh), whose
# clang 19 has the WebAssembly backend. wasi-sdk-28's release, the last built by its Makefile (later ones fetch
# prebuilt tools); compiler-rt isn't fetched either: an empty stand-in for it, which only wasi-libc's shared
# libraries and tests would need. The program links Rust's own copy of the same library.
set -euo pipefail
WORK=${VPI_FRAME_WORK:-/Volumes/storage/personal/vpi-frame}
SRC=$WORK/src/wasi-libc
OUT=$WORK/build/wasi-libc
TAG=wasi-sdk-28 # d181bdd429f0067c92d817909b01e4970ad7d70a
export PATH=/usr/lib/llvm-19/bin:$PATH
if [[ ! -d $SRC ]]; then
   git clone -q https://github.com/WebAssembly/wasi-libc.git "$SRC"
fi
git -C "$SRC" -c advice.detachedHead=false checkout -q "$TAG"
mkdir -p "$OUT"
llvm-ar rc "$OUT/empty-builtins.a"
make -C "$SRC" -j"$(nproc)" CC=clang AR=llvm-ar NM=llvm-nm TARGET_TRIPLE=wasm32-wasip1 THREAD_MODEL=single \
   BUILTINS_LIB="$OUT/empty-builtins.a" SYSROOT="$OUT/sysroot" OBJDIR="$OUT/obj" libc
echo "Done: $OUT/sysroot"
