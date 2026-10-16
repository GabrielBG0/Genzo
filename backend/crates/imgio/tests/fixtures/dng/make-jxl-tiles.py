"""Writes the JPEG XL tiles of the synthetic linear DNG used by src/dng.rs tests.

The image is 40x27 with 3 samples per pixel, in 16x16 tiles (3 across, 2
down), so the right column and the bottom row are partial. Sample (x, y, c)
is (37x + 91y + 13c) mod 1100: past the end of a 1024-entry linearization
table for some pixels. Edge tiles are stored cropped to the image, except
tile 3 (bottom left), which is padded to full size: DNG writers do both.

Each tile is written lossless at a declared bit depth of 16 (t16_<i>.jxl) and
12 (t12_<i>.jxl); the reader must return the same integers for both.
Regenerate with: uv run --no-project --with imagecodecs --with numpy python make-jxl-tiles.py
"""
from pathlib import Path

import imagecodecs
import numpy as np

W, H, T = 40, 27, 16
HERE = Path(__file__).resolve().parent


def sample(x, y, c):
    return (37 * x + 91 * y + 13 * c) % 1100


for i in range(6):
    col, row = i % 3, i // 3
    x0, y0 = col * T, row * T
    w = T if i == 3 else min(T, W - x0)
    h = T if i == 3 else min(T, H - y0)
    tile = np.array(
        [[[sample(x0 + x, y0 + y, c) for c in range(3)] for x in range(w)] for y in range(h)],
        dtype=np.uint16,
    )
    for bits in (16, 12):
        data = imagecodecs.jpegxl_encode(tile, lossless=True, bitspersample=bits)
        assert (imagecodecs.jpegxl_decode(data) == tile).all()
        (HERE / f"t{bits}_{i}.jxl").write_bytes(data)
