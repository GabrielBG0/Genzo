"""Ground truth for the CFA decode path (DEV-01 stage 0): what LibRaw reads
from each camera RAW file, for comparison with what Genzo decodes through
rawler. LibRaw (through rawpy) is used here only as an independent
reference; Genzo never links it (no LibRaw fallback, decided).

For each file it prints the full sensor size, LibRaw's margins, the color
of each site of the repeating pattern at the sensor origin, the black and
white levels, the camera white balance, and the means and a position-weighted
checksum of every sample, computed exactly as backend/crates/imgio/tests/
cfa_corpus.rs computes them on Genzo's output.

  uv run --no-project --with rawpy --with numpy python scripts/cfa-reference.py corpus/raw/...
"""
import sys

import numpy as np
import rawpy


def checksum(samples):
    flat = samples.reshape(-1).astype(np.uint64)
    weights = (np.arange(flat.size, dtype=np.uint64) % 65521) + 1
    return int((flat * weights).sum(dtype=np.uint64))


for path in sys.argv[1:]:
    print(path)
    try:
        with rawpy.imread(path) as raw:
            img = raw.raw_image.copy()
            colors = raw.raw_colors.copy()
            colors[colors == 3] = 1  # LibRaw's second green
            sizes = raw.sizes
            h, w = img.shape
            # The pattern as it repeats from the sensor origin, 6x6 to show
            # X-Trans whole; "RGB"[index].
            pat = ["".join("RGB"[c] for c in row[:6]) for row in colors[:6]]
            print(f"  sensor {w}x{h}, margins top {sizes.top_margin} left {sizes.left_margin}, "
                  f"visible {sizes.width}x{sizes.height}")
            print(f"  pattern from (0,0): {' '.join(pat)}")
            print(f"  black per channel {raw.black_level_per_channel}, white {raw.white_level}")
            # Black per site of the pattern, from the sensor origin.
            sites = raw.raw_colors[:6, :6]
            blacks = [" ".join(str(raw.black_level_per_channel[c]) for c in row) for row in sites]
            print(f"  black from (0,0): {' / '.join(blacks)}")
            print(f"  camera white balance {raw.camera_whitebalance[:3]}")
            n = img.size
            means = [int((int(img[colors == c].astype(np.uint64).sum()) + (colors == c).sum() // 2)
                         // max(1, (colors == c).sum())) for c in range(3)]
            print(f"  means per color {means}, overall {int((int(img.astype(np.uint64).sum()) + n // 2) // n)}")
            print(f"  checksum {checksum(img)}")
    except Exception as e:  # noqa: BLE001 - report and go on
        print(f"  LibRaw: {e}")
