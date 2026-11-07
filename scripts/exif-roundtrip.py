#!/usr/bin/env python3
"""OUT-01 criterion 4: ExifTool reads the same metadata from an export's
EXIF as from its source.

`cargo test -p genzo-imgio --test metadata_corpus` leaves, for each corpus
file, a small JPEG carrying the EXIF Genzo writes for an export, and a
`.source` file naming the original. This script runs ExifTool on both and
compares the tags an export must keep. Orientation must be 1 in the export.

Usage: scripts/exif-roundtrip.py [DIR]   (default backend/target/metadata-corpus)
ExifTool is found on PATH, or at %LOCALAPPDATA%/Programs/ExifTool/ExifTool.exe.
"""

import json
import os
import shutil
import subprocess
import sys
from pathlib import Path

# (group, tag) pairs compared; ExifTool's -n values, so numbers compare
# as numbers.
TAGS = [
    ("IFD0", "Make"),
    ("IFD0", "Model"),
    ("IFD0", "Artist"),
    ("IFD0", "Copyright"),
    ("ExifIFD", "DateTimeOriginal"),
    ("ExifIFD", "SubSecTimeOriginal"),
    ("ExifIFD", "OffsetTimeOriginal"),
    ("ExifIFD", "ExposureTime"),
    ("ExifIFD", "FNumber"),
    ("ExifIFD", "ISO"),
    ("ExifIFD", "FocalLength"),
    ("ExifIFD", "LensMake"),
    ("ExifIFD", "LensModel"),
    ("ExifIFD", "ExposureCompensation"),
    ("ExifIFD", "Flash"),
    ("GPS", "GPSLatitude"),
    ("GPS", "GPSLatitudeRef"),
    ("GPS", "GPSLongitude"),
    ("GPS", "GPSLongitudeRef"),
    ("GPS", "GPSAltitude"),
    ("GPS", "GPSDateStamp"),
]


def exiftool() -> str:
    found = shutil.which("exiftool")
    if found:
        return found
    local = Path(os.environ.get("LOCALAPPDATA", "")) / "Programs/ExifTool/ExifTool.exe"
    if local.exists():
        return str(local)
    sys.exit("ExifTool not found")


def read(tool: str, path: str) -> dict:
    out = subprocess.run(
        [tool, "-j", "-n", "-G1", "-IFD0:all", "-ExifIFD:all", "-GPS:all", path],
        capture_output=True,
        check=True,
    ).stdout
    return json.loads(out)[0]


def main() -> int:
    root = Path(__file__).resolve().parent.parent
    folder = Path(sys.argv[1]) if len(sys.argv) > 1 else root / "backend/target/metadata-corpus"
    tool = exiftool()
    sources = sorted(folder.glob("*.source"))
    if not sources:
        sys.exit(f"nothing in {folder}; run the metadata_corpus test first")
    failures = 0
    for s in sources:
        exported = s.with_suffix(".jpg")
        original = s.read_text(encoding="utf-8").strip()
        a = read(tool, original)
        b = read(tool, str(exported))
        diffs = []
        for group, tag in TAGS:
            key = f"{group}:{tag}"
            # A TIFF/EP file (Kodak DCR) keeps Exif tags in IFD0; the
            # export writes them in the Exif IFD.
            want = a.get(key, a.get(f"IFD0:{tag}") if group == "ExifIFD" else None)
            if want != b.get(key):
                diffs.append(f"{key}: {want!r} -> {b.get(key)!r}")
        if b.get("IFD0:Orientation", 1) != 1:
            diffs.append(f"IFD0:Orientation is {b.get('IFD0:Orientation')}")
        kept = sum(1 for g, t in TAGS if f"{g}:{t}" in a or (g == "ExifIFD" and f"IFD0:{t}" in a))
        status = "ok" if not diffs else "DIFFERS"
        print(f"{status:8} {s.stem}: {kept} tags compared")
        for d in diffs:
            print(f"         {d}")
        failures += bool(diffs)
    print(f"{len(sources) - failures} of {len(sources)} match")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
