"""Ground truth for the Genzo Standard base curve (color/src/tone.rs,
DEV-02): what the Adobe DNG SDK 1.4 computes, from the tone probe
(scripts/dng-sdk-probe, see its README.md).

Writes, under backend/crates/color/tests/fixtures:
  - dng-sdk-tone.json: the ACR3 curve on a 10,001-point grid and its
    4097-entry table, the exposure ramp for five blacks (grid and toe), and
    splines (synthetic point sets and the ProRAW files' ProfileToneCurve)
    with their tables;
  - dng-sdk-rgbtone.f32: RefBaselineRGBTone's output for the probe's shared
    generator (100,000 triples, raw little-endian f32, 3 per triple);
  - dng-sdk-tone-params.jsonl: what dng_render derives for the ramp and the
    curve of each corpus DNG.

The iPhone 16 Pro ProRAW is read from a patched copy (DNG 1.4, Compression
7), as scripts/color-reference.py does: only tags are read, never pixels.

  uv run --no-project --with tifffile --with numpy python scripts/tone-reference.py --probe PATH/tone_probe.exe
"""
import argparse
import importlib.util
import json
import subprocess
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CORPUS = ROOT / "corpus"
OUT = ROOT / "backend/crates/color/tests/fixtures"

spec = importlib.util.spec_from_file_location("color_reference", Path(__file__).parent / "color-reference.py")

PRORAW_12 = "raw/Apple/iPhone 12 Pro/IMG_1361.DNG"
PRORAW_16 = "proraw/iPhone 16 Pro/IPhone16proRaw.DNG"
PARAMS = [
    "raw/Leica/Q2/L1000750.DNG",
    "raw/Pentax/K10D/IMGP5398.DNG",
    "bayer-dng/iphone/Halide_iPhone16Pro_bayer.dng",
    "raw/Samsung/Galaxy S23 Ultra/20230817_120455.dng",
    "raw/Google/Pixel 4a/PXL_20201121_100251397.dng",
    "reference/APS-C_CRAW/APS-C_CRAW.dng",
    PRORAW_12,
]
TRIPLES = 100_000


def stored(path):
    """The tags as the file stores them, before the SDK's rounding."""
    import tifffile
    with tifffile.TiffFile(path) as f:
        tags = {t.code: t.value for t in f.pages[0].tags}

    def value(code):
        v = tags.get(code)
        if v is None:
            return None
        if isinstance(v, tuple) and len(v) == 2 and all(isinstance(x, int) for x in v):
            return v[0] / v[1] if v[1] else 0.0
        return float(v[0] if isinstance(v, tuple) else v)
    return {"baseline_exposure": value(50730), "shadow_scale": value(50739),
            "baseline_exposure_offset": value(51109)}


def run(exe, *args):
    return subprocess.run([exe, *map(str, args)], capture_output=True, text=True, check=True).stdout


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--probe", required=True, help="the built tone_probe executable")
    exe = ap.parse_args().probe
    color_reference = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(color_reference)
    OUT.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory() as tmp:
        proraw_16 = color_reference.patched(CORPUS / PRORAW_16, Path(tmp) / "proraw16.dng",
                                            color_reference.patch_proraw)
        names = {(CORPUS / PRORAW_12).as_posix(): PRORAW_12, proraw_16.as_posix(): PRORAW_16}
        curves = json.loads(run(exe, "--curves", *names))
        for s in curves["splines"]:
            s["name"] = names.get(s["name"], s["name"])
        files = [p for p in curves["splines"] if "/" in p["name"]]
        assert len(files) == 2, [s["name"] for s in curves["splines"]]
        (OUT / "dng-sdk-tone.json").write_text(json.dumps(curves) + "\n", encoding="utf-8", newline="\n")

        lines = []
        for key, path in [(k, CORPUS / k) for k in PARAMS] + [(PRORAW_16, proraw_16)]:
            record = json.loads(run(exe, "--params", path.as_posix()))
            assert "error" not in record, (key, record)
            record["file"] = key
            record["stored"] = stored(path)
            lines.append(json.dumps(record))
            print(f"{key}: curve {record['curve']}, white {record['white']:.6f}, black {record['black']:.6f}")
        (OUT / "dng-sdk-tone-params.jsonl").write_text("\n".join(lines) + "\n", encoding="utf-8", newline="\n")

    run(exe, "--rgbtone", TRIPLES, (OUT / "dng-sdk-rgbtone.f32").as_posix())
    print(f"rgbtone: {(OUT / 'dng-sdk-rgbtone.f32').stat().st_size} bytes")


if __name__ == "__main__":
    main()
