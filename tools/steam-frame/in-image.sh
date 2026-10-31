#!/usr/bin/env bash
# Runs a command inside the image vpi-steamrt4-arm64, with the workspace and the repository mounted at their own
# paths, Cargo's home on the workspace, and the repository as the working folder:
#   tools/steam-frame/in-image.sh cargo build --release -p player
set -euo pipefail
source "$(dirname "$0")/env.sh"
in_image "$@"
