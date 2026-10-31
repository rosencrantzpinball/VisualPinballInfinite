#!/usr/bin/env bash
# Builds Monado (the open OpenXR runtime, BSL-1.0) from its source at a release tag, as an in-process runtime
# with its simulated headset and null compositor, into $WORK/build/monado-install: the player's VR path is
# tested with it in the image, headless, before the headset (vr-test.sh). Eigen's headers, which only the
# build needs, are installed in the throw-away container that builds it.
set -euo pipefail
source "$(dirname "$0")/env.sh"
TAG=v25.1.0
SRC=$WORK/src/monado
BUILD=$WORK/build/monado
INSTALL=$WORK/build/monado-install

if [[ ! -d $SRC ]]; then
   git clone -q --depth 1 --branch "$TAG" https://gitlab.freedesktop.org/monado/monado.git "$SRC"
fi
mkdir -p "$BUILD" "$INSTALL"
in_image bash -c "
   set -e
   apt-get update -qq > /dev/null && apt-get install -y -qq --no-install-recommends libeigen3-dev glslang-tools > /dev/null
   cmake -S '$SRC' -B '$BUILD' -G Ninja -DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX='$INSTALL' \
      -DXRT_FEATURE_SERVICE=OFF -DXRT_MODULE_COMPOSITOR_NULL=ON -DXRT_BUILD_DRIVER_SIMULATED=ON \
      -DXRT_HAVE_OPENGL=OFF -DXRT_HAVE_OPENGLES=OFF -DXRT_HAVE_EGL=OFF -DXRT_HAVE_SDL2=OFF \
      -DXRT_HAVE_LIBUSB=OFF -DXRT_HAVE_HIDAPI=OFF -DXRT_HAVE_OPENCV=OFF -DXRT_HAVE_V4L2=OFF \
      -DXRT_FEATURE_TRACING=OFF -DBUILD_TESTING=OFF -DXRT_BUILD_DRIVER_QWERTY=OFF
   ninja -C '$BUILD' install
   ls '$INSTALL'/lib*/libopenxr_monado.so* '$INSTALL'/share/openxr/1/openxr_monado.json
"
