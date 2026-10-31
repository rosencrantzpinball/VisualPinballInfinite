#!/usr/bin/env bash
# VPI against the owner's VPX fork (as installed on the Frame) with the headset worn, from the Mac: the same tables,
# one player after the other, each measured the same way by versus.py on the Frame (see there). Both are played from
# copies of the tables in the test folder, the fork with a copy of the owner's settings: nothing of the owner's is
# written. Each frame of both is timed by an OpenXR test layer (xr-test-layer/). Runs only when the owner isn't playing.
#
#   tools/steam-frame/versus.sh prepare               copies of the tables and the scripts into ~/vpi-test/versus
#   tools/steam-frame/versus.sh start <name> <plan>    starts the runs on the Frame: offline (unworn, both drawn with the
#                                                      eyes pinned by the test layer), check (the same, short), worn
#                                                      (each waits for the headset); look... (one worn run on
#                                                      STAR FOX, to look down: see the plans)
#   tools/steam-frame/versus.sh start <name> <file>    the same with a plan of one's own (versus.py's JSON)
#   tools/steam-frame/versus.sh wait <name> [seconds]  until they end (bounded), then their files back to
#                                                      $WORK/runs/frame/<name> and what changed in the owner's folders
#   tools/steam-frame/versus.sh stop <name>            ends them early (the player running is asked to close)
set -euo pipefail
source "$(dirname "$0")/env.sh"
HERE=$(cd "$(dirname "$0")" && pwd)
REMOTE=/home/steamos/vpi-test/versus
TABLES=/home/steamos/Documents/VPinballX/Tables
SIX="Symphony IX 1.0.2"
FOX="STAR FOX – BATTLE FOR LYLAT"

ssh_run() { ssh -o ConnectTimeout=15 -o BatchMode=yes "$FRAME" "$@"; }

# the owner's folders (tables, the fork's settings and VPI's), listed before and after: the runs change nothing there
owned() {
   ssh_run "find '$TABLES' ~/.local/share/VPinballX ~/.local/share/VisualPinballInfinite -printf '%p\t%s\t%T@\n' 2>/dev/null | sort"
}

plan() {
   local kind=$1 six="$REMOTE/tables/$SIX/$SIX.vpx" fox="$REMOTE/tables/$FOX/$FOX.vpx"
   # both at fixed sizes (each table's own: Symphony IX's .ini asks 1920 px an eye, STAR FOX has none: the runtime's
   # 1728), the rest as each player has it (FXAA, specular AA, foveation at the table's level, eye-tracked when worn);
   # every frame timed by the test layer, which also pins the eyes and has them drawn when the headset is unworn
   local fixed='"settings": {"PlayerVR.DynamicResolution": "0"}' pin='"layer": {"pin": true}' timed='"layer": {}'
   case $kind in
      offline) cat <<EOF
{"deadline": 1200, "gap": 6, "runs": [
 {"name": "vpi-six", "player": "vpi", "table": "$six", "wait": 90, "warm": 10, "measure": 45, $fixed, $pin},
 {"name": "fork-six", "player": "fork", "table": "$six", "wait": 120, "warm": 10, "measure": 45, $fixed, $pin},
 {"name": "fork-fox", "player": "fork", "table": "$fox", "wait": 150, "warm": 10, "measure": 45, $fixed, $pin},
 {"name": "vpi-fox", "player": "vpi", "table": "$fox", "wait": 90, "warm": 10, "measure": 45, $fixed, $pin},
 {"name": "fork-six-2", "player": "fork", "table": "$six", "wait": 120, "warm": 10, "measure": 45, $fixed, $pin},
 {"name": "vpi-six-2", "player": "vpi", "table": "$six", "wait": 90, "warm": 10, "measure": 45, $fixed, $pin}
]}
EOF
      ;;
      check) cat <<EOF
{"deadline": 400, "gap": 6, "runs": [
 {"name": "vpi-six", "player": "vpi", "table": "$six", "wait": 90, "warm": 5, "measure": 10, $fixed, $pin},
 {"name": "fork-six", "player": "fork", "table": "$six", "wait": 120, "warm": 5, "measure": 10, $fixed, $pin}
]}
EOF
      ;;
      worn) cat <<EOF
{"deadline": 1500, "gap": 4, "runs": [
 {"name": "vpi-six", "player": "vpi", "table": "$six", "wait": 1200, "warm": 10, "measure": 30, $fixed, $timed},
 {"name": "fork-six", "player": "fork", "table": "$six", "wait": 120, "warm": 10, "measure": 30, $fixed, $timed},
 {"name": "fork-fox", "player": "fork", "table": "$fox", "wait": 150, "warm": 10, "measure": 30, $fixed, $timed},
 {"name": "vpi-fox", "player": "vpi", "table": "$fox", "wait": 120, "warm": 10, "measure": 30, $fixed, $timed}
]}
EOF
      ;;
      steps) cat <<EOF
{"deadline": 600, "gap": 6, "runs": [
 {"name": "vpi-steps-profiled", "player": "vpi", "table": "$six", "wait": 90, "warm": 8, "measure": 40, $pin, "env": {"VPI_XR_SCALE_STEPS": "0.85,1.0,2"}},
 {"name": "vpi-steps-sysmem", "player": "vpi", "table": "$six", "wait": 90, "warm": 8, "measure": 40, $pin, "env": {"VPI_XR_SCALE_STEPS": "0.85,1.0,2", "TU_AUTOTUNE_ALGO": "prefer_sysmem"}},
 {"name": "vpi-fixed-sysmem", "player": "vpi", "table": "$six", "wait": 90, "warm": 8, "measure": 40, $fixed, $pin, "env": {"TU_AUTOTUNE_ALGO": "prefer_sysmem"}}
]}
EOF
      ;;
      dynres) cat <<EOF
{"deadline": 600, "gap": 6, "runs": [
 {"name": "vpi-dynres", "player": "vpi", "table": "$six", "wait": 90, "warm": 8, "measure": 60, $pin, "settings": {"PlayerVR.DynamicResolutionTarget": "0.6"}},
 {"name": "vpi-steps", "player": "vpi", "table": "$six", "wait": 90, "warm": 8, "measure": 40, $pin, "env": {"VPI_XR_SCALE_STEPS": "0.85,1.0,2"}}
]}
EOF
      ;;
      trace) local traced='"env": {"MESA_GPU_TRACES": "print"}'; cat <<EOF
{"deadline": 600, "gap": 6, "runs": [
 {"name": "vpi-six", "player": "vpi", "table": "$six", "wait": 90, "warm": 8, "measure": 5, $fixed, $pin, $traced},
 {"name": "fork-six", "player": "fork", "table": "$six", "wait": 120, "warm": 8, "measure": 5, $fixed, $pin, $traced},
 {"name": "fork-fox", "player": "fork", "table": "$fox", "wait": 150, "warm": 8, "measure": 5, $fixed, $pin, $traced},
 {"name": "vpi-fox", "player": "vpi", "table": "$fox", "wait": 90, "warm": 8, "measure": 5, $fixed, $pin, $traced}
]}
EOF
      ;;
      trace-fox) local traced='"env": {"MESA_GPU_TRACES": "print"}'; cat <<EOF
{"deadline": 600, "gap": 6, "runs": [
 {"name": "fork-fox", "player": "fork", "table": "$fox", "wait": 150, "warm": 8, "measure": 5, $fixed, $pin, $traced},
 {"name": "vpi-fox", "player": "vpi", "table": "$fox", "wait": 90, "warm": 8, "measure": 5, $fixed, $pin, $traced}
]}
EOF
      ;;
      vpi) cat <<EOF
{"deadline": 900, "gap": 6, "runs": [
 {"name": "vpi-six", "player": "vpi", "table": "$six", "wait": 90, "warm": 10, "measure": 45, $fixed, $pin},
 {"name": "vpi-fox", "player": "vpi", "table": "$fox", "wait": 90, "warm": 10, "measure": 45, $fixed, $pin},
 {"name": "vpi-six-2", "player": "vpi", "table": "$six", "wait": 90, "warm": 10, "measure": 45, $fixed, $pin}
]}
EOF
      ;;
      aoi) local whole='"env": {"VPI_XR_NO_MIRROR_AREA": "1"}'; cat <<EOF
{"deadline": 900, "gap": 30, "runs": [
 {"name": "six-area", "player": "vpi", "table": "$six", "wait": 90, "warm": 8, "measure": 20, $fixed, $pin},
 {"name": "six-whole", "player": "vpi", "table": "$six", "wait": 90, "warm": 8, "measure": 20, $fixed, $pin, $whole},
 {"name": "six-area-2", "player": "vpi", "table": "$six", "wait": 90, "warm": 8, "measure": 20, $fixed, $pin},
 {"name": "six-whole-2", "player": "vpi", "table": "$six", "wait": 90, "warm": 8, "measure": 20, $fixed, $pin, $whole}
]}
EOF
      ;;
      clip) local old='"env": {"VPI_TEST_CLIP_EVERYWHERE": "1"}'; cat <<EOF
{"deadline": 900, "gap": 20, "runs": [
 {"name": "fox-twins", "player": "vpi", "table": "$fox", "wait": 90, "warm": 8, "measure": 20, $fixed, $pin},
 {"name": "fox-clip", "player": "vpi", "table": "$fox", "wait": 90, "warm": 8, "measure": 20, $fixed, $pin, $old},
 {"name": "six-twins", "player": "vpi", "table": "$six", "wait": 90, "warm": 8, "measure": 20, $fixed, $pin},
 {"name": "six-clip", "player": "vpi", "table": "$six", "wait": 90, "warm": 8, "measure": 20, $fixed, $pin, $old}
]}
EOF
      ;;
      oblique) local twins='"env": {"VPI_TEST_NO_OBLIQUE": "1"}'; cat <<EOF
{"deadline": 900, "gap": 20, "runs": [
 {"name": "fox-oblique", "player": "vpi", "table": "$fox", "wait": 90, "warm": 8, "measure": 20, $fixed, $pin},
 {"name": "fox-twins", "player": "vpi", "table": "$fox", "wait": 90, "warm": 8, "measure": 20, $fixed, $pin, $twins},
 {"name": "six-oblique", "player": "vpi", "table": "$six", "wait": 90, "warm": 8, "measure": 20, $fixed, $pin},
 {"name": "six-twins", "player": "vpi", "table": "$six", "wait": 90, "warm": 8, "measure": 20, $fixed, $pin, $twins}
]}
EOF
      ;;
      worn-blend) local opaque='"args": ["--vr-opaque"]'; cat <<EOF
{"deadline": 1500, "gap": 4, "runs": [
 {"name": "vpi-alpha", "player": "vpi", "table": "$six", "wait": 1200, "warm": 8, "measure": 30, $fixed, $timed},
 {"name": "vpi-opaque", "player": "vpi", "table": "$six", "wait": 120, "warm": 8, "measure": 30, $fixed, $timed, $opaque},
 {"name": "vpi-alpha-2", "player": "vpi", "table": "$six", "wait": 120, "warm": 8, "measure": 30, $fixed, $timed},
 {"name": "vpi-opaque-2", "player": "vpi", "table": "$six", "wait": 120, "warm": 8, "measure": 30, $fixed, $timed, $opaque}
]}
EOF
      ;;
      look|look-fork|look-blend|look-nocull|look-low)
         # one run at a time, worn, to look down at STAR FOX: VPI as it now draws (opaque), the fork, VPI with
         # passthrough and the table's room as it first drew them, VPI without culling, VPI at a lower size
         local player=vpi extra=''
         case $kind in
            look-fork) player=fork ;;
            look-blend) extra=', "args": ["--vr-passthrough"], "env": {"VPI_TEST_PASSTHROUGH_ROOM": "1"}' ;;
            look-nocull) extra=', "env": {"VPI_XR_NO_CULL": "1"}' ;;
            look-low) extra=', "env": {"VPI_XR_SCALE": "0.75"}' ;;
         esac
         cat <<EOF
{"deadline": 1500, "gap": 4, "runs": [
 {"name": "$kind", "player": "$player", "table": "$fox", "wait": 1200, "warm": 8, "measure": 75, $fixed, $timed$extra}
]}
EOF
      ;;
      *) echo "no plan $kind (offline, check, worn, worn-blend, look, look-fork, look-blend, look-nocull, look-low, steps, dynres, trace, trace-fox, vpi, aoi, clip, oblique)" >&2; return 1 ;;
   esac
}

case ${1:-} in
   prepare)
      "$HERE/frame.sh" idle
      ssh_run "df -h /home | tail -1; mkdir -p '$REMOTE/tables'"
      for t in "$SIX" "$FOX"; do
         ssh_run "[ -d '$REMOTE/tables/$t' ] || cp -a '$TABLES/$t' '$REMOTE/tables/'"
      done
      "$HERE/xr-test-layer/build.sh"
      ssh_run "mkdir -p '$REMOTE/xr-test-layer'"
      scp -q -o BatchMode=yes "$HERE/versus.py" "$HERE/fork-run.sh" "$FRAME:$REMOTE/"
      scp -q -o BatchMode=yes "$WORK/build/xr-test-layer/libxr_test_layer.so" "$WORK/build/xr-test-layer/XR_APILAYER_VPI_test.json" \
         "$FRAME:$REMOTE/xr-test-layer/"
      ssh_run "chmod +x '$REMOTE/fork-run.sh'; du -sh '$REMOTE/tables'/*"
      ;;
   start)
      name=$2 kind=$3
      "$HERE/frame.sh" idle
      ssh_run "[ ! -e '$REMOTE/$name' ]" || { echo "$name exists on the Frame already"; exit 1; }
      # a plan of our own: a JSON file (tables under $REMOTE/tables), else one of those above
      if [[ -f $kind ]]; then cat "$kind"; else plan "$kind"; fi | ssh_run "cat > '$REMOTE/$name.json'"
      owned | ssh_run "cat > '$REMOTE/$name-owned-before.txt'"
      ssh_run "cd '$REMOTE' && (setsid nohup python3 versus.py '$REMOTE/$name' '$REMOTE/$name.json' > '$REMOTE/$name.log' 2>&1 < /dev/null & echo started \$!)"
      ;;
   wait)
      name=$2 limit=${3:-1800} waited=0
      until ssh_run "[ -e '$REMOTE/$name/done' ]"; do
         if (( waited >= limit )); then
            echo "not done after ${limit}s: stopping it"
            ssh_run "touch '$REMOTE/$name/stop'"
            sleep 40
            break
         fi
         sleep 10
         waited=$((waited + 10))
      done
      owned | ssh_run "cat > '$REMOTE/$name-owned-after.txt'"
      mkdir -p "$WORK/runs/frame/$name"
      rsync -a -e "ssh -o ConnectTimeout=15 -o BatchMode=yes" "$FRAME:$REMOTE/$name/" "$WORK/runs/frame/$name/"
      for f in "$name.json" "$name.log" "$name-owned-before.txt" "$name-owned-after.txt"; do
         scp -q -o BatchMode=yes "$FRAME:$REMOTE/$f" "$WORK/runs/frame/$name/" || true
      done
      echo "the owner's folders, what changed:"
      diff "$WORK/runs/frame/$name/$name-owned-before.txt" "$WORK/runs/frame/$name/$name-owned-after.txt" && echo "(nothing)"
      cat "$WORK/runs/frame/$name/$name.log"
      ;;
   stop)
      ssh_run "touch '$REMOTE/$2/stop'"
      ;;
   *)
      sed -n '2,15p' "$0" | sed 's/^# \{0,1\}//'
      exit 2
      ;;
esac
