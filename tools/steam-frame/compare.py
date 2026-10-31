"""Compares two pictures of a table, ours and VPX's, pixel by pixel.

  compare.py ours.png vpx.png diff.png [--ignore x,y,w,h ...]

Prints the mean absolute difference (0-255 per channel) and the share of pixels that differ by more than
24 of 255 in any channel, as VPI's own picture check counts them (a table passes at 0.5 % or less:
visualpinballinfinite.org/resources/tested-tables), and writes a difference picture: grey where the same,
red where they differ, brighter the more. Regions given with --ignore (the score display, which
shows a different moment) are left out of the numbers and shown blue.
"""
import argparse

import numpy as np
from PIL import Image


def main():
    p = argparse.ArgumentParser()
    p.add_argument("ours")
    p.add_argument("vpx")
    p.add_argument("diff")
    p.add_argument("--ignore", action="append", default=[])
    a = p.parse_args()
    ours = np.asarray(Image.open(a.ours).convert("RGB")).astype(np.int16)
    vpx = np.asarray(Image.open(a.vpx).convert("RGB")).astype(np.int16)
    if ours.shape != vpx.shape:
        raise SystemExit(f"sizes differ: {ours.shape} and {vpx.shape}")
    d = np.abs(ours - vpx)
    keep = np.ones(d.shape[:2], bool)
    for box in a.ignore:
        x, y, w, h = (int(v) for v in box.split(","))
        keep[y:y + h, x:x + w] = False
    differ = (d.max(axis=2) > 24) & keep
    print(f"mean abs diff {d[keep].mean():.2f}, differing (>24) {100 * differ.sum() / keep.sum():.2f} % of {keep.sum()} px")
    grey = (ours.mean(axis=2) * 0.35).astype(np.uint8)
    out = np.stack([grey, grey, grey], axis=2)
    m = d.max(axis=2)
    red = (m > 24) & keep
    out[red] = np.stack([np.clip(80 + m[red] * 2, 0, 255), np.zeros(red.sum()), np.zeros(red.sum())], axis=1).astype(np.uint8)
    out[~keep] = (out[~keep] * [0.3, 0.3, 1.0]).astype(np.uint8)
    Image.fromarray(out).save(a.diff)


if __name__ == "__main__":
    main()
