#!/usr/bin/env python3
"""Fetch the raw.pixls.us part of the test corpus into corpus/raw/.

Picks one file per (maker, model) in CORPUS below, downloads it, and checks or
records its sha256 in corpus/manifest.json. Files already present are kept.

Every file must be CC0: its license is looked up by sha256 in raw.pixls.us's own
repository data and recorded in the manifest. A file under any other license is
refused and deleted, since golden images derived from corpus files are committed
to this repository.

  python scripts/fetch-raw-corpus.py                 # the whole corpus (~700 MB)
  python scripts/fetch-raw-corpus.py --only KEY ...  # only these manifest keys
  python scripts/fetch-raw-corpus.py --list FILE     # the keys in FILE, one per line

`--only` and `--list` download straight from the manifest's key and sha256,
without listing the site. CI uses `--list corpus/golden-inputs.txt` for the files
the golden-image tests read.

Standard library only. Run from anywhere.
"""
import argparse, hashlib, json, re, sys, urllib.parse, urllib.request
from pathlib import Path

BASE = "https://raw.pixls.us/data"
REPOSITORY = "https://raw.pixls.us/json/getrepository.php?set=all"
CC0 = "Creative Commons 0 - Public Domain"
ROOT = Path(__file__).resolve().parent.parent / "corpus"
MANIFEST = ROOT / "manifest.json"
TIMEOUT = 120  # seconds per request; an outage fails instead of hanging

# (maker, model, why[, filename substring]). One recent body per maker, plus the files a specific check needs.
CORPUS = [
    ("Nikon", "Z 8", "High Efficiency RAW check"),
    ("Nikon", "Z 9", "High Efficiency RAW check", "Lossy_High_Efficiency)"),
    ("Nikon", "Z 9", "lossless baseline for the same body", "Lossless"),
    ("Nikon", "D850", "classic NEF, lossless compressed"),
    ("Canon", "Canon EOS R5m2", "CR3 on a recent body"),
    ("Sony", "ILCE-7RM5", "61 MP ARW, lossless compressed"),
    ("Fujifilm", "X-T5", "X-Trans sensor"),
    ("Fujifilm", "GFX100S", "medium format"),
    ("Panasonic", "DC-S5M2", "RW2"),
    ("Olympus", "E-M1MarkII", "ORF"),
    ("OM Digital Solutions", "OM-3", "newest OM body"),
    ("Leica", "Q2", "DNG from a maker"),
    ("Pentax", "K10D", "PEF"),
    ("Hasselblad", "H3D", "3FR"),
    ("Phase One", "P65+", "IIQ"),
    ("Samsung", "Galaxy S23 Ultra", "phone DNG, 200 MP"),
    ("GoPro", "HERO6 Black", "GPR"),
    ("Kodak", "DCS760C", "legacy"),
    ("Apple", "iPhone 12 Pro", "ProRAW, lossless JPEG, not white balanced in the camera", "IMG_1361"),
]

RAW_EXT = re.compile(r"\.(nef|nrw|cr2|cr3|crw|arw|raf|rw2|orf|dng|pef|3fr|iiq|gpr|dcr|kdc|srw|raw|rwl|erf|mef|mrw|srf|sr2)$", re.I)


def get(url):
    with urllib.request.urlopen(url, timeout=TIMEOUT) as r:
        return r.read()


def listing(url):
    html = get(url).decode()
    return [urllib.parse.unquote(h) for h in re.findall(r'href="([A-Za-z0-9][^"?]*)"', html)]


def licenses():
    """sha256 -> license title, from raw.pixls.us's repository data."""
    rows = json.loads(get(REPOSITORY))["data"]
    out = {}
    for row in rows:
        digest = re.search(r"([0-9a-f]{64})", row[7])
        title = re.search(r"title='([^']*)'", row[5])
        if digest:
            out[digest.group(1)] = title.group(1) if title else "unknown"
    return out


def download(key, dest):
    _, maker, model, name = key.split("/", 3)
    url = f"{BASE}/{urllib.parse.quote(maker)}/{urllib.parse.quote(model)}/{urllib.parse.quote(name)}"
    print(f"fetch {key}")
    dest.parent.mkdir(parents=True, exist_ok=True)
    tmp = dest.with_suffix(dest.suffix + ".part")
    with urllib.request.urlopen(url, timeout=TIMEOUT) as r, open(tmp, "wb") as f:
        while chunk := r.read(1 << 20):
            f.write(chunk)
    tmp.replace(dest)


def fetch(key, why, manifest, license_of):
    dest = ROOT / key
    if not dest.exists():
        download(key, dest)
    digest = hashlib.sha256(dest.read_bytes()).hexdigest()
    if key in manifest and manifest[key]["sha256"] != digest:
        raise RuntimeError(f"sha256 mismatch for {key}")
    license = license_of().get(digest, "not listed on raw.pixls.us")
    if license != CC0:
        dest.unlink()
        raise RuntimeError(f"{key} is not CC0 ({license}); deleted")
    manifest[key] = {"sha256": digest, "bytes": dest.stat().st_size, "why": why, "license": "CC0-1.0"}


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--only", nargs="+", metavar="KEY", help="fetch only these manifest keys")
    parser.add_argument("--list", metavar="FILE", help="fetch the keys in FILE, one per line; # starts a comment")
    args = parser.parse_args()
    if args.list:
        lines = Path(args.list).read_text(encoding="utf-8").splitlines()
        args.only = (args.only or []) + [l.strip() for l in lines if l.strip() and not l.startswith("#")]

    manifest = json.loads(MANIFEST.read_text()) if MANIFEST.exists() else {}
    cache = {}

    def license_of():
        if "map" not in cache:
            cache["map"] = licenses()
        return cache["map"]

    failed = []
    if args.only:
        for key in args.only:
            try:
                if key not in manifest:
                    raise RuntimeError("not in corpus/manifest.json")
                fetch(key, manifest[key]["why"], manifest, license_of)
            except Exception as e:
                print(f"FAILED {key}: {e}", file=sys.stderr)
                failed.append(key)
    else:
        for maker, model, why, *pick in CORPUS:
            try:
                dir_url = f"{BASE}/{urllib.parse.quote(maker)}/{urllib.parse.quote(model)}/"
                names = [n for n in listing(dir_url) if RAW_EXT.search(n) and (not pick or pick[0] in n)]
                if not names:
                    raise RuntimeError("no raw files listed")
                fetch(f"raw/{maker}/{model}/{sorted(names)[0]}", why, manifest, license_of)
            except Exception as e:  # keep going; report at the end
                print(f"FAILED {maker}/{model}: {e}", file=sys.stderr)
                failed.append(f"{maker}/{model}")
    MANIFEST.write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n", newline="\n")
    print(f"{len(manifest)} files in manifest, {len(failed)} failed")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
