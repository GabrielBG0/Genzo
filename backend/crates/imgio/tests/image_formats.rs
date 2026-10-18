//! The non-RAW decoders (JPEG, PNG, TIFF) on the fixtures of
//! tests/fixtures/images (make-images.py). Each `<file>.rgba16` holds what
//! the decoder must return: for PNG and TIFF the array that was encoded, so
//! the match must be exact; for JPEG and JPEG-in-TIFF libjpeg-turbo's
//! decode, matched within the recorded difference.

use std::path::PathBuf;

use genzo_imgio::{
    ColorEncoding, DecodeError, DecodeOptions, DecodedImage, FileFormat, Pixels, decode,
};

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/images")
}

fn read(name: &str) -> Vec<u8> {
    std::fs::read(dir().join(name)).unwrap_or_else(|e| panic!("{name}: {e}"))
}

fn open(name: &str) -> DecodedImage {
    decode(&read(name), DecodeOptions::default()).unwrap_or_else(|e| panic!("{name}: {e}"))
}

fn rgba(img: &DecodedImage) -> (&[u16], &ColorEncoding) {
    match &img.pixels {
        Pixels::Rgba16 { data, color } => (data, color),
        other => panic!("expected RGBA16, got {other:?}"),
    }
}

/// The mean difference from the expectation over R, G and B, in 8-bit
/// steps.
fn mean_steps(name: &str, img: &DecodedImage) -> f64 {
    let want: Vec<u16> = read(&format!("{name}.rgba16"))
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .collect();
    let (got, _) = rgba(img);
    // Over R, G and B; alpha is always exact for JPEG.
    let rgb = got.len() / 4 * 3;
    let sum: f64 = got
        .as_chunks::<4>()
        .0
        .iter()
        .zip(want.as_chunks::<4>().0.iter())
        .flat_map(|(a, b)| (0..3).map(move |c| f64::from(a[c].abs_diff(b[c])) / 257.0))
        .sum();
    sum / rgb as f64
}

/// The largest difference from the expectation, in 16-bit codes.
fn max_diff(name: &str, img: &DecodedImage) -> u16 {
    let want: Vec<u16> = read(&format!("{name}.rgba16"))
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .collect();
    let (got, _) = rgba(img);
    assert_eq!(got.len(), want.len(), "{name}: sample count");
    got.iter()
        .zip(&want)
        .map(|(a, b)| a.abs_diff(*b))
        .max()
        .unwrap_or(0)
}

const W: u32 = 37;
const H: u32 = 29;

/// Fixture, the largest difference allowed from libjpeg-turbo (in 8-bit
/// steps), the largest mean difference over R, G and B (in steps), and why.
/// Measured on 2026-10-18 with zune-jpeg 0.5.15 and libjpeg-turbo 3.x
/// (Pillow 13). The IDCTs agree to a step; the rest is chroma upsampling,
/// which libjpeg-turbo does with its "fancy" triangular filter: it grows
/// with subsampling and sits at color edges, so the means stay small.
const JPEG: &[(&str, u16, f64, &str)] = &[
    (
        "jpeg-444.jpg",
        2,
        0.03,
        "no chroma subsampling: IDCT and color conversion rounding",
    ),
    ("jpeg-gray.jpg", 1, 0.01, "luma only: IDCT rounding"),
    ("jpeg-422.jpg", 3, 0.20, "horizontal chroma upsampling"),
    (
        "jpeg-420.jpg",
        4,
        0.33,
        "both-axis chroma upsampling at the color patches' edges",
    ),
    (
        "jpeg-progressive.jpg",
        4,
        0.33,
        "4:2:0; identical to the baseline decode",
    ),
    (
        "jpeg-restarts.jpg",
        4,
        0.33,
        "4:2:0 with a restart every MCU row; identical to the baseline decode, so restarts are honored",
    ),
    ("jpeg-icc-multi.jpg", 4, 0.33, "4:2:0"),
    ("jpeg-exif6-xmp.jpg", 4, 0.33, "4:2:0"),
    ("jpeg-adobe-rgb-exif.jpg", 4, 0.33, "4:2:0"),
];

#[test]
fn jpeg_matches_libjpeg_turbo() {
    for &(name, steps, mean, why) in JPEG {
        let img = open(name);
        assert_eq!(img.format, FileFormat::Jpeg);
        assert_eq!(
            (img.width, img.height, img.source_bits),
            (W, H, 8),
            "{name}"
        );
        assert!(!img.has_alpha, "{name}");
        let (d, m) = (max_diff(name, &img), mean_steps(name, &img));
        println!("{name}: max {} steps, mean {m:.3}", d / 257);
        assert!(
            d <= steps * 257,
            "{name}: differs by {d} codes, allowed {steps} steps ({why})"
        );
        assert!(
            m <= mean,
            "{name}: mean {m:.3} steps, allowed {mean} ({why})"
        );
    }
}

#[test]
fn jpeg_metadata() {
    // An ICC profile split over three APP2 segments comes back whole.
    let img = open("jpeg-icc-multi.jpg");
    assert_eq!(
        *rgba(&img).1,
        ColorEncoding::Icc(read("jpeg-icc-multi.icc"))
    );
    // Orientation from big-endian EXIF behind an XMP APP1; not applied.
    let img = open("jpeg-exif6-xmp.jpg");
    assert_eq!(img.orientation, 6);
    assert_eq!((img.width, img.height), (W, H));
    assert!(
        img.exif.as_deref().is_some_and(|e| e.starts_with(b"MM\0*")),
        "EXIF without its prefix"
    );
    assert_eq!(*rgba(&img).1, ColorEncoding::Srgb);
    // Adobe RGB the DCF way: EXIF ColorSpace 0xFFFF and interop "R03".
    assert_eq!(
        *rgba(&open("jpeg-adobe-rgb-exif.jpg")).1,
        ColorEncoding::AdobeRgb
    );
    // No profile: sRGB. Gray: equal channels.
    let img = open("jpeg-gray.jpg");
    assert_eq!(*rgba(&img).1, ColorEncoding::Srgb);
    assert!(
        rgba(&img)
            .0
            .as_chunks::<4>()
            .0
            .iter()
            .all(|p| p[0] == p[1] && p[1] == p[2])
    );
    assert_eq!(open("jpeg-444.jpg").orientation, 1);
}

/// Wider than zune-jpeg's default limit of 16384 px.
#[test]
fn jpeg_wider_than_16384() {
    let img = open("jpeg-wide.jpg");
    assert_eq!((img.width, img.height), (17000, 8));
}

/// Fixture, has alpha, source bits, expected color, orientation. All match
/// their `.rgba16` exactly: lossless formats, expectation = what was encoded.
const LOSSLESS: &[(&str, bool, u8, Option<&str>, u8)] = &[
    ("png-rgb8.png", false, 8, None, 1),
    ("png-rgba16.png", true, 16, None, 1),
    ("png-gray1.png", false, 1, None, 1),
    ("png-gray2.png", false, 2, None, 1),
    ("png-gray4.png", false, 4, None, 1),
    ("png-graya16.png", true, 16, None, 1),
    ("png-palette-trns.png", true, 8, None, 1),
    ("png-adam7.png", false, 8, None, 1),
    ("png-iccp.png", false, 8, Some("png-iccp.icc"), 1),
    ("png-cicp-p3.png", false, 8, None, 1),
    ("png-exif8.png", false, 8, None, 8),
    ("tiff-rgb8-lzw.tif", false, 8, None, 1),
    ("tiff-rgb16-deflate-predictor.tif", false, 16, None, 1),
    ("tiff-rgb16-planar.tif", false, 16, None, 1),
    ("tiff-gray16-miniswhite.tif", false, 16, None, 1),
    ("tiff-gray8-packbits.tif", false, 8, None, 1),
    ("tiff-rgba16-associated.tif", true, 16, None, 1),
    (
        "tiff-icc-orient3.tif",
        false,
        8,
        Some("tiff-icc-orient3.icc"),
        3,
    ),
    ("tiff-camera-make.tif", false, 8, None, 1),
    // More than one image's worth of data: layers, pages, XMP.
    ("tiff-photoshop-layers.tif", false, 8, None, 1),
    ("tiff-three-pages.tif", false, 8, None, 1),
    ("tiff-big-xmp.tif", false, 8, None, 1),
];

#[test]
fn png_and_tiff_are_exact() {
    for &(name, alpha, bits, icc, orientation) in LOSSLESS {
        let img = open(name);
        assert_eq!((img.width, img.height), (W, H), "{name}");
        assert_eq!(img.has_alpha, alpha, "{name}: alpha");
        assert_eq!(img.source_bits, bits, "{name}: source bits");
        assert_eq!(img.orientation, orientation, "{name}: orientation");
        let want_color = match (icc, name) {
            (Some(file), _) => ColorEncoding::Icc(read(file)),
            (None, "png-cicp-p3.png") => ColorEncoding::DisplayP3,
            _ => ColorEncoding::Srgb,
        };
        assert_eq!(*rgba(&img).1, want_color, "{name}: color");
        assert_eq!(max_diff(name, &img), 0, "{name}: samples");
    }
    // A plain TIFF whose Make names a camera is still read as a TIFF.
    assert_eq!(open("tiff-camera-make.tif").format, FileFormat::Tiff);
}

/// JPEG-in-TIFF (YCbCr): libtiff's decode is the reference.
#[test]
fn tiff_with_jpeg_matches_libtiff() {
    let img = open("tiff-jpeg-ycbcr.tif");
    assert_eq!((img.width, img.height), (W, H));
    let d = max_diff("tiff-jpeg-ycbcr.tif", &img);
    println!("tiff-jpeg-ycbcr.tif: max {} steps", d / 257);
    assert!(d <= 4 * 257, "differs by {d} codes");
}

#[test]
fn unsupported_files_say_why() {
    for (name, want) in [
        ("jpeg-cmyk.jpg", "CMYK JPEG"),
        ("tiff-cmyk.tif", "CMYK"),
        ("tiff-float32.tif", "floating-point"),
        ("tiff-palette8.tif", "RGBPalette"),
    ] {
        match decode(&read(name), DecodeOptions::default()) {
            Err(DecodeError::Unsupported(_, m)) => assert!(m.contains(want), "{name}: {m}"),
            other => panic!("{name}: expected unsupported, got {other:?}"),
        }
    }
}

#[test]
fn damaged_files_never_panic() {
    for name in [
        "jpeg-420.jpg",
        "png-rgba16.png",
        "tiff-rgb16-deflate-predictor.tif",
        "tiff-palette8.tif",
    ] {
        let file = read(name);
        for len in (0..file.len()).step_by(7) {
            let _ = decode(&file[..len], DecodeOptions::default());
        }
        for i in (0..file.len()).step_by(5) {
            let mut d = file.clone();
            d[i] ^= 0xA5;
            let _ = decode(&d, DecodeOptions::default());
        }
    }
}

/// Adobe DNG Converter's 16-bit TIFF renders in corpus/reference, local only
/// (not on the CI list): uncompressed RGB with an ICC profile. The iPhone 16
/// Pro one is 293 MB, past the tiff crate's default 256 MiB buffer limit.
/// Checked against libtiff (tifffile 2026.10.15): every sample identical,
/// recorded as a position-weighted checksum of the RGBA16 output.
#[test]
fn large_16_bit_tiffs_match_libtiff() {
    let corpus = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../corpus/reference");
    for (file, size, want) in [
        (
            "IMG_1361/IMG_1361.tiff",
            (3024, 4032),
            65_383_764_582_603_442u64,
        ),
        (
            "IPhone16proRaw/IPhone16proRaw.tiff",
            (6048, 8064),
            255_491_889_141_832_582,
        ),
    ] {
        let Ok(bytes) = std::fs::read(corpus.join(file)) else {
            println!("skipped: corpus/reference/{file} is missing");
            continue;
        };
        let img = decode(&bytes, DecodeOptions::default()).unwrap();
        assert_eq!(
            (img.width, img.height, img.source_bits),
            (size.0, size.1, 16),
            "{file}"
        );
        let (data, color) = rgba(&img);
        assert!(matches!(color, ColorEncoding::Icc(_)), "{file}");
        let checksum = data.iter().enumerate().fold(0u64, |acc, (i, &v)| {
            acc.wrapping_add(u64::from(v).wrapping_mul((i % 65521) as u64 + 1))
        });
        assert_eq!(checksum, want, "{file}");
    }
}

/// A fuzz find: a 2 kB JPEG whose header claims 32805 x 11805 (387 MP, under
/// the 400 MP limit). zune-jpeg decodes such files leniently, so without a
/// check the decode would allocate 4.3 GB. The file cannot hold that many
/// pixels (at least one bit per 8 x 8 block), so it is refused as damaged.
#[test]
fn a_small_file_cannot_claim_a_huge_image() {
    match decode(
        &read("fuzz-jpeg-387mp-header.jpg"),
        DecodeOptions::default(),
    ) {
        Err(DecodeError::Corrupt { message, .. }) => {
            assert!(message.contains("cannot hold"), "{message}")
        }
        other => panic!("expected damaged, got {other:?}"),
    }
}

/// A fuzz find: a palette PNG whose PLTE chunk is 7 bytes, not whole RGB
/// entries. png 0.17.16 panics expanding it; the decoder refuses it first.
#[test]
fn a_broken_palette_is_damaged_not_a_panic() {
    match decode(&read("fuzz-png-plte-7-bytes.png"), DecodeOptions::default()) {
        Err(DecodeError::Corrupt { message, .. }) => assert!(message.contains("PLTE"), "{message}"),
        other => panic!("expected damaged, got {other:?}"),
    }
}

/// A JPEG file's preview is its EXIF thumbnail only. A big image after its
/// EOI (an Ultra HDR gain map, an MPF image) is not one, however large.
#[test]
fn a_jpeg_preview_is_never_its_gain_map() {
    // jpeg-exif6-xmp.jpg has EXIF but no thumbnail.
    let mut file = read("jpeg-exif6-xmp.jpg");
    file.extend_from_slice(&read("jpeg-wide.jpg"));
    let preview = genzo_imgio::decode_preview(&file).unwrap();
    assert!(
        preview.is_none(),
        "got {:?}",
        preview.map(|p| (p.width, p.height))
    );
}

/// A CR3 sniffed from bytes 4..12 with its CMT1 box type at offset 0.
#[test]
fn crafted_cr3_heads_never_panic() {
    for data in [
        &b"CMT1ftypcrx "[..],
        b"\0\0\0\x08CMT1ftypcrx ",
        b"\0\0\0\x18ftypcrx CMT1",
    ] {
        let _ = genzo_imgio::decode_preview(data);
    }
}
