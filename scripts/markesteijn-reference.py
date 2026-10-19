"""Ground truth for the X-Trans demosaic (DEV-01, stage 3): LibRaw's own
Markesteijn (dcraw's xtrans_interpolate, 3 passes) on the Fujifilm X-T5
file, as interior crops.

LibRaw runs xtrans_interpolate(quality > 2 ? 3 : 1) for X-Trans
(src/postprocessing/dcraw_process.cpp), so rawpy's AHD (quality 3) is the
3-pass version. Its homogeneity test works in CIELab through the camera
matrix where darktable's uses YPbPr, so the two differ a little by design;
the test compares Genzo's demosaic with LibRaw's and asks that a plain
linear demosaic be clearly farther from it.

Output, camera RGB, no white balance, linear, 16 bits (raw output color,
gamma 1, no auto brightness):
backend/crates/pipeline/tests/fixtures/libraw-xt5-markesteijn.bin: b"GZX1",
little-endian u32 crop count, then per crop u32 sensor x, y, width, height
and u16 RGB samples row by row.

  uv run --no-project --with rawpy --with numpy python scripts/markesteijn-reference.py
"""
import struct
from pathlib import Path

import numpy as np
import rawpy

ROOT = Path(__file__).resolve().parent.parent
SRC = ROOT / "corpus/raw/Fujifilm/X-T5/DSCF0021.RAF"
OUT = ROOT / "backend/crates/pipeline/tests/fixtures/libraw-xt5-markesteijn.bin"
SIZE = 96
# Sensor coordinates, multiples of 6, well inside the image.
CROPS = [(1200, 1200), (3000, 2400), (5400, 1800), (6600, 4200)]

with rawpy.imread(str(SRC)) as raw:
    top, left = raw.sizes.top_margin, raw.sizes.left_margin
    rgb = raw.postprocess(
        demosaic_algorithm=rawpy.DemosaicAlgorithm.AHD,
        output_color=rawpy.ColorSpace.raw,
        gamma=(1, 1),
        no_auto_bright=True,
        use_camera_wb=False,
        use_auto_wb=False,
        user_wb=[1.0, 1.0, 1.0, 1.0],
        output_bps=16,
        user_flip=0,
        highlight_mode=rawpy.HighlightMode.Clip,
    )
    print(f"LibRaw: margins top {top} left {left}, output {rgb.shape}, "
          f"black {raw.black_level_per_channel}, white {raw.white_level}")
out = bytearray(b"GZX1") + struct.pack("<I", len(CROPS))
for x, y in CROPS:
    crop = rgb[y - top:y - top + SIZE, x - left:x - left + SIZE]
    assert crop.shape == (SIZE, SIZE, 3), crop.shape
    out += struct.pack("<4I", x, y, SIZE, SIZE) + crop.astype("<u2").tobytes()
OUT.write_bytes(bytes(out))
print(f"{len(CROPS)} crops of {SIZE} px to {OUT} ({len(out)} bytes)")
