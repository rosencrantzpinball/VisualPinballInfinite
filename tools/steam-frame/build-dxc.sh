#!/usr/bin/env bash
# Builds DXC, the HLSL compiler that turns the player's shaders into SPIR-V, from its source at a release tag,
# in a throw-away container of the base image, into $WORK/build/dxc-install (bin/dxc, lib/libdxcompiler.so).
# image.sh copies that into the image. Takes about half an hour the first time.
set -euo pipefail
source "$(dirname "$0")/env.sh"
TAG=v1.9.2609
SRC=$WORK/src/dxc
BUILD=$WORK/build/dxc
INSTALL=$WORK/build/dxc-install

if [[ ! -d $SRC ]]; then
   git clone -q --depth 1 --branch "$TAG" --recurse-submodules --shallow-submodules \
      https://github.com/microsoft/DirectXShaderCompiler.git "$SRC"
fi
[[ $(git -C "$SRC" describe --tags) == "$TAG" ]] || { echo "$SRC is not at $TAG" >&2; exit 1; }
mkdir -p "$BUILD" "$INSTALL"
IMAGE_FOR_RUN=$BASE_IMAGE in_image bash -c "
   set -e
   cmake -S '$SRC' -B '$BUILD' -G Ninja -C '$SRC/cmake/caches/PredefinedParams.cmake' \
      -DCMAKE_BUILD_TYPE=Release -DCMAKE_C_COMPILER=clang -DCMAKE_CXX_COMPILER=clang++ \
      -DENABLE_SPIRV_CODEGEN=ON -DSPIRV_BUILD_TESTS=OFF -DHLSL_INCLUDE_TESTS=OFF \
      -DLLVM_INCLUDE_TESTS=OFF -DCLANG_INCLUDE_TESTS=OFF -DLLVM_INCLUDE_EXAMPLES=OFF
   ninja -C '$BUILD' dxc
   mkdir -p '$INSTALL/bin' '$INSTALL/lib'
   cp -a '$BUILD'/bin/dxc* '$INSTALL/bin/'
   cp -a '$BUILD'/lib/libdxcompiler.so* '$INSTALL/lib/'
   strip '$INSTALL'/bin/dxc-* '$INSTALL'/lib/libdxcompiler.so.* 2>/dev/null || true
   '$INSTALL/bin/dxc' --version
"
