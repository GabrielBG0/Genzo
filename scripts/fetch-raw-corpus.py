#!/usr/bin/env python3
"""Fetch the raw.pixls.us part of the 0.C test corpus into corpus/raw/.

Picks one file per (maker, model) in CORPUS below, downloads it, and checks
or records its sha256 in corpus/manifest.json. Files already present are kept.
Standard library only. Run from anywhere: python scripts/fetch-raw-corpus.py
"""
import hashlib, json, re, sys, urllib.parse, urllib.request
from pathlib import Path

BASE = "https://raw.pixls.us/data"
ROOT = Path(__file__).resolve().parent.parent / "corpus"
MANIFEST = ROOT / "manifest.json"

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
]

RAW_EXT = re.compile(r"\.(nef|nrw|cr2|cr3|crw|arw|raf|rw2|orf|dng|pef|3fr|iiq|gpr|dcr|kdc|srw|raw|rwl|erf|mef|mrw|srf|sr2)$", re.I)


def listing(url):
    html = urllib.request.urlopen(url, timeout=60).read().decode()
    return [urllib.parse.unquote(h) for h in re.findall(r'href="([A-Za-z0-9][^"?]*)"', html)]


def main():
    manifest = json.loads(MANIFEST.read_text()) if MANIFEST.exists() else {}
    failed = []
    for maker, model, why, *pick in CORPUS:
        try:
            dir_url = f"{BASE}/{urllib.parse.quote(maker)}/{urllib.parse.quote(model)}/"
            names = [n for n in listing(dir_url) if RAW_EXT.search(n) and (not pick or pick[0] in n)]
            if not names:
                raise RuntimeError("no raw files listed")
            name = sorted(names)[0]
            dest = ROOT / "raw" / maker / model / name
            key = f"raw/{maker}/{model}/{name}"
            if not dest.exists():
                dest.parent.mkdir(parents=True, exist_ok=True)
                print(f"fetch {key}")
                urllib.request.urlretrieve(dir_url + urllib.parse.quote(name), dest)
            digest = hashlib.sha256(dest.read_bytes()).hexdigest()
            if key in manifest and manifest[key]["sha256"] != digest:
                raise RuntimeError(f"sha256 mismatch for {key}")
            manifest[key] = {"sha256": digest, "bytes": dest.stat().st_size, "why": why}
        except Exception as e:  # keep going; report at the end
            print(f"FAILED {maker}/{model}: {e}", file=sys.stderr)
            failed.append(f"{maker}/{model}")
    MANIFEST.write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n")
    print(f"{len(manifest)} files in manifest, {len(failed)} failed")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
