#!/usr/bin/env bash
# Builds the image vpi-steamrt4-arm64 (Dockerfile beside this script) from the local vpinball-steamrt4-arm64,
# with the DXC that build-dxc.sh built. `image.sh rust` builds only its first stage (no DXC), tagged :rust.
set -euo pipefail
source "$(dirname "$0")/env.sh"
CONTEXT=$WORK/build/image-context
mkdir -p "$CONTEXT/dxc-install"
cp "$(dirname "$0")/Dockerfile" "$CONTEXT/Dockerfile"
echo "system disk before: $(df -h / | awk 'NR==2 {print $4}') free"
if [[ ${1:-} == rust ]]; then
   docker build --platform linux/arm64 --target rust -t "$IMAGE:rust" "$CONTEXT"
else
   [[ -x $WORK/build/dxc-install/bin/dxc ]] || { echo "build DXC first (build-dxc.sh)" >&2; exit 1; }
   rsync -a --delete "$WORK/build/dxc-install/" "$CONTEXT/dxc-install/"
   docker build --platform linux/arm64 -t "$IMAGE:latest" "$CONTEXT"
fi
echo "system disk after: $(df -h / | awk 'NR==2 {print $4}') free"
