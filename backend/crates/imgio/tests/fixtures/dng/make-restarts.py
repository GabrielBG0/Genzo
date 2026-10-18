"""Writes lj12_restarts.ljpg: the 40x27 image of make-tiles.py as one 12-bit
lossless JPEG stream (SOF3, 3 components, predictor 1) with a restart
interval of 4 rows (DRI 160), as Samsung writes its DNG strips (every 16
rows). The last interval holds the 3 rows left.

libjpeg-turbo cannot write restarts in lossless mode through imagecodecs, so
this script encodes the stream itself (ITU T.81, annex H), with one fixed
Huffman table of 5-bit codes. libjpeg-turbo decoding it back to the image
checks the encoder.
Regenerate with: uv run --no-project --with imagecodecs --with numpy python make-restarts.py
"""
from pathlib import Path

import imagecodecs
import numpy as np

W, H, ROWS, BITS = 40, 27, 4, 12
HERE = Path(__file__).resolve().parent


def sample(x, y, c):
    return (37 * x + 91 * y + 13 * c) % 1100


class Bits:
    def __init__(self):
        self.out, self.acc, self.n = bytearray(), 0, 0

    def put(self, value, n):
        for i in reversed(range(n)):
            self.acc = (self.acc << 1) | ((value >> i) & 1)
            self.n += 1
            if self.n == 8:
                self.out.append(self.acc)
                if self.acc == 0xFF:
                    self.out.append(0)  # byte stuffing
                self.acc, self.n = 0, 0

    def flush(self):
        """Pads the last byte with 1 bits, as a restart or EOI requires."""
        while self.n:
            self.put(1, 1)
        return bytes(self.out)


def interval(image, y0, rows):
    """Entropy data of rows y0..y0+rows, prediction restarted at y0."""
    bits = Bits()
    for y in range(y0, y0 + rows):
        for x in range(W):
            for c in range(3):
                if y == y0 and x == 0:
                    pred = 1 << (BITS - 1)
                elif y == y0:
                    pred = int(image[y, x - 1, c])
                elif x == 0:
                    pred = int(image[y - 1, x, c])
                else:
                    pred = int(image[y, x - 1, c])  # predictor 1: Ra
                diff = (int(image[y, x, c]) - pred) % 65536
                if diff >= 32768:
                    diff -= 65536
                ssss = 16 if diff == -32768 else abs(diff).bit_length()
                bits.put(ssss, 5)  # code of category ssss is ssss, 5 bits
                if 0 < ssss < 16:
                    bits.put(diff if diff > 0 else diff - 1 + (1 << ssss), ssss)
    return bits.flush()


def segment(marker, payload):
    return bytes([0xFF, marker]) + (len(payload) + 2).to_bytes(2, "big") + payload


image = np.array(
    [[[sample(x, y, c) for c in range(3)] for x in range(W)] for y in range(H)],
    dtype=np.uint16,
)
out = bytearray(b"\xff\xd8")
out += segment(0xC3, bytes([BITS]) + H.to_bytes(2, "big") + W.to_bytes(2, "big") + b"\x03"
               + b"".join(bytes([c, 0x11, 0]) for c in range(3)))
# Table 0, DC class: 17 codes of 5 bits for categories 0..16.
counts = [0] * 16
counts[4] = 17
out += segment(0xC4, b"\x00" + bytes(counts) + bytes(range(17)))
out += segment(0xDD, (ROWS * W).to_bytes(2, "big"))
out += segment(0xDA, b"\x03" + b"".join(bytes([c, 0]) for c in range(3)) + b"\x01\x00\x00")
for n, y0 in enumerate(range(0, H, ROWS)):
    if n:
        out += bytes([0xFF, 0xD0 + (n - 1) % 8])
    out += interval(image, y0, min(ROWS, H - y0))
out += b"\xff\xd9"

assert (imagecodecs.jpeg8_decode(bytes(out)) == image).all(), "libjpeg-turbo disagrees"
(HERE / "lj12_restarts.ljpg").write_bytes(bytes(out))
print(f"{len(out)} bytes, {(H + ROWS - 1) // ROWS} intervals")
