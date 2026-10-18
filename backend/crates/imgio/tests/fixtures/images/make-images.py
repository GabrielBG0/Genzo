"""Writes the non-RAW decoder fixtures of tests/image_formats.rs.

Every fixture is small (37 x 29, odd on both sides, so chroma subsampling and
partial blocks are exercised) and made from a synthetic scene. Beside each
file goes `<file>.rgba16`: interleaved RGBA, 16 bits per sample, little
endian, of what the decoder must return:

- lossless formats (PNG, TIFF without JPEG): the array that was encoded,
  expanded the way the spec defines (palette and tRNS expanded, 1/2/4-bit and
  8-bit samples scaled to 16 bits, MinIsWhite inverted, associated alpha
  un-premultiplied), so the expectation is the truth, not another decoder;
- JPEG and JPEG-in-TIFF: libjpeg-turbo's decode (through Pillow and
  imagecodecs), the independent reference; tests record the largest
  difference they allow per file.

Files that must be refused (CMYK, floating point) get no `.rgba16`.

Regenerate with:
  uv run --no-project --with pillow --with tifffile --with imagecodecs \
    --with numpy --with pypng python make-images.py
"""
import io
import struct
import zlib
from pathlib import Path

import imagecodecs
import numpy as np
import png
import tifffile
from PIL import Image

HERE = Path(__file__).resolve().parent
W, H = 37, 29


def scene(bits=8):
    """RGB gradients with a few saturated patches, at `bits` per sample."""
    top = (1 << bits) - 1
    y, x = np.mgrid[0:H, 0:W]
    r = x / (W - 1)
    g = y / (H - 1)
    b = ((x + y) % 7) / 6
    rgb = np.stack([r, g, b], axis=-1)
    rgb[3:9, 4:12] = [1.0, 0.1, 0.05]
    rgb[15:22, 20:30] = [0.05, 0.2, 1.0]
    return np.round(rgb * top).astype(np.uint16 if bits > 8 else np.uint8)


def alpha(bits=8):
    top = (1 << bits) - 1
    y, x = np.mgrid[0:H, 0:W]
    a = 0.2 + 0.8 * ((x * 3 + y * 5) % 11) / 10
    return np.round(a * top).astype(np.uint16 if bits > 8 else np.uint8)


def to16(a, bits):
    """Samples of `bits` scaled to 16 bits, as the decoder widens them."""
    a = a.astype(np.uint32)
    if bits == 16:
        return a.astype(np.uint16)
    return (a * 65535 // ((1 << bits) - 1)).astype(np.uint16)


def write_expect(name, rgb16, a16=None):
    if a16 is None:
        a16 = np.full(rgb16.shape[:2], 65535, np.uint16)
    rgba = np.dstack([rgb16, a16]).astype("<u2")
    (HERE / f"{name}.rgba16").write_bytes(rgba.tobytes())


def gray_rgb(g16):
    return np.dstack([g16, g16, g16])


# ---------------------------------------------------------------- JPEG ----
def jpeg(name, img, **kw):
    buf = io.BytesIO()
    img.save(buf, "JPEG", quality=kw.pop("quality", 92), **kw)
    data = buf.getvalue()
    (HERE / name).write_bytes(data)
    return data


def jpeg_reference(name, data):
    ref = np.asarray(Image.open(io.BytesIO(data)).convert("RGB"))
    write_expect(name, to16(ref, 8))


rgb8 = scene(8)
img = Image.fromarray(rgb8, "RGB")
for name, sub in [("jpeg-444.jpg", 0), ("jpeg-422.jpg", 1), ("jpeg-420.jpg", 2)]:
    jpeg_reference(name, jpeg(name, img, subsampling=sub))
jpeg_reference("jpeg-gray.jpg", jpeg("jpeg-gray.jpg", img.convert("L")))
jpeg_reference("jpeg-progressive.jpg", jpeg("jpeg-progressive.jpg", img, progressive=True, subsampling=2))
jpeg_reference("jpeg-restarts.jpg", jpeg("jpeg-restarts.jpg", img, restart_marker_rows=1, subsampling=2))

# An ICC profile of 150 kB: split over three APP2 segments. The bytes are
# arbitrary; the decoder must hand back exactly these.
icc = bytes((i * 7 + 3) % 256 for i in range(150_000))
(HERE / "jpeg-icc-multi.icc").write_bytes(icc)
jpeg_reference("jpeg-icc-multi.jpg", jpeg("jpeg-icc-multi.jpg", img, icc_profile=icc))


def tiff_ifd(entries, big_endian=False):
    """A minimal TIFF structure (EXIF payload) with one IFD of SHORT/LONG
    or ASCII entries; nested IFDs given as (tag, [entries])."""
    e = ">" if big_endian else "<"
    out = bytearray(b"MM\0*" if big_endian else b"II*\0") + struct.pack(e + "I", 8)

    def put_ifd(entries):
        at = len(out)
        n = len(entries)
        out.extend(b"\0" * (2 + 12 * n + 4))
        struct.pack_into(e + "H", out, at, n)
        for i, (tag, val) in enumerate(sorted(entries, key=lambda t: t[0])):
            p = at + 2 + 12 * i
            if isinstance(val, list):  # a sub-IFD
                sub = put_ifd(val)
                struct.pack_into(e + "HHII", out, p, tag, 4, 1, sub)
            elif isinstance(val, bytes):  # ASCII, at most 4 bytes inline
                struct.pack_into(e + "HHI", out, p, tag, 2, len(val))
                out[p + 8:p + 8 + len(val)] = val
            else:
                struct.pack_into(e + "HHI", out, p, tag, 3, 1)
                struct.pack_into(e + "H", out, p + 8, val)
        return at

    put_ifd(entries)
    return bytes(out)


def with_app1_before(data, payload):
    """Inserts an APP1 segment right after SOI."""
    seg = b"\xff\xe1" + struct.pack(">H", len(payload) + 2) + payload
    return data[:2] + seg + data[2:]


# Orientation 6 (rotate 90 CW) in big-endian EXIF, with an XMP APP1 before it.
exif6 = b"Exif\0\0" + tiff_ifd([(274, 6)], big_endian=True)
data = jpeg("jpeg-exif6-xmp.jpg", img, exif=exif6)
xmp = b"http://ns.adobe.com/xap/1.0/\0<x:xmpmeta xmlns:x='adobe:ns:meta/'/>"
data = with_app1_before(data, xmp)
(HERE / "jpeg-exif6-xmp.jpg").write_bytes(data)
jpeg_reference("jpeg-exif6-xmp.jpg", data)

# Adobe RGB by the DCF rule: EXIF ColorSpace 0xFFFF (uncalibrated) and the
# Interoperability index "R03", no ICC profile.
exif_adobe = b"Exif\0\0" + tiff_ifd([(34665, [(0xA001, 0xFFFF), (0xA005, [(0x0001, b"R03\0")])])])
jpeg_reference("jpeg-adobe-rgb-exif.jpg", jpeg("jpeg-adobe-rgb-exif.jpg", img, exif=exif_adobe))

# Wider than zune-jpeg's default 16384 px limit.
wide = Image.fromarray(np.tile(rgb8[:8, :1], (1, 17000, 1)), "RGB")
# Only its size is checked, so no expectation file (it would be 1 MB).
jpeg("jpeg-wide.jpg", wide, quality=50)

# CMYK: refused, no expectation.
jpeg("jpeg-cmyk.jpg", img.convert("CMYK"))


# ----------------------------------------------------------------- PNG ----
def png_chunk(kind, body):
    return struct.pack(">I", len(body)) + kind + body + struct.pack(">I", zlib.crc32(kind + body) & 0xFFFFFFFF)


def png_write(name, rows, chunks=(), **kw):
    buf = io.BytesIO()
    png.Writer(W, H, **kw).write(buf, rows)
    data = buf.getvalue()
    if chunks:  # after IHDR (8 + 25 bytes)
        data = data[:33] + b"".join(png_chunk(k, b) for k, b in chunks) + data[33:]
    (HERE / name).write_bytes(data)


rgb16 = scene(16)
a8, a16 = alpha(8), alpha(16)

png_write("png-rgb8.png", rgb8.reshape(H, -1), greyscale=False, bitdepth=8)
write_expect("png-rgb8.png", to16(rgb8, 8))

png_write("png-rgba16.png", np.dstack([rgb16, a16]).reshape(H, -1), greyscale=False, alpha=True, bitdepth=16)
write_expect("png-rgba16.png", rgb16, a16)

g = rgb8[..., 1]
for bits in (1, 2, 4):
    gb = (g.astype(np.uint32) * ((1 << bits) - 1) // 255).astype(np.uint8)
    png_write(f"png-gray{bits}.png", gb, greyscale=True, bitdepth=bits)
    write_expect(f"png-gray{bits}.png", gray_rgb(to16(gb, bits)))

g16 = rgb16[..., 0]
png_write("png-graya16.png", np.dstack([g16, a16]).reshape(H, -1), greyscale=True, alpha=True, bitdepth=16)
write_expect("png-graya16.png", gray_rgb(g16), a16)

palette = [(0, 0, 0, 255), (255, 0, 0, 128), (0, 255, 0, 0), (20, 40, 250, 255), (255, 255, 255, 200)]
idx = ((np.mgrid[0:H, 0:W].sum(axis=0)) % len(palette)).astype(np.uint8)
png_write("png-palette-trns.png", idx, palette=palette, bitdepth=8)
pal = np.array(palette, np.uint16)
write_expect("png-palette-trns.png", to16(pal[idx][..., :3], 8), to16(pal[idx][..., 3], 8))

png_write("png-adam7.png", rgb8.reshape(H, -1), greyscale=False, bitdepth=8, interlace=True)
write_expect("png-adam7.png", to16(rgb8, 8))

# Color chunks: an ICC profile (iCCP), cICP (Display P3), and eXIf with
# orientation 8. iCCP's profile is zlib-compressed after a name.
png_icc = bytes((i * 13 + 1) % 256 for i in range(3000))
(HERE / "png-iccp.icc").write_bytes(png_icc)
png_write("png-iccp.png", rgb8.reshape(H, -1), [(b"iCCP", b"test\0\0" + zlib.compress(png_icc))], greyscale=False, bitdepth=8)
write_expect("png-iccp.png", to16(rgb8, 8))
png_write("png-cicp-p3.png", rgb8.reshape(H, -1), [(b"cICP", bytes([12, 13, 0, 1]))], greyscale=False, bitdepth=8)
write_expect("png-cicp-p3.png", to16(rgb8, 8))
png_write("png-exif8.png", rgb8.reshape(H, -1), [(b"eXIf", tiff_ifd([(274, 8)]))], greyscale=False, bitdepth=8)
write_expect("png-exif8.png", to16(rgb8, 8))


# ---------------------------------------------------------------- TIFF ----
def tif(name, data, **kw):
    tifffile.imwrite(HERE / name, data, metadata=None, **kw)


tif("tiff-rgb8-lzw.tif", rgb8, photometric="rgb", compression="lzw")
write_expect("tiff-rgb8-lzw.tif", to16(rgb8, 8))

tif("tiff-rgb16-deflate-predictor.tif", rgb16, photometric="rgb", compression="zlib", predictor=2)
write_expect("tiff-rgb16-deflate-predictor.tif", rgb16)

tif("tiff-rgb16-planar.tif", np.moveaxis(rgb16, -1, 0), photometric="rgb", planarconfig="separate")
write_expect("tiff-rgb16-planar.tif", rgb16)

gw = g16
tif("tiff-gray16-miniswhite.tif", gw, photometric="miniswhite")
write_expect("tiff-gray16-miniswhite.tif", gray_rgb(65535 - gw))

tif("tiff-gray8-packbits.tif", g, photometric="minisblack", compression="packbits")
write_expect("tiff-gray8-packbits.tif", gray_rgb(to16(g, 8)))

# Palette: TIFF ColorMap holds 16-bit values, all reds, then greens, blues.
cmap = np.zeros((3, 256), np.uint16)
for i, (r, gg, b, _) in enumerate(palette):
    cmap[:, i] = [r * 257, gg * 257, b * 257]
tif("tiff-palette8.tif", idx, photometric="palette", colormap=cmap)
write_expect("tiff-palette8.tif", np.moveaxis(cmap[:, idx], 0, -1))

# Associated (premultiplied) alpha, 16-bit; expected straight alpha.
pre = (rgb16.astype(np.uint64) * a16[..., None] // 65535).astype(np.uint16)
tif("tiff-rgba16-associated.tif", np.dstack([pre, a16]), photometric="rgb", extrasamples=[1])
unpre = np.where(a16[..., None] > 0, (pre.astype(np.uint64) * 65535 + a16[..., None] // 2) // np.maximum(a16[..., None], 1), 0)
write_expect("tiff-rgba16-associated.tif", np.minimum(unpre, 65535).astype(np.uint16), a16)

# ICC (tag 34675) and orientation 3.
tif_icc = bytes((i * 5 + 9) % 256 for i in range(2000))
(HERE / "tiff-icc-orient3.icc").write_bytes(tif_icc)
tif("tiff-icc-orient3.tif", rgb8, photometric="rgb", iccprofile=tif_icc, extratags=[(274, "H", 1, 3, False)])
write_expect("tiff-icc-orient3.tif", to16(rgb8, 8))

# A plain TIFF whose Make and Model name a camera: must still decode as TIFF.
tif("tiff-camera-make.tif", rgb8, photometric="rgb",
    extratags=[(271, "s", 0, "NIKON CORPORATION", False), (272, "s", 0, "NIKON D850", False)])
write_expect("tiff-camera-make.tif", to16(rgb8, 8))

# Plain TIFFs that carry much more than one image: 200 kB of Photoshop
# layer data (tag 37724), three pages, 100 kB of XMP. They decode as IFD0.
blob = bytes((i * 31 + 7) % 256 for i in range(200_000))
tif("tiff-photoshop-layers.tif", rgb8, photometric="rgb", compression="lzw",
    extratags=[(37724, "B", len(blob), blob, False)])
write_expect("tiff-photoshop-layers.tif", to16(rgb8, 8))
with tifffile.TiffWriter(HERE / "tiff-three-pages.tif") as w:
    for k in range(3):
        w.write(np.roll(rgb8, k * 5, axis=1), photometric="rgb", metadata=None)
write_expect("tiff-three-pages.tif", to16(rgb8, 8))
xmp_big = b"<x:xmpmeta xmlns:x='adobe:ns:meta/'>" + b" " * 100_000 + b"</x:xmpmeta>"
tif("tiff-big-xmp.tif", rgb8, photometric="rgb", extratags=[(700, "B", len(xmp_big), xmp_big, False)])
write_expect("tiff-big-xmp.tif", to16(rgb8, 8))

# JPEG-in-TIFF (YCbCr, compression 7): libtiff's reading is the reference.
tif("tiff-jpeg-ycbcr.tif", rgb8, photometric="ycbcr", compression="jpeg", compressionargs={"level": 90})
write_expect("tiff-jpeg-ycbcr.tif", to16(tifffile.imread(HERE / "tiff-jpeg-ycbcr.tif"), 8))

# Refused: CMYK and floating point.
tif("tiff-cmyk.tif", np.dstack([rgb8, rgb8[..., :1]]), photometric="separated")
tif("tiff-float32.tif", rgb16.astype(np.float32) / 65535, photometric="rgb")

print("written to", HERE)
