#!/usr/bin/env bash
# Plays a table with the Linux player in the image, in a window on Xvfb drawn by lavapipe (Mesa's Vulkan on
# the processor), with a settings and data folder of its own: the flat test run of the Steam Frame port.
#
#   tools/steam-frame/flat.sh <run folder> <table.vpx> [player options...]
#
# The run folder gets log.txt and data/ (VPI_DATA: settings, saved values); a --shot picture goes where the
# options say. The player binary is target/release/vpi-player (build it first).
set -euo pipefail
source "$(dirname "$0")/env.sh"
RUN=$(mkdir -p "$1" && cd "$1" && pwd)
TABLE=$2
shift 2
in_image bash -c '
   export VPI_DATA="$1/data" SDL_AUDIO_DRIVER=dummy XDG_RUNTIME_DIR=/tmp/xdg
   mkdir -p "$VPI_DATA" /tmp/xdg && chmod 700 /tmp/xdg
   shift
   xvfb-run -a -s "-screen 0 2560x1440x24" target/release/vpi-player "$@"
' run "$RUN" "$TABLE" "$@" > "$RUN/log.txt" 2>&1
