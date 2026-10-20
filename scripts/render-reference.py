"""Ground truth for DEV-02's criterion 5: corpus DNGs rendered by the Adobe
DNG SDK 1.4's dng_render (scripts/dng-sdk-probe/render_probe.cpp) into
linear ProPhoto RGB, as 64 px block means over the default crop.

Exposure +1 EV for every file and 0 EV for those whose BaselineExposure is
0 or more: below a total of 0 EV the SDK darkens its curve instead of
applying a gain (DEV-02, *Where Genzo differs*).

Output: backend/crates/pipeline/tests/fixtures/sdk-render/<name>@<ev>.json,
one per render. The corpus test pipeline/tests/render_sdk.rs reads them.

  python scripts/render-reference.py --probe PATH/render_probe.exe
"""
import argparse
import json
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CORPUS = ROOT / "corpus"
OUT = ROOT / "backend/crates/pipeline/tests/fixtures/sdk-render"

CASES = [
    ("leica-q2", "raw/Leica/Q2/L1000750.DNG", [0, 1]),
    ("pentax-k10d", "raw/Pentax/K10D/IMGP5398.DNG", [1]),
    ("halide-iphone-16-pro", "bayer-dng/iphone/Halide_iPhone16Pro_bayer.dng", [0, 1]),
    ("galaxy-s23-ultra", "raw/Samsung/Galaxy S23 Ultra/20230817_120455.dng", [0, 1]),
    ("iphone-12-pro", "raw/Apple/iPhone 12 Pro/IMG_1361.DNG", [1]),
]
BLOCK = 64


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--probe", required=True, help="the built render_probe executable")
    exe = ap.parse_args().probe
    OUT.mkdir(parents=True, exist_ok=True)
    for name, key, exposures in CASES:
        for ev in exposures:
            out = subprocess.run([exe, (CORPUS / key).as_posix(), str(ev), str(BLOCK)],
                                 capture_output=True, text=True, check=True).stdout
            record = json.loads(out)
            record["key"] = key
            # 6 significant digits are plenty for a 0.5 % tolerance.
            record["means"] = [[[float(f"{v:.6g}") for v in px] for px in row] for row in record["means"]]
            path = OUT / f"{name}@{ev:+d}.json"
            path.write_text(json.dumps(record, separators=(",", ":")) + "\n", encoding="utf-8", newline="\n")
            rows = record["means"]
            print(f"{path.name}: {len(rows[0])} x {len(rows)} blocks, crop {record['crop']}, "
                  f"opcode list 3 cleared: {record['opcode_list_3_cleared']}")


if __name__ == "__main__":
    main()
