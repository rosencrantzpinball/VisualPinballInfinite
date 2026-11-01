#!/usr/bin/env bash
# Builds the browser's player and lays out one table as a site to put anywhere static files are served:
#
#   tools/web/package.sh <table folder or .vpx> <out folder> [--title "Name"] [--byline "by ..."] [--with-backglass]
#
# The out folder gets the page (index.html, vpi.js, gpu.js), the program (vpi.wasm), manifest.json, VPX's shared
# scripts (scripts/, core.vbs and the rest, as the Frame's package carries them), and table/: the table's folder as it
# is (the score display's fonts beside it, its physics file, its own settings file), but for its backglass files
# (.directb2s: the browser shows the playfield alone) unless --with-backglass. The program is
# built in the image (tools/steam-frame/in-image.sh), after tools/web/wasi-libc.sh and tools/web/libwinevbs.sh.
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
SRC=$1
OUT=$2
shift 2
TITLE= BYLINE= BACKGLASS=0
while [[ $# -gt 0 ]]; do
   case $1 in
      --title) TITLE=$2; shift 2 ;;
      --byline) BYLINE=$2; shift 2 ;;
      --with-backglass) BACKGLASS=1; shift ;;
      *) echo "unknown option $1" >&2; exit 2 ;;
   esac
done
if [[ -f $SRC ]]; then TABLE_DIR=$(cd "$(dirname "$SRC")" && pwd); VPX=$(basename "$SRC"); else TABLE_DIR=$(cd "$SRC" && pwd); VPX=$(cd "$TABLE_DIR" && ls *.vpx | head -1); fi
[[ -n $VPX && -f $TABLE_DIR/$VPX ]] || { echo "no .vpx in $SRC" >&2; exit 1; }

"$ROOT/tools/steam-frame/in-image.sh" cargo build --profile web --target wasm32-wasip1 -p vpi-web
mkdir -p "$OUT/table"
cp "$ROOT/engine/web/site/index.html" "$ROOT/engine/web/site/vpi.js" "$ROOT/engine/web/site/gpu.js" "$OUT/"
cp "$ROOT/target/wasm32-wasip1/web/vpi_web.wasm" "$OUT/vpi.wasm"
EXCLUDE=(--exclude '.DS_Store')
[[ $BACKGLASS == 1 ]] || EXCLUDE+=(--exclude '*.directb2s')
rsync -a --delete --prune-empty-dirs "${EXCLUDE[@]}" "$TABLE_DIR/" "$OUT/table/"
rsync -a --delete "$ROOT/vpx-build/vpinball-5436/scripts/" "$OUT/scripts/"
python3 -I - "$OUT" "$VPX" "$TITLE" "$BYLINE" <<'PY'
import json, os, re, sys, urllib.parse
out, vpx, title, byline = sys.argv[1:5]
files = []
for root, _, names in [w for top in ('table', 'scripts') for w in os.walk(os.path.join(out, top))]:
    for n in sorted(names):
        full = os.path.join(root, n)
        rel = os.path.relpath(full, out).replace(os.sep, '/')
        files.append({'path': '/' + rel, 'url': urllib.parse.quote(rel), 'size': os.path.getsize(full)})
stem = os.path.splitext(vpx)[0]
manifest = {
    'title': title or stem,
    'byline': byline,
    'id': re.sub(r'[^a-z0-9]+', '-', stem.lower()).strip('-'),
    'table': '/table/' + vpx,
    'files': files,
}
json.dump(manifest, open(os.path.join(out, 'manifest.json'), 'w'), indent=1)
total = sum(f['size'] for f in files) + os.path.getsize(os.path.join(out, 'vpi.wasm'))
print(f"{out}: {len(files)} files, {total / 1e6:.0f} MB in all")
PY
