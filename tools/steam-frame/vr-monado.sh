#!/usr/bin/env bash
# Plays a table in VR in the image, headless: Monado's simulated headset and null compositor (built by
# build-monado.sh) as the OpenXR runtime, lavapipe drawing, a settings and data folder of the run's own.
#
#   tools/steam-frame/vr-monado.sh <run folder> <table.vpx> [player options...]
#
# The player is started with --vr; add --vr-capture <folder> --vr-capture-at <ms> for the eyes' pictures,
# (VPI_XR_ENVIRONMENT_ALPHA, VPI_XR_SCALE, VPI_XR_FOVEA, VPI_XR_FOVEA_DEBUG, VPI_XR_NO_CULL, VPI_XR_NO_SPECULAR_AA and
# VPI_PANEL_CLICKS from the caller's environment pass through). `--library` in the table's place: the headset's panel
# alone, on ~/Documents/VPinballX in the image's home (WORK/home)
# --vr-timings <file> for each frame's, --quit-after <ms> to end. The run folder gets log.txt and data/.
set -euo pipefail
source "$(dirname "$0")/env.sh"
RUN=$(mkdir -p "$1" && cd "$1" && pwd)
TABLE=$2
shift 2
in_image bash -c '
   export VPI_DATA="$1/data" SDL_AUDIO_DRIVER=dummy XDG_RUNTIME_DIR=/tmp/xdg
   export XR_RUNTIME_JSON='"$WORK"'/build/monado-install/share/openxr/1/openxr_monado.json
   export SIMULATED_ENABLE=1 XRT_COMPOSITOR_NULL=1 VPI_XR_ENVIRONMENT_ALPHA='"${VPI_XR_ENVIRONMENT_ALPHA:-}"'
   export VPI_XR_SCALE='"${VPI_XR_SCALE:-}"' VPI_XR_FOVEA='"${VPI_XR_FOVEA:-}"' VPI_XR_FOVEA_DEBUG='"${VPI_XR_FOVEA_DEBUG:-}"'
   [ -n '"${VPI_PANEL_CLICKS:-}"' ] && export VPI_PANEL_CLICKS='"${VPI_PANEL_CLICKS:-}"'
   [ -n '"${VPI_XR_NO_CULL:-}"' ] && export VPI_XR_NO_CULL=1
   [ -n '"${VPI_XR_NO_SPECULAR_AA:-}"' ] && export VPI_XR_NO_SPECULAR_AA=1
   mkdir -p "$VPI_DATA" /tmp/xdg && chmod 700 /tmp/xdg
   shift
   xvfb-run -a -s "-screen 0 1280x720x24" target/release/vpi-player "$@" --vr
' run "$RUN" "$TABLE" "$@" > "$RUN/log.txt" 2>&1
