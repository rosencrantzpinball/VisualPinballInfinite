#!/usr/bin/env bash
# Plays video in the CEF built here, on the Steam Frame: what it decodes there, and with what, the Frame's hardware
# decoder through V4L2 (qcom-iris-decoder) or FFmpeg. codec-test.html beside this file asks Chromium about each codec
# and plays a few seconds of an H.264 clip with AAC sound, an HEVC clip and an AAC one, made here by the Mac's ffmpeg
# (Homebrew's, with x264 and x265). On the Frame, cefsimple draws on the GPU with no window, muted and with no audio
# device, and sees only the GPU and the video decoder, in a /dev of its own (bwrap): Chromium otherwise opens every
# /dev/video* node to look for decoders, the headset's camera pipeline's among them. Mesa's OpenGL there is zink on
# Turnip. V4L2 decoding is a feature that builds without VA-API leave off: AcceleratedVideoDecoder. PLATFORM picks
# where cefsimple's window is:
#  - gamescope (the default): X11, as in the Frame's game session, from a gamescope of its own with no output
#    (--backend headless), its sockets in a /tmp and network of the run's own. ANGLE draws through GLX: through EGL
#    (or Vulkan) it finds no config for the X window, and Chromium, compositing in software, then decodes in software;
#  - headless: Chromium's headless platform on Mesa's surfaceless EGL (ANGLE's "gl" being GLX, through "gl-egl"). It
#    reports what decodes, but can't import decoded frames (no native pixmaps there), so hardware decoding can't play.
# Like frame.sh: only when the owner isn't playing, in ~/vpi-test (the app in cef/, the run in runs/<name>, with its
# HOME there, so Chromium's profile and NSS database stay in it, deleted after), in a process group of its own,
# stopped after LIMIT seconds at most. The run's log comes back to $WORK/runs/frame/<name>.
#
#   tools/steam-frame/cef/frame-codecs.sh <name> [cefsimple options...]
set -euo pipefail
source "$(dirname "$0")/env.sh"
name=${1:?name the run}
shift
REMOTE=/home/steamos/vpi-test
APP=$REMOTE/cef/app
TEST=$REMOTE/cef/test
DIR=$REMOTE/runs/$name
CLIPS=$CEF_WORK/frame-test
CRATE=$CEF_WORK/dist/cef_linux_aarch64
LIMIT=${LIMIT:-120}
ssh_run() { ssh -o ConnectTimeout=15 -o BatchMode=yes "$FRAME" "$@"; }
copy() { rsync -az -e "ssh -o ConnectTimeout=15 -o BatchMode=yes" "$@"; }
temps() {
   ssh_run 'for z in /sys/class/thermal/thermal_zone*; do t=$(cat $z/type); case $t in gpuss-0-thermal|cpuss0-thermal|ddr-thermal) echo "$t $(( $(cat $z/temp) / 1000 ))C";; esac; done; date -Is'
}

# the clips: 6 s of a 1080p test pattern at 30 fps, in H.264 High with AAC sound and in HEVC Main; and AAC alone
mkdir -p "$CLIPS"
ffmpeg=(ffmpeg -hide_banner -loglevel error -y)
pattern=(-f lavfi -i testsrc2=size=1920x1080:rate=30:duration=6)
tone=(-f lavfi -i sine=frequency=440:sample_rate=48000:duration=6)
[[ -f $CLIPS/h264.mp4 ]] || "${ffmpeg[@]}" "${pattern[@]}" "${tone[@]}" -c:v libx264 -profile:v high -level 4.0 \
   -pix_fmt yuv420p -b:v 6M -c:a aac -b:a 128k -movflags +faststart "$CLIPS/h264.mp4"
[[ -f $CLIPS/hevc.mp4 ]] || "${ffmpeg[@]}" "${pattern[@]}" -c:v libx265 -profile:v main -pix_fmt yuv420p -b:v 6M \
   -tag:v hvc1 -x265-params log-level=error -movflags +faststart "$CLIPS/hevc.mp4"
[[ -f $CLIPS/aac.m4a ]] || "${ffmpeg[@]}" "${tone[@]}" -c:a aac -b:a 128k "$CLIPS/aac.m4a"

# CEF's runtime files as the cef crate lays them out, without what only builds use, and cefsimple; the test page
ssh_run "mkdir -p '$APP' '$TEST'"
copy --exclude={include,libcef_dll,cmake,CMakeLists.txt,archive.json,CREDITS.html,chrome-sandbox} "$CRATE/" "$FRAME:$APP/"
copy "$CHROMIUM_SRC/out/Release_GN_arm64/cefsimple" "$FRAME:$APP/"
copy "$REPO/tools/steam-frame/cef/codec-test.html" "$CLIPS/h264.mp4" "$CLIPS/hevc.mp4" "$CLIPS/aac.m4a" "$FRAME:$TEST/"

quoted=""
for a in "$@"; do quoted+=" $(printf '%q' "$a")"; done
case ${PLATFORM:-gamescope} in
   gamescope)
      env="XDG_RUNTIME_DIR='$DIR/xdg'"
      sandbox="--unshare-net --tmpfs /tmp"
      start="gamescope --backend headless -W 1280 -H 720 -w 1280 -h 720 -- ./cefsimple --ozone-platform=x11 \
         --use-angle=gl"
      ;;
   headless)
      env="EGL_PLATFORM=surfaceless"
      sandbox=""
      start="./cefsimple --ozone-platform=headless --use-angle=gl-egl"
      ;;
   *) echo "PLATFORM: gamescope or headless" >&2; exit 2 ;;
esac
"$REPO/tools/steam-frame/frame.sh" idle
ssh_run "rm -rf '$DIR' && mkdir -p '$DIR/home' && mkdir -m 700 '$DIR/xdg'"
temps | ssh_run "cat > '$DIR/temps-before.txt'"
# its own session (setsid): the run's whole process tree is one group to stop
ssh_run "cd '$APP' && (env -i PATH=/usr/bin:/bin HOME='$DIR/home' LANG=C.UTF-8 $env \
   setsid nohup bwrap --dev-bind / / --dev /dev --tmpfs /dev/shm --dev-bind /dev/dri /dev/dri \
      --dev-bind /dev/video22 /dev/video22 $sandbox \
   $start --url=file://$TEST/codec-test.html \
      --enable-features=AcceleratedVideoDecoder --autoplay-policy=no-user-gesture-required --mute-audio \
      --disable-audio-output --enable-logging=stderr --vmodule='*/media/gpu/*=2'$quoted \
   > '$DIR/launch.log' 2>&1 < /dev/null & echo \$! > '$DIR/pid')"
waited=0
until ssh_run "grep -q 'VPI-CODEC done' '$DIR/launch.log' || ! kill -0 \$(cat '$DIR/pid') 2>/dev/null"; do
   if (( waited >= LIMIT )); then
      echo "not done after ${LIMIT}s"
      break
   fi
   sleep 2
   waited=$((waited + 2))
done
# the page closes its window; whatever of the run is left is asked, then made, to end
sleep 2
ssh_run "pgid=\$(cat '$DIR/pid'); kill -TERM -- -\$pgid 2>/dev/null; sleep 3; kill -KILL -- -\$pgid 2>/dev/null; rm -rf '$DIR/home' '$DIR/xdg'; true"
temps | ssh_run "cat > '$DIR/temps-after.txt'"
mkdir -p "$WORK/runs/frame/$name"
copy "$FRAME:$DIR/" "$WORK/runs/frame/$name/"
log=$WORK/runs/frame/$name/launch.log
echo "collected into $WORK/runs/frame/$name"
sed -n 's/.*"\(VPI-CODEC [^"]*\)".*/\1/p' "$log"
