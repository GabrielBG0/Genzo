"""Ground truth for imgio's embedded-preview choice (tests/preview_corpus.rs).

An independent parser finds every embedded baseline or progressive JPEG in
each file (not the one at offset 0), applies the documented rule (color
streams before grayscale ones, which are an iPhone ProRAW DNG's semantic
mattes; then the smallest whose long edge is at least 1280 px, else the
largest), and
decodes that exact stream with libjpeg-turbo (Pillow) to its per-channel
means. Genzo's preview of the file must have the same size and, within
JPEG decoder tolerance, the same means.

  uv run --no-project --with pillow --with numpy python scripts/preview-reference.py FILE...
"""
import io
import sys

import numpy as np
from PIL import Image

PREFERRED_EDGE = 1280


def streams(d):
    out, i = [], 0
    while True:
        i = d.find(b"\xff\xd8\xff", i)
        if i < 0:
            return out
        if i > 0:
            j, size = i + 2, None
            for _ in range(64):
                if j + 4 > len(d) or d[j] != 0xFF:
                    break
                m = d[j + 1]
                if m == 0xFF:
                    j += 1
                    continue
                n = int.from_bytes(d[j + 2:j + 4], "big")
                if m in (0xC0, 0xC1, 0xC2) and d[j + 4] == 8:
                    size = (int.from_bytes(d[j + 7:j + 9], "big"), int.from_bytes(d[j + 5:j + 7], "big"))
                    components = d[j + 9]
                if m == 0xDA:
                    break
                j += 2 + n
            if size and size[0] and size[1]:
                out.append((i, size, components))
        i += 3


def decodable(d, start):
    try:
        img = Image.open(io.BytesIO(d[start:]))
        img.load()
        return np.asarray(img.convert("RGB"))
    except Exception:  # noqa: BLE001 - a false match; try the next
        return None


for path in sys.argv[1:]:
    d = open(path, "rb").read()
    found = streams(d)
    order = []
    for color in (True, False):
        kind = [s for s in found if (s[2] != 1) == color]
        order += sorted((s for s in kind if max(s[1]) >= PREFERRED_EDGE), key=lambda s: s[1][0] * s[1][1])
        order += sorted((s for s in kind if max(s[1]) < PREFERRED_EDGE), key=lambda s: -s[1][0] * s[1][1])
    name = path.replace("\\", "/").split("corpus/")[-1]
    for start, (w, h), _ in order:
        rgb = decodable(d, start)
        if rgb is not None and rgb.shape[:2] == (h, w):
            means = [round(float(rgb[..., c].mean()), 3) for c in range(3)]
            print(f'("{name}", Some(({w}, {h}, {means}))),  // at {start}')
            break
    else:
        print(f'("{name}", None),')
