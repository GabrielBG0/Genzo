"""Ground truth for CAT16 in the non-RAW white balance (color/src/white_balance.rs,
DEV-01, *White balance*): colour-science's von Kries adaptation with the CAT16
matrix, an implementation independent of Genzo's.

Writes backend/crates/color/tests/fixtures/white-balance-colour.jsonl: one
record per pair of whites, with the xy of each and the 3x3 XYZ matrix that
adapts the first to the second.

  uv run --no-project --with colour-science==0.4.7 python scripts/white-balance-reference.py
"""
import json
import warnings
from pathlib import Path

warnings.filterwarnings("ignore")  # colour warns about optional packages

import colour  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "backend/crates/color/tests/fixtures/white-balance-colour.jsonl"

D65 = [0.3127, 0.3290]
# Whites the non-RAW sliders reach (about 3940 K to 18,600 K, tint off the
# locus both ways), plus illuminants A and D50.
WHITES = [
    [0.3820, 0.3770],
    [0.2560, 0.2630],
    [0.3300, 0.3600],
    [0.3000, 0.2950],
    [0.44757, 0.40745],
    [0.3457, 0.3585],
]


def main():
    lines = []
    for w in WHITES:
        for src, dst in [(w, D65), (D65, w)]:
            m = colour.adaptation.matrix_chromatic_adaptation_VonKries(
                colour.xy_to_XYZ(src), colour.xy_to_XYZ(dst), transform="CAT16"
            )
            lines.append(json.dumps({"from": src, "to": dst, "cat16": m.tolist()}))
    OUT.write_text("\n".join(lines) + "\n")
    print(f"{len(lines)} records -> {OUT.relative_to(ROOT)}")


if __name__ == "__main__":
    main()
