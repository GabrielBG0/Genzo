"""Ground truth for the GainMap CPU reference (pipeline/tests/gain_map.rs):
Adobe DNG Converter's linear rendering of the Google Pixel 4a DNG (CC0,
raw.pixls.us), reduced to per-channel means of 64 x 64 blocks.

The Pixel 4a stores four 30 x 40 lens-shading GainMaps (one per Bayer site,
gains 1.0 to 3.4) in OpcodeList2. Adobe's linear output has them applied (its
OpcodeList3 warp is the identity, so pixels stay where they are). The test
applies Genzo's GainMap to the raw mosaic and checks that its block means
are proportional to Adobe's in every block.

Input: corpus/reference/PXL_20201121_100251397/PXL_20201121_100251397.dng,
made by scripts/make-dng-references.ps1 (Adobe DNG Converter, -l linear).
Output: backend/crates/pipeline/tests/fixtures/pixel4a-adobe-blocks.f32,
little-endian f32, blocks row by row, R G B per block, normalized by the
reference's black and white levels.

  uv run --no-project --with tifffile --with numpy python scripts/gainmap-reference.py
"""
from pathlib import Path

import numpy as np
import tifffile

ROOT = Path(__file__).resolve().parent.parent
REF = ROOT / "corpus/reference/PXL_20201121_100251397/PXL_20201121_100251397.dng"
OUT = ROOT / "backend/crates/pipeline/tests/fixtures/pixel4a-adobe-blocks.f32"
BLOCK = 64

with tifffile.TiffFile(REF) as t:
    raw = next(p for p in t.series if p.shape == (3024, 4032, 3))
    page = raw.pages[0] if hasattr(raw, "pages") else raw
    tags = page.tags

    def level(name):
        # A RATIONAL tag comes as flat (numerator, denominator) pairs.
        v = tags[name].value
        v = list(v) if isinstance(v, (tuple, list)) else [v]
        if tags[name].dtype in (5, 10):
            v = [v[i] / v[i + 1] for i in range(0, len(v), 2)]
        return float(np.mean(v))

    black, white = level("BlackLevel"), level("WhiteLevel")
    assert (black, white) == (4096.0, 65535.0), (black, white)
    img = raw.asarray().astype(np.float64)

norm = (img - black) / (white - black)
h, w = norm.shape[0] // BLOCK, norm.shape[1] // BLOCK
blocks = norm[: h * BLOCK, : w * BLOCK].reshape(h, BLOCK, w, BLOCK, 3).mean(axis=(1, 3))
OUT.parent.mkdir(parents=True, exist_ok=True)
OUT.write_bytes(blocks.astype("<f4").tobytes())
print(f"black {black}, white {white}: {h} x {w} blocks of {BLOCK} px to {OUT}")
