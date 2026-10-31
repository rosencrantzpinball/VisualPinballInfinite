#!/usr/bin/env bash
# The headset's panel (vpi-panel, PLAN decision 20), built and checked in the CEF build image:
#  - ui: the app's screens (app/ui) built into app/ui/dist with npm and Vite, the owner's allowance for app/ui
#    alone (npm ci keeps to package-lock.json's hashes); npm is the one in the Node.js source built for CEF
#    (cef/build-toolchain.sh), its cache on the data volume.
#  - atspi: AT-SPI's toolkit bridge, which libcef.so links and Steam's runtime lacks (it has ATK alone):
#    libatk-bridge-2.0 and libatspi, built from at-spi2-core's source, the release the SDK has and so the one
#    CEF was linked against, GNOME's tarball checked against its published sha256. Into $CEF_WORK/atspi, for
#    package.sh to put beside libcef.so (its RUNPATH is $ORIGIN); built once.
#  - build: vpi-panel built against the CEF built here (CEF_PATH, cef/build.sh distrib), with
#    CEF_DOWNLOAD_URL pointing nowhere: should CEF_PATH ever fail the cef crate's check, the build fails
#    instead of fetching a prebuilt CEF. CEF's files land beside it in target/release.
#  - shot <picture.png> [library folder] [ms]: the panel drawn alone (Chromium's headless platform,
#    SwiftShader), its picture saved after the time given (5000 ms), from the built screens and the
#    library's tables (an empty one by default).
#
#   tools/steam-frame/panel.sh [ui] [atspi] [build] [shot <picture.png> [library] [ms]]
set -euo pipefail
source "$(dirname "$0")/cef/env.sh"
NODE=$CEF_WORK/toolchain/node/bin/node
NPM=$CEF_WORK/build/node-v24.12.0/deps/npm/bin/npm-cli.js
CRATE=$CEF_WORK/dist/cef_linux_aarch64
ATSPI=at-spi2-core-2.56.2
ATSPI_URL=https://download.gnome.org/sources/at-spi2-core/2.56/$ATSPI.tar.xz

steps=("$@")
[[ ${#steps[@]} -gt 0 ]] || steps=(ui atspi build)
while [[ ${#steps[@]} -gt 0 ]]; do
   step=${steps[0]}
   steps=("${steps[@]:1}")
   case $step in
      ui)
         in_cef_image bash -c "set -euo pipefail
            cd '$REPO/app/ui'
            export npm_config_cache='$CEF_WORK/npm-cache' npm_config_update_notifier=false
            '$NODE' '$NPM' ci --no-audit --no-fund
            '$NODE' node_modules/vite/bin/vite.js build"
         ;;
      atspi)
         [[ -f $CEF_WORK/atspi/libatk-bridge-2.0.so.0 && -f $CEF_WORK/atspi/libatspi.so.0 ]] && continue
         sums=$CEF_WORK/dl/$ATSPI.sha256sum
         [[ -f $sums ]] || curl -fsS -o "$sums" "${ATSPI_URL%.tar.xz}.sha256sum"
         want=$(awk -v f="$ATSPI.tar.xz" '$2 == f {print $1}' "$sums")
         if [[ ! -f $CEF_WORK/dl/$ATSPI.tar.xz ]] || [[ $(shasum -a 256 "$CEF_WORK/dl/$ATSPI.tar.xz" | cut -d' ' -f1) != "$want" ]]; then
            curl -fsSL -o "$CEF_WORK/dl/$ATSPI.tar.xz" "$ATSPI_URL"
            [[ $(shasum -a 256 "$CEF_WORK/dl/$ATSPI.tar.xz" | cut -d' ' -f1) == "$want" ]] || { echo "$ATSPI.tar.xz: sha256 differs from GNOME's" >&2; exit 1; }
         fi
         # as Debian builds it, without what only the desktop's own services, GTK 2 or documentation need
         in_cef_image bash -c "set -euo pipefail
            rm -rf '$CEF_WORK/build/$ATSPI' '$CEF_WORK/atspi'
            tar -xJf '$CEF_WORK/dl/$ATSPI.tar.xz' -C '$CEF_WORK/build'
            cd '$CEF_WORK/build/$ATSPI'
            meson setup _build --prefix=/usr --libdir=lib --buildtype=release -Dintrospection=disabled -Ddocs=false \
               -Dgtk2_atk_adaptor=false -Duse_systemd=false -Dx11=enabled
            meson compile -C _build
            DESTDIR='$CEF_WORK/atspi/root' meson install -C _build
            cp -L '$CEF_WORK/atspi/root/usr/lib/libatk-bridge-2.0.so.0' '$CEF_WORK/atspi/root/usr/lib/libatspi.so.0' COPYING '$CEF_WORK/atspi/'
            strip --strip-unneeded '$CEF_WORK/atspi/libatk-bridge-2.0.so.0' '$CEF_WORK/atspi/libatspi.so.0'"
         ;;
      build)
         [[ -f $CRATE/archive.json ]] || { echo "no CEF for the cef crate in $CRATE (cef/build.sh distrib)" >&2; exit 1; }
         in_cef_image bash -c "set -euo pipefail
            cd '$REPO'
            CEF_PATH='$CRATE' CEF_DOWNLOAD_URL=http://127.0.0.1:9 cargo build --release -p panel-host --features cef"
         ;;
      shot)
         picture=${steps[0]:?shot needs a picture to save}
         library=$CEF_WORK/tmp/panel-library
         at=5000
         steps=("${steps[@]:1}")
         if [[ ${#steps[@]} -gt 0 && ! ${steps[0]} =~ ^[0-9]+$ && ${steps[0]} != ui && ${steps[0]} != build && ${steps[0]} != shot ]]; then
            library=${steps[0]}
            steps=("${steps[@]:1}")
         fi
         if [[ ${#steps[@]} -gt 0 && ${steps[0]} =~ ^[0-9]+$ ]]; then
            at=${steps[0]}
            steps=("${steps[@]:1}")
         fi
         mkdir -p "$library/Tables" "$(dirname "$picture")"
         in_cef_image bash -c "set -euo pipefail
            cd '$REPO'
            timeout 90 target/release/vpi-panel --shot '$picture' --at $at --ui app/ui/dist --library '$library' \
               --data '$CEF_WORK/tmp/panel-data' --size 1600x1000 --scale 125 2>&1 \
               | grep -vE 'dbus|UPower|login1|cursor_factory' || true"
         [[ -f $picture ]] || { echo "no picture" >&2; exit 1; }
         ;;
      *)
         sed -n '2,17p' "$0" | sed 's/^# \{0,1\}//'
         exit 2
         ;;
   esac
done
