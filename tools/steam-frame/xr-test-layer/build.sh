#!/usr/bin/env bash
# Builds the OpenXR test layer (xr_test_layer.c) in the image, against the OpenXR SDK's headers, into
# $WORK/build/xr-test-layer: the library and its manifest, which versus.sh sends to the Frame.
set -euo pipefail
source "$(dirname "$0")/../env.sh"
OUT=$WORK/build/xr-test-layer
mkdir -p "$OUT"
in_image gcc -std=gnu17 -O2 -Wall -Wextra -Werror -fPIC -shared -fvisibility=hidden \
   -I "$WORK/src/openxr-sdk/include" -o "$OUT/libxr_test_layer.so" \
   "$REPO/tools/steam-frame/xr-test-layer/xr_test_layer.c" -lm -lpthread
cp "$REPO/tools/steam-frame/xr-test-layer/XR_APILAYER_VPI_test.json" "$OUT/"
echo "built $OUT/libxr_test_layer.so"
