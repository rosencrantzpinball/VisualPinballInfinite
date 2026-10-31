#!/bin/sh
# Visual Pinball Infinite (Frame test): the start command the SteamOS devkit registers (argv ["run.sh"]), run by
# Steam in "Steam Linux Runtime 4.0 ARM64" (SteamLinuxRuntime_4-arm64). Plays a table in VR.
#
#   run.sh [table.vpx] [player options...]
#
# With the headset's panel beside the player (panel/vpi-panel), and no table given: the library, the app's screens on
# a panel in the room (vpi-player --library), where a table is chosen. Else the table: the one given, else the one
# named in ~/.local/share/VisualPinballInfinite/frame-table.txt (the last one chosen), else Symphony IX 1.0.2 in
# ~/Documents/VPinballX/Tables. Tables are read there, never written. The player keeps its
# settings and what tables save in ~/.local/share/VisualPinballInfinite, its log there as vpi-player.log (the
# previous one as vpi-player.1.log), and what's changed for a table (its options and its view, in the pause menu
# or the library's table settings) in its table-settings folder: VPI's copy of the table's own <table>.ini,
# started from the one beside the table, read instead of it from then on.
cd "$(dirname "$0")" || exit 1
DATA=${VPI_DATA:-${XDG_DATA_HOME:-$HOME/.local/share}/VisualPinballInfinite}
mkdir -p "$DATA"

# the libraries beside the player (libwinevbs, the OpenXR loader)
export LD_LIBRARY_PATH="$PWD${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
# tables only read: their own settings saved into VPI's folder (PLAN decision 9), a value already set wins
export VPI_TABLE_SETTINGS="${VPI_TABLE_SETTINGS:-$DATA/table-settings}"
# Turnip, the Frame's Vulkan driver, measures tiled against direct rendering for each render pass and keeps the
# faster (the VPX fork's finding: 14.0 to 9.1 ms on a heavy table); a value already set wins
export TU_AUTOTUNE_ALGO="${TU_AUTOTUNE_ALGO:-profiled}"
# no desktop window at all: SDL's offscreen video (a window presented beside the headset held its frames back)
export SDL_VIDEO_DRIVER="${SDL_VIDEO_DRIVER:-offscreen}"
# the player foveates with shading rate images of its own; SteamVR's FDM injection layer would add density maps
# to the passes into the headset's images, which can't go with them (the fork turned it off the same way)
export DISABLE_VULKAN_FDM_INJECTION_LAYER="${DISABLE_VULKAN_FDM_INJECTION_LAYER:-1}"

TABLE=
case ${1:-} in
   *.vpx) TABLE=$1; shift ;;
esac
MODE=table
if [ -z "$TABLE" ] && [ -x ./panel/vpi-panel ]; then
   MODE=library
fi
if [ -z "$TABLE" ] && [ -f "$DATA/frame-table.txt" ]; then
   TABLE=$(head -n 1 "$DATA/frame-table.txt")
fi
if [ -z "$TABLE" ]; then
   TABLE="$HOME/Documents/VPinballX/Tables/Symphony IX 1.0.2/Symphony IX 1.0.2.vpx"
fi

if [ -f "$DATA/vpi-player.log" ]; then
   mv -f "$DATA/vpi-player.log" "$DATA/vpi-player.1.log"
fi
# the player ends with 75 when a table was chosen (in its menu, or in the library), which it wrote into
# frame-table.txt, and with 76 when the library was (its menu's): one OpenXR session per process on SteamVR, so
# a table is changed by starting again
while :; do
   if [ "$MODE" = library ]; then
      ./vpi-player --library "$@" >> "$DATA/vpi-player.log" 2>&1
   else
      ./vpi-player "$TABLE" --vr "$@" >> "$DATA/vpi-player.log" 2>&1
   fi
   code=$?
   if [ "$code" -eq 76 ] && [ -x ./panel/vpi-panel ]; then
      MODE=library
      echo "--- again, the library" >> "$DATA/vpi-player.log"
      continue
   fi
   if [ "$code" -ne 75 ] || [ ! -f "$DATA/frame-table.txt" ]; then
      exit "$code"
   fi
   MODE=table
   TABLE=$(head -n 1 "$DATA/frame-table.txt")
   echo "--- again, with $TABLE" >> "$DATA/vpi-player.log"
done
