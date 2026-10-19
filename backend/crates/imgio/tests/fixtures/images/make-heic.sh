#!/usr/bin/env bash
# Writes heic-p3-nclx.heic: a Display P3 HEIC tagged the way iPhones tag
# theirs (NCLX primaries 12, transfer 13), 64 x 48, with the P3 primaries at
# full saturation and a near-black patch. Lossy (4:2:0), so tests compare
# what imgio decodes, not the scene. Needs Docker (libheif 1.19 in Debian
# trixie; 1.15 in bookworm drops the NCLX box):
#   bash make-heic.sh
set -euo pipefail
cd "$(dirname "$0")"
work=$(mktemp -d)
uv run --no-project --with pillow --with numpy python - "$work/scene.png" <<'PY'
import sys
import numpy as np
from PIL import Image
W, H = 64, 48
y, x = np.mgrid[0:H, 0:W]
rgb = np.stack([x / (W - 1), y / (H - 1), ((x + y) % 7) / 6], axis=-1)
rgb[4:14, 4:14] = [1, 0, 0]
rgb[4:14, 20:30] = [0, 1, 0]
rgb[4:14, 36:46] = [0, 0, 1]
rgb[30:40, 4:14] = [0.02, 0.01, 0.0]
Image.fromarray(np.round(rgb * 255).astype(np.uint8), "RGB").save(sys.argv[1])
PY
MSYS_NO_PATHCONV=1 docker run --rm -v "$work":/w -v "$PWD":/out debian:trixie bash -c \
  "apt-get update -qq >/dev/null && apt-get install -y -qq libheif-examples libheif-plugin-x265 >/dev/null && \
   heif-enc -q 95 --matrix_coefficients=6 --colour_primaries=12 --transfer_characteristic=13 \
     --full_range_flag=1 -o /out/heic-p3-nclx.heic /w/scene.png"
rm -rf "$work"
