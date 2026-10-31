#!/usr/bin/env bash
# Installs the package (package.sh) on the Steam Frame, over ssh, the way the VPX fork's install.sh does: as a
# devkit game of its own (game id VisualPinballInfinite, beside the fork's VisualPinball), its files in
# ~/devkit-game/VisualPinballInfinite, registered with the Steam client through Valve's devkit scripts (MIT,
# already in ~/devkit-utils from the fork's install) to run run.sh (Steam starts it on SteamOS itself), and named
# "Visual Pinball Infinite (Frame test)" in the library with the fork's own tool on the Frame. Re-running updates
# it in place.
#
#   tools/steam-frame/install.sh [user@host]
set -euo pipefail
source "$(dirname "$0")/env.sh"
HOST=${1:-$FRAME}
GAMEID=VisualPinballInfinite
NAME='Visual Pinball Infinite (Frame test)'
RUNTIME=SteamLinuxRuntime_4-arm64
PACKAGE=$WORK/package/$GAMEID
[[ -x $PACKAGE/vpi-player ]] || { echo "no package in $PACKAGE (package.sh)" >&2; exit 1; }
ssh_run() { ssh -o ConnectTimeout=15 -o BatchMode=yes "$HOST" "$@"; }

# not while the owner plays (the default host's check)
if [[ $HOST == "$FRAME" ]]; then
   "$(dirname "$0")/frame.sh" idle
fi
ssh_run 'test -f ~/devkit-utils/steamos-prepare-upload' || { echo "Valve's devkit scripts aren't in ~/devkit-utils on the Frame" >&2; exit 1; }
DIRECTORY=$(ssh_run "python3 ~/devkit-utils/steamos-prepare-upload --gameid $GAMEID | tail -1 | python3 -c 'import json,sys; print(json.load(sys.stdin)[\"directory\"])'")
[[ $DIRECTORY == */devkit-game/$GAMEID ]] || { echo "unexpected game folder: $DIRECTORY" >&2; exit 1; }
echo "files -> $HOST:$DIRECTORY"
rsync -az --delete --chmod=Du=rwx,Dgo=rx,Fu=rwx,Fog=rx -e "ssh -o ConnectTimeout=15 -o BatchMode=yes" "$PACKAGE/" "$HOST:$DIRECTORY/"
PARMS='{"gameid": "'$GAMEID'", "directory": "'$DIRECTORY'", "argv": ["run.sh"], "env": {}, "settings": {"steam_play": "0", "compat_tool": "'$RUNTIME'"}, "force_appid": "", "lepton_args": ""}'
RESULT=$(ssh_run "python3 ~/devkit-utils/steam-client-create-shortcut --parms '$PARMS'" | tail -1)
case $RESULT in
   *'"error"'*) echo "Steam didn't register the game: $RESULT" >&2; exit 1 ;;
esac
if ssh_run "test -f ~/.local/share/vpx-frame/set-library-name.py && python3 ~/.local/share/vpx-frame/set-library-name.py $GAMEID '$NAME'"; then
   echo "registered as \"$NAME\""
else
   echo "registered as \"Devkit Game: $GAMEID\" (the name couldn't be set)"
fi
ssh_run "cat '$DIRECTORY/VERSION'"
