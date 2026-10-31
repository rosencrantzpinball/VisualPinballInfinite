#!/usr/bin/env bash
# Builds CEF for the Steam Frame, natively on arm64 in the CEF build image (image.sh), from the sources fetch.sh
# fetched and with the toolchain build-toolchain.sh built:
#  - configure: our patches to CEF's tools (patches/cef-*.patch: a native arm64 Linux build without a sysroot);
#    what the build runs put where Chromium looks for it, in place of the tarball's prebuilt x86-64 copies (below);
#    then CEF's own hook (its translated files, its patches to Chromium, GN's arguments and `gn gen` into
#    out/Release_GN_arm64), then our patches to Chromium (patches/chromium-*.patch: fixes for what Google's own
#    builds never compile, such as V4L2 without VA-API on desktop Linux).
#  - build: ninja, for the targets given (by default cefsimple and chrome_sandbox: libcef, its resources, CEF's
#    simplest client and the setuid sandbox helper CEF's distributions carry), carrying on until KEEP_GOING steps have
#    failed (ninja's -k: 1 by default, 0 for no limit).
#  - distrib: CEF's own packaging, its minimal binary distribution, in dist/ as a directory and a .tar.bz2; and that
#    laid out for the cef crate (dist/cef_linux_aarch64, below).
# GN's arguments, beyond CEF's own: a Release build that isn't Google's official one (no ThinLTO or PGO), without
# symbols; against the image's own libraries (the Steam Runtime's) rather than Chromium's Debian sysroot; with our
# clang and gn, and Chromium's own Rust toolchain as built here; with H.264 and AAC (FFmpeg's, the "Chrome"
# branding), HEVC's parser and Linux's V4L2 hardware decoders (the Frame's video block) and without VA-API (no driver
# for it there); the system allocator left alone (PartitionAlloc not taking over malloc, as in CEF's own builds: the
# player is Rust); DevTools' front end type-checked and left unbundled, links made by our lld rather than mold: the
# esbuild that DevTools otherwise runs and mold come only prebuilt, for x86-64 (TypeScript's tsgo is built here); and
# Chromium's real credits (about:credits, and CEF's CREDITS.html: the licences of all it contains), which only
# official builds generate by default.
#
#   tools/steam-frame/cef/build.sh [configure|build|distrib|all] [ninja targets...]
set -euo pipefail
source "$(dirname "$0")/env.sh"
PREFIX=$CEF_WORK/toolchain/llvm
RUST_PREFIX=$CEF_WORK/toolchain/rust
OUT=out/Release_GN_arm64
DIST=$CEF_WORK/dist
PATCHES=$REPO/tools/steam-frame/cef/patches
JOBS=${JOBS:-12}
step=${1:-all}
shift || true
targets=("$@")
[[ ${#targets[@]} -gt 0 ]] || targets=(cefsimple chrome_sandbox)
for tool in "$PREFIX/bin/clang" "$PREFIX/bin/gn" "$PREFIX/bin/bindgen" "$CEF_WORK/toolchain/node/bin/node" \
            "$RUST_PREFIX/bin/rustc" "$RUST_PREFIX/bin/rustfmt" "$RUST_PREFIX/bin/cc_bindings_from_rs" \
            "$CEF_WORK/toolchain/tsgo/tsc" "$CEF_WORK/toolchain/rollup/rollup.linux-arm64-gnu.node" \
            "$CEF_WORK/toolchain/$GO/bin/go"; do
   [[ -x $tool ]] || { echo "no $tool (build-toolchain.sh)" >&2; exit 1; }
done
[[ -d $CHROMIUM_SRC/cef/.git ]] || { echo "no sources (fetch.sh)" >&2; exit 1; }
echo "system disk: $(df -h / | awk 'NR==2 {print $4}') free; data disk: $(df -h "$WORK" | awk 'NR==2 {print $4}') free"

# runs in the image with our clang and gn first on the PATH; Go (Dawn's generators) may fetch modules' source, never
# a toolchain
in_build() {
   in_cef_image env PATH="$PREFIX/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin" GOTOOLCHAIN=local \
      bash -c "set -euo pipefail; $1"
}

if [[ $step == configure || $step == all ]]; then
   # our patches to CEF's tools, each once
   for p in "$PATCHES"/cef-*.patch; do
      [[ -f $p ]] || continue
      if git -C "$CHROMIUM_SRC/cef" apply --reverse --check "$p" 2>/dev/null; then
         echo "$(basename "$p"): already applied"
      else
         git -C "$CHROMIUM_SRC/cef" apply "$p"
         echo "$(basename "$p"): applied"
      fi
   done
   # Chromium's build runs its scripts' Python, Node.js and Blink's gperf, crubit its clang-format, Dawn its Go and
   # CEF's API hash its clang, from where the tarball's prebuilt ones are (x86-64, but for Dawn's arm64 Go), with no
   # setting to say otherwise: ours (Node.js, Go, clang and clang-format built from source, Debian's Python and gperf)
   # go there instead
   for pair in "third_party/cpython3/host/bin/python3=/usr/bin/python3" \
               "third_party/node/linux/node-linux-x64/bin/node=$CEF_WORK/toolchain/node/bin/node" \
               "third_party/gperf/cipd/bin/gperf=/usr/bin/gperf" \
               "buildtools/linux64-format/clang-format=$PREFIX/bin/clang-format" \
               "third_party/dawn/tools/golang/linux-arm64=$CEF_WORK/toolchain/$GO" \
               "third_party/llvm-build/Release+Asserts/bin/clang=$PREFIX/bin/clang"; do
      at=$CHROMIUM_SRC/${pair%%=*}
      if [[ ! -L $at ]]; then
         [[ -e $at ]] && mv "$at" "$at.x86-64-prebuilt"
         ln -s "${pair#*=}" "$at"
      fi
   done
   # DevTools' TypeScript compiler (tsgo, built from source), copied there rather than linked: it finds its library
   # files beside itself. Every copy here keeps its file's time (-p): a new one would have ninja redo each step that
   # uses the file, thousands of them for tsc.
   tsc=$CHROMIUM_SRC/third_party/typescript/linux-amd64/src/lib/tsc
   [[ -e $tsc.x86-64-prebuilt ]] || mv "$tsc" "$tsc.x86-64-prebuilt"
   install -p -m 755 "$CEF_WORK/toolchain/tsgo/tsc" "$tsc"
   # and the native core of DevTools' Rollup (built from source; its node_modules carry it for x86-64 alone) beside
   # Rollup, where it looks first
   install -p -m 755 "$CEF_WORK/toolchain/rollup/rollup.linux-arm64-gnu.node" \
      "$CHROMIUM_SRC/third_party/devtools-frontend/src/node_modules/rollup/dist/rollup.linux-arm64-gnu.node"
   # and Chromium's own Rust toolchain, where Chromium looks for it: ours (rustc, rustfmt and crubit's
   # cc_bindings_from_rs, built from source), with the bindgen and libclang built here, and the parts of the
   # tarball's copy that are source (crubit's support files, the Rust library's source, which Chromium compiles
   # itself) and its VERSION, which names the same Rust and LLVM revisions
   toolchain=$CHROMIUM_SRC/third_party/rust-toolchain
   if [[ ! -L $toolchain ]]; then
      mv "$toolchain" "$toolchain.x86-64-prebuilt"
      ln -s "$RUST_PREFIX" "$toolchain"
   fi
   for part in lib/third_party/crubit lib/rustlib/src; do
      mkdir -p "$RUST_PREFIX/$part"
      rsync -a --delete "$toolchain.x86-64-prebuilt/$part/" "$RUST_PREFIX/$part/"
   done
   cp -p "$toolchain.x86-64-prebuilt/VERSION" "$RUST_PREFIX/VERSION"
   install -p -m 755 "$PREFIX/bin/bindgen" "$RUST_PREFIX/bin/bindgen"
   cp -pP "$PREFIX"/lib/libclang.so* "$RUST_PREFIX/lib/"
   # GN_DEFINES as CEF's hook reads it (split as a shell would: a quoted value stays one, and "24" a string)
   defines=$CEF_WORK/build/gn_defines.txt
   mkdir -p "$CEF_WORK/build"
   cat > "$defines" <<EOF
is_official_build=false symbol_level=0 blink_symbol_level=0 v8_symbol_level=0 use_sysroot=false
'clang_base_path=$PREFIX' 'clang_version="$CLANG_VERSION"'
proprietary_codecs=true ffmpeg_branding=Chrome use_vaapi=false use_v4l2_codec=true
use_partition_alloc_as_malloc=false enable_backup_ref_ptr_support=false
treat_warnings_as_errors=false chrome_pgo_phase=0 use_siso=false
devtools_skip_typecheck=false devtools_bundle=false use_mold=false
generate_about_credits=true
EOF
   in_build "
      cd '$CHROMIUM_SRC/cef'
      export GN_DEFINES=\"\$(tr -s ' \n' ' ' < '$defines')\" GN_OUT_CONFIGS=Release_GN_arm64
      # no depot_tools: CEF's hook takes the one function it wants of it from our stand-in
      export PYTHONPATH='$REPO/tools/steam-frame/cef/shim'
      echo \"GN_DEFINES=\$GN_DEFINES\"
      python3 tools/gclient_hook.py
   "
   # our patches to Chromium, each once, after CEF's
   for p in "$PATCHES"/chromium-*.patch; do
      [[ -f $p ]] || continue
      if (cd "$CHROMIUM_SRC" && patch -p1 --reverse --dry-run --force < "$p" > /dev/null 2>&1); then
         echo "$(basename "$p"): already applied"
      else
         (cd "$CHROMIUM_SRC" && patch -p1 --forward < "$p")
         echo "$(basename "$p"): applied"
      fi
   done
fi

if [[ $step == build || $step == all ]]; then
   in_build "ninja -C '$CHROMIUM_SRC/$OUT' -j $JOBS -k ${KEEP_GOING:-1} ${targets[*]}"
fi

if [[ $step == distrib || $step == all ]]; then
   name=cef_binary_${CEF_VERSION}_linuxarm64_minimal
   crate=$DIST/cef_linux_aarch64
   rm -rf "${DIST:?}/$name" "$DIST/$name.tar.bz2" "$crate"
   # The cef crate (cef-dll-sys) takes CEF from CEF_PATH laid out as it lays out the distributions it downloads:
   # Release/ and Resources/ in one directory with the headers, the wrapper, CMake's files and the credits, and
   # archive.json naming the archive, whose version it checks. Should that check fail, it downloads CEF instead: build
   # with CEF_DOWNLOAD_URL set to somewhere that doesn't answer, so it fails instead.
   in_build "
      cd '$CHROMIUM_SRC/cef/tools'
      CEF_ARCHIVE_FORMAT=tar.bz2 python3 make_distrib.py --output-dir='$DIST' --ninja-build --arm64-build --minimal \
         --no-docs
      mkdir '$crate'
      cp -a '$DIST/$name/Release/.' '$DIST/$name/Resources/.' '$crate/'
      cp -a '$DIST/$name'/{CMakeLists.txt,cmake,include,libcef_dll,CREDITS.html} '$crate/'
      printf '{\n  \"type\": \"minimal\",\n  \"name\": \"%s\",\n  \"sha1\": \"%s\"\n}' '$name.tar.bz2' \
         \"\$(sha1sum '$DIST/$name.tar.bz2' | cut -d' ' -f1)\" > '$crate/archive.json'
   "
   echo "CEF_PATH=$crate"
fi
echo "system disk: $(df -h / | awk 'NR==2 {print $4}') free; data disk: $(df -h "$WORK" | awk 'NR==2 {print $4}') free"
