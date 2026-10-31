"""Ground truth for imgio's shooting values (DEV-03, criterion 7;
tests/capture_corpus.rs): what ExifTool reads from each file's EXIF IFD.

ExifTool's own EXIF group only (not maker notes or composites), as numbers
(-n), so the values compare with Genzo's rationals. Prints the Rust table
the test holds, with ExifTool's version.

  python scripts/exif-reference.py [--exiftool PATH] CORPUS_DIR

PATH defaults to $EXIFTOOL, else `exiftool` on PATH. A .pl path (the
Image-ExifTool distribution) is run with perl.
"""
import argparse
import json
import os
import pathlib
import subprocess
import sys

SKIP = {".txt", ".md", ".json"}
TAGS = ["ISO", "ExposureTime", "FNumber", "FocalLength"]


def run(exiftool, args):
    cmd = (["perl", exiftool] if exiftool.endswith((".pl", "exiftool")) and os.path.isfile(exiftool) else [exiftool]) + args
    return subprocess.run(cmd, check=True, capture_output=True).stdout


def first(v):
    if isinstance(v, list):
        v = v[0] if v else None
    if isinstance(v, str):
        # A list written as text, "400 800".
        v = v.split()[0] if v.split() else None
        v = float(v) if v is not None else None
    return v


def rust(v, integer=False):
    if v is None or v == 0:
        return "None"
    return f"Some({int(v)})" if integer else f"Some({float(v)!r})"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--exiftool", default=os.environ.get("EXIFTOOL", "exiftool"))
    ap.add_argument("corpus")
    a = ap.parse_args()
    root = pathlib.Path(a.corpus)
    files = sorted(
        p for p in root.rglob("*")
        if p.is_file() and p.suffix.lower() not in SKIP and "reference" not in p.relative_to(root).parts
    )
    version = run(a.exiftool, ["-ver"]).decode().strip()
    out = json.loads(run(a.exiftool, ["-j", "-n", "-q"] + [f"-EXIF:{t}" for t in TAGS] + [str(f) for f in files]))
    by_file = {pathlib.Path(o["SourceFile"]).resolve(): o for o in out}
    print(f"// ExifTool {version}, EXIF group, -n: scripts/exif-reference.py")
    print("const SHOOTING: &[Shooting] = &[")
    for f in files:
        o = by_file.get(f.resolve(), {})
        key = f.relative_to(root).as_posix()
        vals = [first(o.get(t)) for t in TAGS]
        print(
            f'    Shooting {{ file: "{key}", iso: {rust(vals[0], True)}, exposure_time: {rust(vals[1])}, '
            f"f_number: {rust(vals[2])}, focal_length: {rust(vals[3])} }},"
        )
    print("];")


if __name__ == "__main__":
    sys.exit(main())
