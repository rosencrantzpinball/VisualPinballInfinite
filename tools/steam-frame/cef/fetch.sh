#!/usr/bin/env bash
# Fetches what CEF is built from, into $CEF_WORK (env.sh): Chromium's release source tarball, checked against the
# hashes Google publishes beside it and unpacked into src/chromium/src; CEF's source at its commit, into that
# tree's cef/ where CEF's tools expect it; and LLVM's source at the commit Chromium's clang is built from, into
# src/llvm-project. What's already there is left as it is. Sources only: nothing fetched here is run.
set -euo pipefail
source "$(dirname "$0")/env.sh"
mkdir -p "$CEF_WORK/dl" "$CEF_WORK/src" "$CEF_WORK/logs"
TARBALL=chromium-$CHROMIUM_VERSION.tar.xz
URL=https://commondatastorage.googleapis.com/chromium-browser-official/$TARBALL

# a repository's one commit, without its history
fetch_commit() { # <url> <commit> <folder>
   [[ -d $3/.git ]] && [[ $(git -C "$3" rev-parse HEAD) == "$2" ]] && return
   mkdir -p "$3"
   git -C "$3" init -q
   git -C "$3" remote add origin "$1" 2>/dev/null || git -C "$3" remote set-url origin "$1"
   git -C "$3" fetch -q --depth 1 origin "$2"
   git -C "$3" checkout -q FETCH_HEAD
   echo "$3: $(git -C "$3" log --oneline -1)"
}

# Chromium's source
if [[ ! -d $CHROMIUM_SRC ]]; then
   cd "$CEF_WORK/dl"
   [[ -f $TARBALL.hashes ]] || curl -fsS -O "$URL.hashes"
   want=$(awk '$1 == "sha256" {print $2}' "$TARBALL.hashes")
   if [[ ! -f $TARBALL ]] || [[ $(shasum -a 256 "$TARBALL" | cut -d' ' -f1) != "$want" ]]; then
      curl -fsS -C - --retry 5 --retry-delay 10 -o "$TARBALL" "$URL"
      got=$(shasum -a 256 "$TARBALL" | cut -d' ' -f1)
      [[ $got == "$want" ]] || { echo "$TARBALL: sha256 $got, published $want" >&2; exit 1; }
   fi
   echo "unpacking $TARBALL"
   in_cef_image bash -c "
      mkdir -p '$CEF_WORK/src/chromium'
      tar -xJf '$CEF_WORK/dl/$TARBALL' -C '$CEF_WORK/src/chromium'
      mv '$CEF_WORK/src/chromium/chromium-$CHROMIUM_VERSION' '$CHROMIUM_SRC'
   "
   # the tarball's prebuilt programs (x86-64, for Google's builders) made unrunnable: the image runs x86-64
   # programs through Rosetta, and nothing downloaded is to run; what the build needs of them is built from source
   python3 -I "$(dirname "$0")/prebuilts.py" "$CHROMIUM_SRC" --disarm > "$CEF_WORK/logs/prebuilts.txt"
fi
# Chromium's test fonts, data CEF's test support carries along (libcef links it) and the source tarball leaves out:
# fetched as a checkout's DEPS fetches them, from Google's bucket, and checked against the sha256 DEPS names
fonts=$CHROMIUM_SRC/third_party/test_fonts/test_fonts
if [[ ! -f $fonts/Ahem.ttf ]]; then
   block=$(awk "/'src\/third_party\/test_fonts\/test_fonts'/{f=1} f{print} f && /\]/{exit}" "$CHROMIUM_SRC/DEPS")
   object=$(sed -n "s/.*'object_name': '\([0-9a-f]*\)'.*/\1/p" <<< "$block")
   want=$(sed -n "s/.*'sha256sum': '\([0-9a-f]*\)'.*/\1/p" <<< "$block")
   [[ -n $object && -n $want ]] || { echo "DEPS names no test fonts' object" >&2; exit 1; }
   archive=$CEF_WORK/dl/test_fonts-$object.tar.gz
   [[ -f $archive ]] || curl -fsS -o "$archive" "https://storage.googleapis.com/chromium-fonts/$object"
   [[ $(shasum -a 256 "$archive" | cut -d' ' -f1) == "$want" ]] || { echo "$archive: sha256 differs from DEPS's" >&2; exit 1; }
   mkdir -p "$fonts"
   tar -xzf "$archive" -C "$fonts"
   echo "test fonts: $(ls "$fonts" | wc -l | tr -d ' ') files"
fi
# CEF's, with its history: its version number counts the commits on its release branch since it left master
cef=$CHROMIUM_SRC/cef
if [[ ! -d $cef/.git ]]; then
   git clone -q https://github.com/chromiumembedded/cef.git "$cef"
elif [[ $(git -C "$cef" rev-parse --is-shallow-repository) == true ]]; then
   git -C "$cef" fetch -q --unshallow origin '+refs/heads/*:refs/remotes/origin/*'
fi
git -C "$cef" checkout -q "$CEF_COMMIT"
echo "$cef: $(git -C "$cef" log --oneline -1)"
# LLVM's
fetch_commit https://github.com/llvm/llvm-project.git "$LLVM_COMMIT" "$CEF_WORK/src/llvm-project"
echo "data disk: $(df -h "$WORK" | awk 'NR==2 {print $4}') free"
