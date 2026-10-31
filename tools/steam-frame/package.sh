#!/usr/bin/env bash
# Gathers what goes on the Steam Frame into $WORK/package/VisualPinballInfinite: the player (stripped), the
# headless runner (which the headset's table settings ask a table's options of), the
# libraries it loads beside itself (libwinevbs, the OpenXR loader), the PinMAME host and libpinmame, VPX 5436's
# shared scripts (core.vbs, …), the start command (run.sh) and the licences; and, once built (panel.sh), the
# headset's panel in panel/: vpi-panel, CEF's files (cef/build.sh distrib) stripped and the app's screens. Build
# the player, the runner and the host (cargo build --release -p player -p headless -p pinmame-host), libwinevbs
# (tools/libwinevbs/setup.sh), libpinmame (tools/pinmame/setup.sh) and the loader (tools/openxr/setup.sh) first.
# install.sh sends it to the Frame.
set -euo pipefail
source "$(dirname "$0")/env.sh"
OUT=$WORK/package/VisualPinballInfinite
rm -rf "$OUT"
mkdir -p "$OUT/licences"
in_image bash -c "
   set -e
   cp target/release/vpi-player '$OUT/vpi-player'
   cp target/release/vpi-headless '$OUT/vpi-headless'
   strip '$OUT/vpi-player' '$OUT/vpi-headless'
   cp -L vpx-build/linux-aarch64/libwinevbs/build/libwinevbs.so.0 '$OUT/libwinevbs.so.0'
   cp -L vpx-build/linux-aarch64/openxr/libopenxr_loader.so.1 '$OUT/libopenxr_loader.so.1'
   strip '$OUT/libwinevbs.so.0' '$OUT/libopenxr_loader.so.1'
   cp target/release/vpi-pinmame '$OUT/vpi-pinmame'
   cp -L vpx-build/linux-aarch64/pinmame-5436/build/libpinmame.so '$OUT/libpinmame.so'
   strip '$OUT/vpi-pinmame' '$OUT/libpinmame.so'
"
# the headset's panel: CEF's files as the cef crate lays them out, but what only builds use (and the setuid
# sandbox helper, which the panel doesn't use), its libraries stripped of what only debugging reads; and beside
# libcef.so, AT-SPI's toolkit bridge, which it links and Steam's runtime lacks (panel.sh atspi)
CEF=$WORK/cef/dist/cef_linux_aarch64
ATSPI=$WORK/cef/atspi
if [[ -x $REPO/target/release/vpi-panel && -f $REPO/app/ui/dist/index.html && -f $CEF/libcef.so && -f $ATSPI/libatspi.so.0 ]]; then
   mkdir -p "$OUT/panel"
   rsync -a --exclude=include --exclude=libcef_dll --exclude=cmake --exclude=CMakeLists.txt --exclude=archive.json \
      --exclude=chrome-sandbox --exclude=CREDITS.html "$CEF/" "$OUT/panel/"
   cp "$REPO/target/release/vpi-panel" "$OUT/panel/vpi-panel"
   cp -R "$REPO/app/ui/dist" "$OUT/panel/ui"
   cp "$ATSPI/libatk-bridge-2.0.so.0" "$ATSPI/libatspi.so.0" "$OUT/panel/"
   in_image bash -c "set -e; strip '$OUT/panel/vpi-panel'; strip --strip-unneeded '$OUT/panel/libcef.so' '$OUT/panel/libvk_swiftshader.so' '$OUT/panel/libvulkan.so.1'"
   cp "$CEF/CREDITS.html" "$OUT/licences/Chromium and CEF's parts (CREDITS).html"
   cp "$(ls -d "$WORK"/cef/dist/cef_binary_*_minimal | head -1)/LICENSE.txt" "$OUT/licences/CEF (BSD-3-Clause).txt"
   cp "$ATSPI/COPYING" "$OUT/licences/at-spi2-core (LGPL-2.1-or-later).txt"
else
   echo "the headset's panel isn't built (tools/steam-frame/panel.sh ui atspi build): left out"
fi
cp "$REPO/tools/steam-frame/run.sh" "$OUT/run.sh"
chmod +x "$OUT/run.sh" "$OUT/vpi-player" "$OUT/vpi-headless" "$OUT/vpi-pinmame"
cp -R "$REPO/vpx-build/vpinball-5436/scripts" "$OUT/scripts"
cp "$REPO/LICENSE" "$OUT/licences/Visual Pinball Infinite (GPL-3.0-or-later).txt"
# libwinevbs carries no licence file: its README names Wine's LGPL-2.1 and ReactOS's GPL-2.0
cp "$REPO/vpx-build/linux-aarch64/libwinevbs/README.md" "$OUT/licences/libwinevbs (README, its licences).md"
cp "$REPO/vpx-build/linux-aarch64/openxr/OpenXR-SDK/LICENSE" "$OUT/licences/OpenXR loader (Apache-2.0).txt"
# PinMAME: BSD-3-Clause and the older MAME licence (its LICENSE); the host is BSD-3-Clause, not GPL, as it links it
cp "$REPO/vpx-build/linux-aarch64/pinmame-5436/LICENSE" "$OUT/licences/PinMAME (BSD-3-Clause, MAME).txt"
echo "$(git -C "$REPO" describe --always --dirty --abbrev=7) $(date -u +%Y-%m-%dT%H:%MZ)" > "$OUT/VERSION"
du -sh "$OUT"
