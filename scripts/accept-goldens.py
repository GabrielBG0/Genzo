#!/usr/bin/env python3
"""Accept golden-image candidates: copy renders a failed run wrote into the
crates' tests/goldens/ folders (see backend/crates/golden).

  python scripts/accept-goldens.py                  # from backend/target/golden-actual
  python scripts/accept-goldens.py DIR              # from a folder, such as a downloaded artifact
  python scripts/accept-goldens.py --run RUN_ID     # download a CI run's golden-actual-* artifacts first
  ... --only decode/iphone-12-pro spike/synthetic-scene   # accept only these cases

GPU renders are accepted only from lavapipe, the reference device; renders from
any other adapter are refused. CPU renders are accepted from any machine.
Every accepted golden needs a stated reason in the PR that commits it.

Standard library only (plus the gh CLI for --run). Run from anywhere.
"""
import argparse, json, shutil, subprocess, sys, tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CRATES = ROOT / "backend" / "crates"


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("dir", nargs="?", default=str(ROOT / "backend" / "target" / "golden-actual"))
    parser.add_argument("--run", help="a GitHub Actions run id to download artifacts from")
    parser.add_argument("--only", nargs="+", metavar="CASE", help="accept only these case names")
    args = parser.parse_args()

    source = Path(args.dir)
    if args.run:
        source = Path(tempfile.mkdtemp(prefix="goldens-"))
        subprocess.run(
            ["gh", "run", "download", args.run, "--pattern", "golden-actual-*", "--dir", str(source)],
            check=True,
        )

    metas = sorted(source.rglob("*.meta.json"))
    if not metas:
        print(f"no candidates under {source}")
        return 1
    accepted, refused = [], []
    for meta_path in metas:
        meta = json.loads(meta_path.read_text(encoding="utf-8"))
        case, crate = meta["case"], meta["crate"]
        if args.only and case not in args.only:
            continue
        render = meta_path.with_name(meta_path.name.replace(".meta.json", ".f16.png"))
        if meta["producer"] == "gpu" and "llvmpipe" not in meta["adapter"].lower():
            refused.append(f"{crate}/{case}: rendered on {meta['adapter']!r}, not lavapipe")
            continue
        if not render.exists():
            refused.append(f"{crate}/{case}: {render.name} is missing")
            continue
        dest = CRATES / crate / "tests" / "goldens" / f"{case}.f16.png"
        dest.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(render, dest)
        accepted.append(f"{dest.relative_to(ROOT)}  ({meta['reason']})")

    for line in accepted:
        print(f"accepted {line}")
    for line in refused:
        print(f"REFUSED  {line}", file=sys.stderr)
    if accepted:
        print("State the reason for each new or changed golden in the PR.")
    # A CI run uploads every adapter's renders, so refusing the non-lavapipe
    # ones is normal; only accepting nothing is a failure.
    return 0 if accepted else 1


if __name__ == "__main__":
    sys.exit(main())
