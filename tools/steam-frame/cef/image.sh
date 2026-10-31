#!/usr/bin/env bash
# Builds the image vpi-cef-build-arm64 (Dockerfile beside this script) from the player's vpi-steamrt4-arm64.
set -euo pipefail
source "$(dirname "$0")/env.sh"
CONTEXT=$CEF_WORK/build/image-context
mkdir -p "$CONTEXT"
cp "$(dirname "$0")/Dockerfile" "$CONTEXT/Dockerfile"
echo "system disk before: $(df -h / | awk 'NR==2 {print $4}') free"
docker build --platform linux/arm64 -t "$CEF_IMAGE:latest" "$CONTEXT"
echo "system disk after: $(df -h / | awk 'NR==2 {print $4}') free"
