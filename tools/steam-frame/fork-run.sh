#!/bin/sh
# The owner's VPX fork as its own start command runs it (~/devkit-game/VisualPinball/run.sh), without -Launcher: it
# plays the table given (-Play). Run by versus.py.
cd /home/steamos/devkit-game/VisualPinball || exit 1
export LD_LIBRARY_PATH="$PWD${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
exec ./VPinballX_BGFX "$@"
