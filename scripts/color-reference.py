"""Ground truth for the camera color math (color/src/camera.rs): what the
Adobe DNG SDK 1.4 computes for each corpus DNG, with the tag values it read.

For every file it runs the SDK probe (scripts/dng-sdk-probe, see its
README.md) and reads the color tags from IFD0 with tifffile, so the fixture
holds both the inputs and the SDK's results and the color crate's test needs
no corpus. The pipeline crate's test checks that Genzo's decoder reads the
same inputs from the corpus files.

Two entries come from patched copies (made in a temporary directory):
  - the iPhone 16 Pro ProRAW is DNG 1.7 with JPEG XL tiles, which SDK 1.4
    refuses: the copy says DNGBackwardVersion 1.4 and Compression 7. Only
    tags are read, never pixels, so the color math is unaffected.
  - the Leica Q2 copy has CalibrationIlluminant2 set to 0 (unknown), which
    takes the SDK's single-matrix branch (both matrices count as 5000 K).

Output: backend/crates/color/tests/fixtures/dng-sdk-color.jsonl (one file per
line) and dng-sdk-temperature.jsonl (temperature and tint to xy).

  uv run --no-project --with tifffile --with numpy python scripts/color-reference.py --probe PATH/color_probe.exe
"""
import argparse
import json
import shutil
import struct
import subprocess
import tempfile
from pathlib import Path

import tifffile

ROOT = Path(__file__).resolve().parent.parent
CORPUS = ROOT / "corpus"
OUT = ROOT / "backend/crates/color/tests/fixtures"

FILES = [
    "bayer-dng/iphone/Halide_iPhone16Pro_bayer.dng",
    "raw/Apple/iPhone 12 Pro/IMG_1361.DNG",
    "raw/Google/Pixel 4a/PXL_20201121_100251397.dng",
    "raw/Leica/Q2/L1000750.DNG",
    "raw/Pentax/K10D/IMGP5398.DNG",
    "raw/Samsung/Galaxy S23 Ultra/20230817_120455.dng",
    # Adobe DNG Converter output (scripts/make-dng-references.ps1): Adobe's
    # profiles, with ForwardMatrix, CameraCalibration and signatures.
    "reference/20230817_120455/20230817_120455.dng",
    "reference/7RM5-LosslessCompressedLarge/7RM5-LosslessCompressedLarge.dng",
    "reference/APS-C_CRAW/APS-C_CRAW.dng",
    "reference/DSCF0021/DSCF0021.dng",
    "reference/IMG_1361/IMG_1361.dng",
    "reference/IPhone16proRaw/IPhone16proRaw.dng",
    "reference/Nikon_-_NIKON_Z_9_-_14bit_compressed_(Lossless)/Nikon_-_NIKON_Z_9_-_14bit_compressed_(Lossless).dng",
    "reference/Nikon_Z8_high_efficiency_low/Nikon_Z8_high_efficiency_low.dng",
    "reference/Nikon-D850-12bit-compressed/Nikon-D850-12bit-compressed.dng",
    "reference/P1126458_mechanical/P1126458_mechanical.dng",
    "reference/PXL_20201121_100251397/PXL_20201121_100251397.dng",
]
PRORAW = "proraw/iPhone 16 Pro/IPhone16proRaw.DNG"
LEICA = "raw/Leica/Q2/L1000750.DNG"

TAGS = {
    "color_matrix_1": 50721, "color_matrix_2": 50722,
    "camera_calibration_1": 50723, "camera_calibration_2": 50724,
    "analog_balance": 50727, "as_shot_neutral": 50728, "as_shot_white_xy": 50729,
    "illuminant_1": 50778, "illuminant_2": 50779,
    "camera_calibration_signature": 50931, "profile_calibration_signature": 50932,
    "forward_matrix_1": 50964, "forward_matrix_2": 50965,
}
# DNG 1.6 third illuminant: SDK 1.4 does not know it. None of the corpus has it.
UNSUPPORTED = {52529: "CalibrationIlluminant3", 52531: "ColorMatrix3"}


def numbers(tag):
    """A tag's values as floats; RATIONALs come as flat (num, den) pairs."""
    v = tag.value
    v = list(v) if isinstance(v, (tuple, list)) else [v]
    if tag.dtype in (5, 10):
        v = [v[i] / v[i + 1] for i in range(0, len(v), 2)]
    return [float(x) for x in v]


def inputs(path):
    with tifffile.TiffFile(path) as t:
        tags = t.pages[0].tags
        for code, name in UNSUPPORTED.items():
            assert code not in tags, f"{path}: {name} is not supported"
        out = {}
        for name, code in TAGS.items():
            if code not in tags:
                continue
            tag = tags[code]
            if name.endswith("signature"):
                v = tag.value
                out[name] = v.split(b"\0")[0].decode() if isinstance(v, bytes) else v
            elif name.startswith("illuminant"):
                out[name] = int(tag.value)
            else:
                out[name] = numbers(tag)
        return out


def ifds(data, e, ifd, seen):
    while ifd and ifd not in seen:
        seen.add(ifd)
        n = struct.unpack(e + "H", data[ifd:ifd + 2])[0]
        entries = [ifd + 2 + 12 * i for i in range(n)]
        yield entries
        for at in entries:
            tag, _, count = struct.unpack(e + "HHI", data[at:at + 8])
            if tag == 330:  # SubIFDs
                off = at + 8 if count == 1 else struct.unpack(e + "I", data[at + 8:at + 12])[0]
                for k in range(count):
                    sub = struct.unpack(e + "I", data[off + 4 * k:off + 4 * k + 4])[0]
                    yield from ifds(data, e, sub, seen)
        ifd = struct.unpack(e + "I", data[ifd + 2 + 12 * n:ifd + 6 + 12 * n])[0]


def patched(src, dst, patch):
    """Copies src to dst and calls patch(entry, tag, value bytes, ifd index)
    for every IFD entry; it returns new 4 value bytes or None."""
    shutil.copyfile(src, dst)
    with open(dst, "r+b") as f:
        data = f.read()
        e = "<" if data[:2] == b"II" else ">"
        for index, entries in enumerate(ifds(data, e, struct.unpack(e + "I", data[4:8])[0], set())):
            for at in entries:
                tag = struct.unpack(e + "H", data[at:at + 2])[0]
                new = patch(e, tag, data[at + 8:at + 12], index)
                if new is not None:
                    f.seek(at + 8)
                    f.write(new)
    return dst


def patch_proraw(e, tag, value, _):
    if tag == 50707:  # DNGBackwardVersion, BYTE[4]
        return bytes([1, 4, 0, 0])
    if tag == 259 and struct.unpack(e + "H", value[:2])[0] == 52546:  # JPEG XL
        return struct.pack(e + "HH", 7, 0)
    return None


def patch_single(e, tag, value, index):
    if tag == 50779 and index == 0:  # CalibrationIlluminant2, SHORT
        return struct.pack(e + "HH", 0, 0)
    return None


def probe(exe, path):
    out = subprocess.run([exe, Path(path).as_posix()], capture_output=True, text=True, check=True).stdout
    record = json.loads(out)
    assert "error" not in record, (path, record)
    return {"as_shot_source": record["as_shot_source"], "specs": record["specs"]}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--probe", required=True, help="the built color_probe executable")
    exe = ap.parse_args().probe
    OUT.mkdir(parents=True, exist_ok=True)
    lines = []
    with tempfile.TemporaryDirectory() as tmp:
        cases = [(key, None, CORPUS / key) for key in FILES]
        cases.append((PRORAW, "patched_version_and_compression",
                      patched(CORPUS / PRORAW, Path(tmp) / "proraw.dng", patch_proraw)))
        cases.append((LEICA, "illuminant_2_unknown",
                      patched(CORPUS / LEICA, Path(tmp) / "leica.dng", patch_single)))
        for key, variant, path in cases:
            record = {"key": key, "variant": variant, "inputs": inputs(path), "sdk": probe(exe, path)}
            lines.append(json.dumps(record))
            white = record["sdk"]["specs"][0]
            print(f"{key} {variant or ''}: {white['temperature']:.0f} K, tint {white['tint']:+.1f}")
    (OUT / "dng-sdk-color.jsonl").write_text("\n".join(lines) + "\n", encoding="utf-8")
    temps = subprocess.run([exe, "--temperature"], capture_output=True, text=True, check=True).stdout
    (OUT / "dng-sdk-temperature.jsonl").write_text(temps.replace("\r\n", "\n"), encoding="utf-8")


main()
