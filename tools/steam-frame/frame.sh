#!/usr/bin/env bash
# The Steam Frame's test harness, from the Mac, over ssh (key auth): install the build, start a run, wait for it
# (bounded), collect its logs, frame timings and pictures, stop it. Runs only when the owner isn't playing, and
# touches nothing but its own folder (~/vpi-test on the Frame) and its own processes.
#
#   tools/steam-frame/frame.sh idle                         is the Frame free? (exit 1 when something of the owner's runs)
#   tools/steam-frame/frame.sh install                      package.sh, then install.sh
#   tools/steam-frame/frame.sh stage                        package.sh, then the package to ~/vpi-test/game alone:
#        a copy to try (STAGED=1 below) before it's installed, the game in Steam's library left as it is
#   tools/steam-frame/frame.sh vr <name> <seconds> [table|library] [--set Section.Key=value...] [--env NAME=value...] [player options...]
#        a VR session started directly in Steam Linux Runtime 4 with the session's environment, the eyes pinned
#        where a player stands (drawn even unworn), pictures of both eyes at 5 s and 15 s, each frame's timings;
#        --set writes a setting into the run's own settings file (its data folder's Settings.ini), --env sets an
#        environment variable for the run (MESA_GPU_TRACES=print: Turnip's own trace of each render pass);
#        "library" in the table's place starts run.sh with none: the headset's panel on the Frame's library,
#        its pictures at those times (VPI_PANEL_CLICKS=ms:x,y;... in --env clicks on it)
#   tools/steam-frame/frame.sh headless <name> [table] [player options...]
#        the player on the Frame's GPU with no window and no OpenXR (SDL's offscreen video): a picture (--shot)
#   tools/steam-frame/frame.sh panel <name> [ms[,ms...]] [--env NAME=value...] [panel options...]
#        the headset's panel alone in Steam Linux Runtime 4, with no OpenXR (Chromium's headless platform, on the
#        GPU; on the session's X server with --env DISPLAY=:1, as Steam starts the game), on the Frame's library,
#        read only: its picture after ms (8000; several: shot-<ms>.png each) and CEF's log; --clicks ms:x,y;...
#        clicks on it
#   tools/steam-frame/frame.sh steam <name> <seconds>
#        the game started as the owner starts it, through the Steam client (steam://rungameid), then stopped
#   tools/steam-frame/frame.sh stop <name>                  ends a run of ours still going
#
# Runs start the installed game, or with STAGED=1 the staged copy. headless and panel runs, which start no
# OpenXR session, run beside what the owner does with ALLOW_BUSY=1, only on the owner's word.
#
# Each run's files come back to $WORK/runs/frame/<name>: launch.log, data/vpi-player.log, frames.csv, eyes/,
# temps-before.txt and temps-after.txt.
set -euo pipefail
source "$(dirname "$0")/env.sh"
REMOTE=/home/steamos/vpi-test
INSTALLED=/home/steamos/devkit-game/VisualPinballInfinite
STAGE=$REMOTE/game
GAME=$INSTALLED
[[ ${STAGED:-0} == 1 ]] && GAME=$STAGE
RUNTIME=/home/steamos/.local/share/Steam/steamapps/common/SteamLinuxRuntime_4-arm64
LIBRARY=/home/steamos/Documents/VPinballX
TABLE_DEFAULT="$LIBRARY/Tables/Symphony IX 1.0.2/Symphony IX 1.0.2.vpx"
# the session's own environment, as Steam's games get it (the user's bus and runtime folder)
SESSION_ENV='XDG_RUNTIME_DIR=/run/user/1000 DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus'

SETTINGS=()

ssh_run() { ssh -o ConnectTimeout=15 -o BatchMode=yes "$FRAME" "$@"; }

# What of the owner's runs: a game Steam launched (its reaper's "SteamLaunch"), VPX, or a player not ours.
busy() {
   ssh_run 'pgrep -af "[S]teamLaunch|[V]PinballX|[v]pinball|[v]pi-player" | grep -v "vpi-test/runs" || true'
}

check_idle() {
   local b
   b=$(busy)
   if [[ -n $b ]]; then
      echo "the Frame is busy (the owner may be playing):"
      echo "$b"
      return 1
   fi
}

# Runs that start no OpenXR session (headless, panel): beside what the owner does, on their word (ALLOW_BUSY=1).
check_idle_or_allowed() {
   check_idle || { [[ ${ALLOW_BUSY:-0} == 1 ]] && echo "running beside it (ALLOW_BUSY=1)"; }
}

temps() {
   ssh_run 'for z in /sys/class/thermal/thermal_zone*; do t=$(cat $z/type); case $t in gpuss-0-thermal|cpuss0-thermal|cpu4-bottom-thermal|ddr-thermal) echo "$t $(( $(cat $z/temp) / 1000 ))C";; esac; done; date -Is'
}

collect() {
   local name=$1
   mkdir -p "$WORK/runs/frame/$name"
   rsync -a -e "ssh -o ConnectTimeout=15 -o BatchMode=yes" "$FRAME:$REMOTE/runs/$name/" "$WORK/runs/frame/$name/"
   echo "collected into $WORK/runs/frame/$name"
}

# Waits for a run's process group to end, `limit` seconds at most, then ends it.
wait_run() {
   local name=$1 limit=$2 dir=$REMOTE/runs/$1 waited=0
   while ssh_run "kill -0 \$(cat $dir/pid) 2>/dev/null"; do
      if (( waited >= limit )); then
         echo "still running after ${limit}s: stopping it"
         stop_run "$name"
         break
      fi
      sleep 5
      waited=$((waited + 5))
   done
}

stop_run() {
   local dir=$REMOTE/runs/$1
   # the run's own process group (setsid), first asked, then made, to end
   ssh_run "pgid=\$(cat $dir/pid); kill -TERM -- -\$pgid 2>/dev/null; sleep 3; kill -KILL -- -\$pgid 2>/dev/null; true"
}

# The run's own settings file, from Section.Key=value items.
settings_ini() {
   local item section key value
   for item in "$@"; do
      section=${item%%.*}; key=${item#*.}; value=${key#*=}; key=${key%%=*}
      printf '[%s]\n%s = %s\n' "$section" "$key" "$value"
   done
}

start_run() {
   local name=$1 dir=$REMOTE/runs/$1
   shift
   ssh_run "rm -rf '$dir' && mkdir -p '$dir/data' && df -h /home | tail -1 > '$dir/disk.txt'"
   if (( ${#SETTINGS[@]} )); then
      settings_ini "${SETTINGS[@]}" | ssh_run "cat > '$dir/data/Settings.ini'"
   fi
   temps | ssh_run "cat > '$dir/temps-before.txt'"
   local quoted=""
   for a in "$@"; do quoted+=" $(printf '%q' "$a")"; done
   # its own session (setsid), so the run's whole process tree is one group to stop; nothing waits for ssh. Its
   # data in the run's folder, tables' own settings too (run.sh's VPI_TABLE_SETTINGS, for runs without it)
   ssh_run "cd '$GAME' && (env $SESSION_ENV VPI_DATA='$dir/data' VPI_TABLE_SETTINGS='$dir/data/table-settings' setsid nohup$quoted > '$dir/launch.log' 2>&1 < /dev/null & echo \$! > '$dir/pid')"
}

finish_run() {
   local name=$1
   temps | ssh_run "cat > '$REMOTE/runs/$name/temps-after.txt'"
   collect "$name"
}

case ${1:-} in
   idle)
      # fails when busy, so that what's chained after it doesn't run
      check_idle
      echo "the Frame is free"
      ;;
   install)
      "$(dirname "$0")/package.sh"
      "$(dirname "$0")/install.sh"
      ;;
   stage)
      # into the test folder alone: what the owner plays is left as it is, so this runs whenever
      "$(dirname "$0")/package.sh"
      rsync -az --delete -e "ssh -o ConnectTimeout=15 -o BatchMode=yes" "$WORK/package/VisualPinballInfinite/" "$FRAME:$STAGE/"
      ssh_run "cat '$STAGE/VERSION'"
      ;;
   vr)
      name=$2 seconds=$3
      shift 3
      table=("$TABLE_DEFAULT")
      if [[ ${1:-} == *.vpx ]]; then table=("$1"); shift; fi
      if [[ ${1:-} == library ]]; then table=(); shift; fi
      envs=()
      while [[ ${1:-} == --set || ${1:-} == --env ]]; do
         if [[ $1 == --set ]]; then SETTINGS+=("$2"); else envs+=("$2"); fi
         shift 2
      done
      check_idle
      dir=$REMOTE/runs/$name
      start_run "$name" env "${envs[@]}" "$RUNTIME/_v2-entry-point" --verb=run -- ./run.sh "${table[@]}" \
         --vr-pin-eyes --vr-timings "$dir/frames.csv" --vr-capture "$dir/eyes" --vr-capture-at 5000,15000 \
         --quit-after "$((seconds * 1000))" "$@"
      wait_run "$name" $((seconds + 90))
      finish_run "$name"
      grep -E "^VR|saved|error|vpi-player:" "$WORK/runs/frame/$name/data/vpi-player.log" | tail -25 || true
      ;;
   headless)
      name=$2
      shift 2
      table=$TABLE_DEFAULT
      if [[ ${1:-} == *.vpx ]]; then table=$1; shift; fi
      check_idle_or_allowed
      dir=$REMOTE/runs/$name
      start_run "$name" env SDL_VIDEO_DRIVER=offscreen SDL_AUDIO_DRIVER=dummy LD_LIBRARY_PATH="$GAME" \
         "$RUNTIME/_v2-entry-point" --verb=run -- ./vpi-player "$table" --shot "$dir/shot.png" "$@"
      wait_run "$name" 240
      finish_run "$name"
      tail -5 "$WORK/runs/frame/$name/launch.log" || true
      ;;
   panel)
      name=$2
      shift 2
      at=8000
      if [[ ${1:-} =~ ^[0-9]+(,[0-9]+)*$ ]]; then at=$1; shift; fi
      last=$(tr , '\n' <<< "$at" | sort -n | tail -1)
      envs=()
      while [[ ${1:-} == --env ]]; do envs+=("$2"); shift 2; done
      check_idle_or_allowed
      dir=$REMOTE/runs/$name
      start_run "$name" env "${envs[@]}" "$RUNTIME/_v2-entry-point" --verb=run -- ./panel/vpi-panel --shot "$dir/shot.png" --at "$at" \
         --library "$LIBRARY" --data "$dir/data/panel" "$@"
      wait_run "$name" $((last / 1000 + 60))
      finish_run "$name"
      grep -vE "dbus|UPower|login1" "$WORK/runs/frame/$name/launch.log" | tail -20 || true
      ;;
   steam)
      name=$2 seconds=$3
      check_idle
      dir=$REMOTE/runs/$name
      ssh_run "rm -rf '$dir' && mkdir -p '$dir'"
      # the logs already in the game's data folder (the owner's own plays) kept aside first: run.sh keeps one before
      # its own, so two starts would lose them
      ssh_run "mkdir -p '$dir/before' && cp -p ~/.local/share/VisualPinballInfinite/vpi-player*.log '$dir/before/' 2>/dev/null; true"
      # the shortcut's id in Steam (shortcuts.vdf, by its devkit game id), then steam://rungameid/<id << 32 | 0x02000000>
      ssh_run "python3 - <<'PY' > '$dir/gameid'
import glob, os, struct
def vdf(d, p=0):
    out = {}
    while True:
        t = d[p]; p += 1
        if t == 8: return out, p
        e = d.index(b'\\0', p); k = d[p:e].decode('utf-8', 'replace'); p = e + 1
        if t == 0: out[k], p = vdf(d, p)
        elif t == 1: e = d.index(b'\\0', p); out[k] = d[p:e].decode('utf-8', 'replace'); p = e + 1
        elif t == 2: out[k] = struct.unpack_from('<I', d, p)[0]; p += 4
for f in glob.glob(os.path.expanduser('~/.local/share/Steam/userdata/*/config/shortcuts.vdf')):
    s, _ = vdf(open(f, 'rb').read())
    for e in next((v for k, v in s.items() if k.lower() == 'shortcuts'), {}).values():
        if e.get('DevkitGameID') == 'VisualPinballInfinite':
            print((e['appid'] << 32) | 0x02000000)
PY"
      gameid=$(ssh_run "head -1 '$dir/gameid'")
      [[ -n $gameid ]] || { echo "the game isn't in Steam's shortcuts (install it first)"; exit 1; }
      temps | ssh_run "cat > '$dir/temps-before.txt'"
      echo "starting steam://rungameid/$gameid through the Steam client"
      ssh_run "echo 'steam steam://rungameid/$gameid' > ~/.steam/steam.pipe"
      sleep "$seconds"
      ssh_run "ps -eo pid,pgid,etime,args | grep -E '[v]pi-player|[S]teamLaunch' > '$dir/processes.txt' || true; cp ~/.local/share/VisualPinballInfinite/vpi-player.log '$dir/' 2>/dev/null || true"
      # ours alone: the player this launch started (its log is the data folder's)
      ssh_run "pkill -TERM -x vpi-player || true"
      sleep 5
      finish_run "$name"
      cat "$WORK/runs/frame/$name/processes.txt" || true
      ;;
   stop)
      stop_run "$2"
      ;;
   *)
      sed -n '2,32p' "$0" | sed 's/^# \{0,1\}//'
      exit 2
      ;;
esac
