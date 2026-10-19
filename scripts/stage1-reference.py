"""Ground truth for stage 1 (DEV-01): the Adobe DNG SDK 1.4's stage 2 image
of corpus DNGs, the raw image after OpcodeList1, black and white levels and
OpcodeList2, over the ActiveArea (scripts/dng-sdk-probe/stage2_probe.cpp).

The full images are 20 to 95 MB, so the fixture keeps, per file:
  - five 48 x 48 crops at full precision: the top-left and bottom-right
    corners, the center, and two straddling 1024 px tile boundaries;
  - the mean of every 128 x 128 block, per plane.

Output: backend/crates/pipeline/tests/fixtures/sdk-stage2/<name>.bin and
index.json. Each .bin: b"GZC1", then little-endian u32 width, height,
planes, block size and crop count; each crop as u32 x, y, w, h and its
u16 samples (planes interleaved, 65535 = 1); then the block means as f32,
row by row, planes interleaved.

  uv run --no-project --with numpy python scripts/stage1-reference.py --probe PATH/stage2_probe.exe
"""
import argparse
import json
import struct
import subprocess
import tempfile
from pathlib import Path

import numpy as np

ROOT = Path(__file__).resolve().parent.parent
CORPUS = ROOT / "corpus"
OUT = ROOT / "backend/crates/pipeline/tests/fixtures/sdk-stage2"
CROP = 48
BLOCK = 128

FILES = {
    # GainMaps in OpcodeList2 (lens shading), black levels per site.
    "pixel4a": "raw/Google/Pixel 4a/PXL_20201121_100251397.dng",
    # FixBadPixelsConstant in OpcodeList1; active area 32 px in from the left.
    "leica-q2": "raw/Leica/Q2/L1000750.DNG",
    "pentax-k10d": "raw/Pentax/K10D/IMGP5398.DNG",
    "halide-iphone16pro": "bayer-dng/iphone/Halide_iPhone16Pro_bayer.dng",
    # Linear DNGs, three planes; the Samsung has no-op GainMaps.
    "galaxy-s23-ultra": "raw/Samsung/Galaxy S23 Ultra/20230817_120455.dng",
    "iphone12pro": "raw/Apple/iPhone 12 Pro/IMG_1361.DNG",
}


def read_stage2(path):
    data = path.read_bytes()
    assert data[:4] == b"GZS2", path
    w, h, planes, kind = struct.unpack("<4I", data[4:20])
    assert kind == 3, f"{path}: pixel type {kind}, expected 16-bit"
    return np.frombuffer(data[20:], dtype="<u2").reshape(h, w, planes)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--probe", required=True, help="the built stage2_probe executable")
    exe = ap.parse_args().probe
    OUT.mkdir(parents=True, exist_ok=True)
    index = []
    with tempfile.TemporaryDirectory() as tmp:
        for name, key in FILES.items():
            dump = Path(tmp) / f"{name}.bin"
            subprocess.run([exe, (CORPUS / key).as_posix(), dump.as_posix()], check=True,
                           capture_output=True)
            img = read_stage2(dump)
            h, w, planes = img.shape
            crops = [(0, 0), (w - CROP, h - CROP), (w // 2, h // 2),
                     (1024 - CROP // 2, 1024 - CROP // 2), (2048 - CROP // 2, 0)]
            out = bytearray(b"GZC1")
            out += struct.pack("<5I", w, h, planes, BLOCK, len(crops))
            for x, y in crops:
                out += struct.pack("<4I", x, y, CROP, CROP)
                out += img[y:y + CROP, x:x + CROP].astype("<u2").tobytes()
            bh, bw = h // BLOCK, w // BLOCK
            blocks = (img[: bh * BLOCK, : bw * BLOCK].astype(np.float64) / 65535.0)
            blocks = blocks.reshape(bh, BLOCK, bw, BLOCK, planes).mean(axis=(1, 3))
            out += blocks.astype("<f4").tobytes()
            (OUT / f"{name}.bin").write_bytes(bytes(out))
            index.append({"name": name, "key": key, "width": w, "height": h, "planes": planes})
            print(f"{name}: {w} x {h} x {planes}, {len(out)} bytes")
    (OUT / "index.json").write_text(json.dumps(index, indent=1) + "\n", encoding="utf-8", newline="\n")


main()
