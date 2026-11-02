#!/usr/bin/env bash
# Builds the browser's player and lays out one table as a site to put anywhere static files are served:
#
#   tools/web/package.sh <table folder or .vpx> <out folder> [--title "Name"] [--byline "by ..."] [--with-backglass]
#                        [--no-source] [--slim] [--stream-sounds seconds]
#
# The out folder gets the page (index.html, vpi.js, gpu.js), the program (vpi.wasm), manifest.json, VPX's shared
# scripts (scripts/, core.vbs and the rest, as the Frame's package carries them), and table/: the table's folder as it
# is (the score display's fonts beside it, its physics file, its own settings file), but for its backglass files
# (.directb2s: the browser shows the playfield alone) unless --with-backglass. A file over 24 MiB goes in pieces of
# 24 MiB (name.000, name.001, ...) that the page joins: static hosts cap a file's size (Cloudflare Pages at 25 MiB,
# GitHub at 100 MB), and a file that gzip shrinks by a twentieth or more is gzipped (the page unzips it). source/ holds the program's source as built (the repository's tracked files, uncommitted
# changes included), which a site serving GPL software has to offer, unless --no-source; .nojekyll keeps GitHub
# Pages from running its site builder over it all. --slim puts a smaller copy of the table in place of the table itself
# (tools/web/slim: its pictures WebP, lossy where that still matches by SSIM, its uncompressed sounds Ogg Vorbis);
# --stream-sounds has the page keep sounds longer than that many seconds as their files and decode them as they play
# (0, the default: all decoded as the table loads). The program is built in the image (tools/steam-frame/in-image.sh),
# after tools/web/wasi-libc.sh and tools/web/libwinevbs.sh; the table and the out folder are under the work folder.
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
source "$ROOT/tools/steam-frame/env.sh"
SRC=$1
OUT=$2
shift 2
TITLE= BYLINE= BACKGLASS=0 SOURCE=1 SLIM=0 STREAM=0
while [[ $# -gt 0 ]]; do
   case $1 in
      --title) TITLE=$2; shift 2 ;;
      --byline) BYLINE=$2; shift 2 ;;
      --with-backglass) BACKGLASS=1; shift ;;
      --no-source) SOURCE=0; shift ;;
      --slim) SLIM=1; shift ;;
      --stream-sounds) STREAM=$2; shift 2 ;;
      *) echo "unknown option $1" >&2; exit 2 ;;
   esac
done
if [[ -f $SRC ]]; then TABLE_DIR=$(cd "$(dirname "$SRC")" && pwd); VPX=$(basename "$SRC"); else TABLE_DIR=$(cd "$SRC" && pwd); VPX=$(cd "$TABLE_DIR" && ls *.vpx | head -1); fi
[[ -n $VPX && -f $TABLE_DIR/$VPX ]] || { echo "no .vpx in $SRC" >&2; exit 1; }

# a panic names its source file: by the image's own names for the folders (/work, /cargo, /vpi), not this machine's
REMAP="--remap-path-prefix=$WORK=/work --remap-path-prefix=$WORK/cargo=/cargo --remap-path-prefix=$ROOT=/vpi"
in_image env CARGO_TARGET_WASM32_WASIP1_RUSTFLAGS="$REMAP" cargo build --profile web --target wasm32-wasip1 -p vpi-web
mkdir -p "$OUT/table"
cp "$ROOT/engine/web/site/index.html" "$ROOT/engine/web/site/vpi.js" "$ROOT/engine/web/site/gpu.js" "$OUT/"
cp "$ROOT/target/wasm32-wasip1/web/vpi_web.wasm" "$OUT/vpi.wasm"
touch "$OUT/.nojekyll"
EXCLUDE=(--exclude '.DS_Store')
[[ $BACKGLASS == 1 ]] || EXCLUDE+=(--exclude '*.directb2s')
rsync -a --delete --prune-empty-dirs "${EXCLUDE[@]}" "$TABLE_DIR/" "$OUT/table/"
rsync -a --delete "$ROOT/vpx-build/vpinball-5436/scripts/" "$OUT/scripts/"
if [[ $SLIM == 1 ]]; then
   in_image cargo build --release --manifest-path tools/web/slim/Cargo.toml --target-dir target/slim
   in_image target/slim/release/vpx-slim "$TABLE_DIR/$VPX" "$OUT/table/$VPX"
fi

BUILD=$(git -C "$ROOT" rev-parse --short=12 HEAD)
SOURCE_FILE=
rm -rf "$OUT/source"
if [[ $SOURCE == 1 ]]; then
   # the tracked files as they are, uncommitted changes in them too (stash create leaves the tree and index alone)
   TREE=$(git -C "$ROOT" stash create)
   [[ -z $TREE ]] || BUILD=$BUILD-modified
   mkdir -p "$OUT/source"
   SOURCE_FILE=source/vpi-$BUILD.tar.gz
   git -C "$ROOT" archive --format=tar.gz --prefix="vpi-$BUILD/" -o "$OUT/$SOURCE_FILE" "${TREE:-HEAD}"
   cat > "$OUT/source/README.txt" <<EOF
The program this site plays tables with (vpi.wasm) is Visual Pinball Infinite's player, GPL-3.0-or-later, built from
vpi-$BUILD.tar.gz here: tools/web/README.md in it says how.

Linked into it, each under its own licence:
- libwinevbs, Wine's VBScript engine (LGPL-2.1-or-later): https://github.com/vpinball/libwinevbs at the commit
  tools/web/libwinevbs.sh names, with the patches in tools/libwinevbs/patches and tools/web/patches;
- wasi-libc (its own licences, MIT and Apache-2.0 among them): https://github.com/WebAssembly/wasi-libc at the release
  tools/web/wasi-libc.sh names;
- the Rust crates Cargo.lock lists.

The table in table/ is its author's, under the author's terms.
EOF
fi

python3 -I - "$OUT" "$VPX" "$TITLE" "$BYLINE" "$BUILD" "$SOURCE_FILE" "$STREAM" <<'PY'
import gzip, hashlib, json, os, re, sys, urllib.parse
out, vpx, title, byline, build, source, stream = sys.argv[1:8]
PIECE = 24 << 20
digest = lambda data: hashlib.sha256(data).hexdigest()[:16]
# each file's (or piece's) served bytes' hash names its version in the page's cache (`?v=`), and its contents'
# hash (`content`) is what the page checks it fetched: a cache may hand an older copy just after the site changes
# a file gzipped where that saves a twentieth or more (the page unzips it; hosts don't all compress), its bytes as served
def served(path, data):
    z = gzip.compress(data, compresslevel=9, mtime=0)
    if len(z) < len(data) * 0.95:
        open(path, 'wb').write(z)
        return z, True
    open(path, 'wb').write(data)
    return data, False
files = []
for root, _, names in [w for top in ('table', 'scripts') for w in os.walk(os.path.join(out, top))]:
    for n in sorted(names):
        full = os.path.join(root, n)
        rel = os.path.relpath(full, out).replace(os.sep, '/')
        data = open(full, 'rb').read()
        entry = {'path': '/' + rel, 'size': len(data)}
        if len(data) > PIECE:
            entry['parts'] = []
            for k, at in enumerate(range(0, len(data), PIECE)):
                piece = data[at:at + PIECE]
                bytes_, gz = served(f'{full}.{k:03d}', piece)
                entry['parts'].append({'url': urllib.parse.quote(f'{rel}.{k:03d}'), 'size': len(piece), 'sha256': digest(bytes_), 'content': digest(piece), **({'gzip': True} if gz else {})})
            os.remove(full)
        else:
            bytes_, gz = served(full, data)
            entry.update(url=urllib.parse.quote(rel), sha256=digest(bytes_), content=digest(data), **({'gzip': True} if gz else {}))
        files.append(entry)
program_bytes, program_gz = served(os.path.join(out, 'vpi.wasm'), open(os.path.join(out, 'vpi.wasm'), 'rb').read())
program = {'url': 'vpi.wasm', 'sha256': digest(program_bytes), **({'gzip': True} if program_gz else {})}
stem = os.path.splitext(vpx)[0]
manifest = {
    'title': title or stem,
    'byline': byline,
    'id': re.sub(r'[^a-z0-9]+', '-', stem.lower()).strip('-'),
    'table': '/table/' + vpx,
    'build': build,
    'program': program,
    'source': source,
    'streamSounds': float(stream),
    'files': files,
}
json.dump(manifest, open(os.path.join(out, 'manifest.json'), 'w'), indent=1)
total = sum(os.path.getsize(os.path.join(r, n)) for r, _, ns in os.walk(out) for n in ns if not os.path.relpath(r, out).startswith('source'))
biggest = max(os.path.getsize(os.path.join(r, n)) for r, _, ns in os.walk(out) for n in ns)
print(f"{out}: {len(files)} files, {total / 1e6:.0f} MB to fetch, the biggest file {biggest / 1e6:.1f} MB")
PY
