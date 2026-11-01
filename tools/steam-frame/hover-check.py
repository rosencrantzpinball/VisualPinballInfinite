"""Checks a `frame.sh panel` run with the pointer moved down the library's left column (`--moves`) for the
column breaking up: each picture (shot-<ms>.png) against the run's first, taken before any move, counting the
pixels changed in the column away from the pointer's row (none when it draws whole), in the pointer's band,
and in the rest of the page. The breakup this caught: run.sh's TU_AUTOTUNE_ALGO=profiled in the panel
(docs/steam-frame/PROGRESS.md, hover-tune-1).

usage: python3 -I tools/steam-frame/hover-check.py <run folder> '<ms:x,y;... as given to --moves>' <first picture's ms>
Pictures are read with ffmpeg as raw RGB (no Python imaging library needed).
"""
import os
import subprocess
import sys

run, moves_text, first = sys.argv[1], sys.argv[2], int(sys.argv[3])
LIMIT = 24  # a channel moved by more than this counts as changed


def size_of(path):
    """A PNG's width and height, from its header."""
    with open(path, "rb") as f:
        head = f.read(24)
    return int.from_bytes(head[16:20], "big"), int.from_bytes(head[20:24], "big")


W, H = size_of(os.path.join(run, f"shot-{first}.png"))
# at 1600 px wide (the app's page at 125 %): the left column 300 px wide; rows within 34 px of the pointer's
# its own (the hovered row and its ring); in proportion at other sizes
COLUMN = 300 * W // 1600
BAND = 34 * W // 1600


def pixels(ms):
    path = os.path.join(run, f"shot-{ms}.png")
    raw = subprocess.run(["ffmpeg", "-loglevel", "error", "-i", path, "-f", "rawvideo", "-pix_fmt", "rgb24", "-"], capture_output=True, check=True).stdout
    assert len(raw) == W * H * 3, (path, len(raw))
    return raw


moves = []
for item in moves_text.split(";"):
    t, xy = item.split(":")
    x, y = xy.split(",")
    moves.append((int(t), float(x), float(y)))

base = pixels(first)
shots = sorted(int(f[5:-4]) for f in os.listdir(run) if f.startswith("shot-") and f.endswith(".png"))
print(f"{'shot':>6} {'pointer':>11} {'column, away':>13} {'column, band':>13} {'rest':>8}  rows changed away from the pointer")
for ms in shots:
    if ms == first:
        continue
    now = pixels(ms)
    at = [m for m in moves if m[0] <= ms]
    px, py = (at[-1][1], at[-1][2]) if at else (-1, -1)
    away = band = rest = 0
    rows = set()
    for y in range(H):
        row = y * W * 3
        for x in range(W):
            i = row + x * 3
            if max(abs(now[i] - base[i]), abs(now[i + 1] - base[i + 1]), abs(now[i + 2] - base[i + 2])) <= LIMIT:
                continue
            if x < COLUMN:
                if px < COLUMN and abs(y - py) <= BAND:
                    band += 1
                else:
                    away += 1
                    rows.add(y // 10 * 10)
            else:
                rest += 1
    spans = sorted(rows)
    print(f"{ms:>6} {f'{px:.0f},{py:.0f}':>11} {away:>13} {band:>13} {rest:>8}  {spans[:1] + spans[-1:] if spans else ''}")
