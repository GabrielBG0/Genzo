//! The CFA path (camera RAW and CFA DNG through rawler) on the real files of
//! the corpus, against two independent readings:
//!
//! - the samples, against LibRaw 0.22.1 (rawpy 0.27.1, scripts/
//!   cfa-reference.py): sensor size, the color filter pattern at the sensor
//!   origin, the means per color and a position-weighted checksum of every
//!   sample, recorded from LibRaw and identical in Genzo's decode;
//! - the color metadata, against Adobe DNG Converter's DNGs of the same files
//!   (corpus/reference/), read with this crate's own DNG tag reader.
//!
//! Every corpus file is CC0 (raw.pixls.us). A missing file is skipped, unless
//! GENZO_REQUIRE_CORPUS=1 and corpus/golden-inputs.txt lists it (CI).
//!   cargo test --release -p genzo-imgio --features raw --test cfa_corpus -- --nocapture
#![cfg(feature = "raw")]

use std::path::PathBuf;

use genzo_imgio::raw::{AsShot, CfaPattern, ColorMatrix};
use genzo_imgio::{DecodeError, DecodeOptions, DecodedImage, FileFormat, Pixels, decode, dng};

fn corpus_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../corpus")
}

/// A corpus file's bytes, or None to skip.
fn file(key: &str) -> Option<Vec<u8>> {
    let path = corpus_dir().join(key);
    if path.exists() {
        return Some(std::fs::read(&path).unwrap());
    }
    let listed = std::fs::read_to_string(corpus_dir().join("golden-inputs.txt"))
        .unwrap_or_default()
        .lines()
        .any(|l| l.trim() == key);
    if listed && std::env::var("GENZO_REQUIRE_CORPUS").as_deref() == Ok("1") {
        panic!("corpus file {key} is missing: python scripts/fetch-raw-corpus.py --only \"{key}\"");
    }
    println!("skipped: {key} is missing");
    None
}

fn mosaic(img: &DecodedImage) -> (&CfaPattern, &[u16]) {
    match &img.pixels {
        Pixels::Cfa { pattern, data, .. } => (pattern, data),
        other => panic!("expected a CFA mosaic, got {other:?}"),
    }
}

/// The pattern as it repeats from the sensor origin, 6 × 6, rows joined by
/// spaces: the form scripts/cfa-reference.py prints.
fn pattern_rows(p: &CfaPattern) -> String {
    (0..6)
        .map(|y| {
            (0..6)
                .map(|x| ["R", "G", "B"][usize::from(p.color_at(y, x))])
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Rounded mean per color, as the reference script computes it.
fn means(p: &CfaPattern, data: &[u16], width: usize) -> [u64; 3] {
    let (mut sums, mut counts) = ([0u64; 3], [0u64; 3]);
    for (i, &v) in data.iter().enumerate() {
        let c = usize::from(p.color_at(i / width, i % width));
        sums[c] += u64::from(v);
        counts[c] += 1;
    }
    [0, 1, 2].map(|c| (sums[c] + counts[c] / 2) / counts[c].max(1))
}

fn checksum(data: &[u16]) -> u64 {
    data.iter().enumerate().fold(0u64, |acc, (i, &v)| {
        acc.wrapping_add(u64::from(v).wrapping_mul((i % 65521) as u64 + 1))
    })
}

const BAYER_RGGB: &str = "RGRGRG GBGBGB RGRGRG GBGBGB RGRGRG GBGBGB";

/// LibRaw's reading of each file: corpus key, sensor size, pattern, means
/// per color, checksum. Genzo's decode must equal it exactly. The NEF and DCR
/// rows hold only because Genzo's rawler is patched to apply curves without
/// dithering (backend/vendor/README.md).
/// Corpus key, sensor size, pattern, means per color, checksum.
type Reference = (&'static str, (u32, u32), &'static str, [u64; 3], u64);

const LIBRAW: &[Reference] = &[
    (
        "raw/Canon/Canon EOS R5m2/APS-C_CRAW.CR3",
        (5376, 3574),
        BAYER_RGGB,
        [1188, 1846, 1217],
        959_095_280_512_638,
    ),
    (
        "raw/Fujifilm/GFX100S/Fujifilm-GFX100S-14bits-compress-4_3.RAF",
        (11808, 8754),
        BAYER_RGGB,
        [1535, 2885, 1752],
        7_667_779_135_913_683,
    ),
    (
        "raw/Fujifilm/X-T5/DSCF0021.RAF",
        (7872, 5196),
        "GGRGGB GGBGGR BRGRBG GGBGGR GGRGGB RBGBRG",
        [1909, 3074, 2458],
        3_589_393_870_684_816,
    ),
    (
        "raw/Hasselblad/H3D/A9391120.3FR",
        (7262, 5456),
        BAYER_RGGB,
        [4708, 10685, 8125],
        11_091_663_622_778_457,
    ),
    (
        "raw/Kodak/DCS760C/86L57188.DCR",
        (3040, 2016),
        "GRGRGR BGBGBG GRGRGR BGBGBG GRGRGR BGBGBG",
        [287, 323, 190],
        56_149_572_759_110,
    ),
    (
        "raw/Leica/Q2/L1000750.DNG",
        (8424, 5632),
        "GBGBGB RGRGRG GBGBGB RGRGRG GBGBGB RGRGRG",
        [1293, 2313, 2066],
        3_101_946_293_838_347,
    ),
    (
        "raw/Nikon/D850/Nikon-D850-12bit-compressed.NEF",
        (8288, 5520),
        BAYER_RGGB,
        [178, 290, 253],
        378_362_896_438_797,
    ),
    (
        "raw/Nikon/Z 9/Nikon_-_NIKON_Z_9_-_14bit_compressed_(Lossless).NEF",
        (8280, 5520),
        BAYER_RGGB,
        [1505, 1834, 1554],
        2_517_501_360_593_064,
    ),
    (
        "raw/Olympus/E-M1MarkII/Olympus_EM1mk2_Standard_20MP.ORF",
        (5240, 3912),
        BAYER_RGGB,
        [356, 375, 284],
        233_210_336_059_023,
    ),
    (
        "raw/OM Digital Solutions/OM-3/P5048182.ORF",
        (5220, 3912),
        BAYER_RGGB,
        [397, 556, 370],
        314_085_407_513_870,
    ),
    (
        "raw/Panasonic/DC-S5M2/P1126458_mechanical.RW2",
        (6008, 4008),
        BAYER_RGGB,
        [1863, 3236, 1940],
        2_024_531_814_872_665,
    ),
    (
        "raw/Pentax/K10D/IMGP5398.DNG",
        (3936, 2624),
        BAYER_RGGB,
        [211, 293, 271],
        89_997_529_018_460,
    ),
    (
        "raw/Sony/ILCE-7RM5/7RM5-LosslessCompressedLarge.ARW",
        (9728, 6656),
        BAYER_RGGB,
        [1212, 2500, 1092],
        3_872_728_192_615_017,
    ),
    // Not CC0-listed (a Halide test file), so local only: a CFA DNG with
    // lossless JPEG tiles and its pattern 96 columns into the sensor.
    (
        "bayer-dng/iphone/Halide_iPhone16Pro_bayer.dng",
        (4224, 3024),
        BAYER_RGGB,
        [743, 852, 663],
        325_217_345_301_164,
    ),
];

#[test]
fn samples_match_libraw() {
    for &(key, size, pattern, want_means, want_checksum) in LIBRAW {
        let Some(bytes) = file(key) else { continue };
        println!("{key}");
        let img = decode(&bytes, DecodeOptions::default()).unwrap();
        assert_eq!((img.width, img.height), size, "{key}");
        let (p, data) = mosaic(&img);
        assert_eq!(pattern_rows(p), pattern, "{key}");
        assert_eq!(means(p, data, img.width as usize), want_means, "{key}");
        assert_eq!(checksum(data), want_checksum, "{key}");
    }
}

/// Phase One IIQ: rawler applies the camera's sensor calibration (black,
/// defect columns, flat field) inside its decoder, while LibRaw's raw image
/// is the data before it, so the samples cannot be compared. Only the shape
/// is checked; the values are not independently verified.
#[test]
fn phase_one_iiq_decodes() {
    let Some(bytes) = file("raw/Phase One/P65+/CF027310.IIQ") else {
        return;
    };
    let img = decode(&bytes, DecodeOptions::default()).unwrap();
    assert_eq!((img.width, img.height), (9036, 6784));
    let (p, _) = mosaic(&img);
    assert_eq!(pattern_rows(p), BAYER_RGGB);
}

/// Adobe DNG Converter's DNG of the same file, its matrices and neutral.
fn adobe(stem: &str) -> Option<genzo_imgio::RawMetadata> {
    let path = corpus_dir()
        .join("reference")
        .join(stem)
        .join(format!("{stem}.dng"));
    let bytes = std::fs::read(path).ok()?;
    Some(dng::read_metadata(&bytes).unwrap().0)
}

fn close(a: &[f64], b: &[f64], tol: f64) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() <= tol)
}

/// The color metadata mapped from rawler, against Adobe's reading of the
/// same file: the matrix for each calibration illuminant to 1e-4, the as-shot
/// neutral to 1e-3, the orientation exactly.
#[test]
fn color_metadata_matches_adobe() {
    let files = [
        ("raw/Canon/Canon EOS R5m2/APS-C_CRAW.CR3", "APS-C_CRAW"),
        ("raw/Fujifilm/X-T5/DSCF0021.RAF", "DSCF0021"),
        (
            "raw/Nikon/D850/Nikon-D850-12bit-compressed.NEF",
            "Nikon-D850-12bit-compressed",
        ),
        (
            "raw/Nikon/Z 9/Nikon_-_NIKON_Z_9_-_14bit_compressed_(Lossless).NEF",
            "Nikon_-_NIKON_Z_9_-_14bit_compressed_(Lossless)",
        ),
        (
            "raw/Panasonic/DC-S5M2/P1126458_mechanical.RW2",
            "P1126458_mechanical",
        ),
        (
            "raw/Sony/ILCE-7RM5/7RM5-LosslessCompressedLarge.ARW",
            "7RM5-LosslessCompressedLarge",
        ),
    ];
    for (key, stem) in files {
        let (Some(bytes), Some(want)) = (file(key), adobe(stem)) else {
            continue;
        };
        println!("{key}");
        let img = decode(&bytes, DecodeOptions::default()).unwrap();
        let Pixels::Cfa { raw, .. } = &img.pixels else {
            panic!("{key}: not a CFA image")
        };
        let ours = [&raw.color.color_matrix_1, &raw.color.color_matrix_2];
        let theirs = [&want.color.color_matrix_1, &want.color.color_matrix_2];
        for m in theirs.into_iter().flatten() {
            let found = ours
                .iter()
                .copied()
                .flatten()
                .find(|o: &&ColorMatrix| o.illuminant == m.illuminant)
                .unwrap_or_else(|| panic!("{key}: no matrix for illuminant {:?}", m.illuminant));
            assert!(
                close(&found.matrix, &m.matrix, 1e-4),
                "{key}: illuminant {:?}: {:?} vs Adobe {:?}",
                m.illuminant,
                found.matrix,
                m.matrix
            );
        }
        match (raw.color.as_shot, want.color.as_shot) {
            (Some(AsShot::Neutral(a)), Some(AsShot::Neutral(b))) => {
                assert!(close(&a, &b, 1e-3), "{key}: neutral {a:?} vs Adobe {b:?}")
            }
            other => panic!("{key}: as shot {other:?}"),
        }
        assert_eq!(img.orientation, 1, "{key}");
    }
}

/// Files Genzo cannot decode say why, and as unsupported, not damaged.
#[test]
fn unsupported_files_say_why() {
    let cases = [
        (
            "raw/Nikon/Z 8/Nikon_Z8_high_efficiency_low.NEF",
            "Nikon High Efficiency RAW is not supported yet",
        ),
        (
            "raw/Nikon/Z 9/Nikon_-_NIKON_Z_9_-_14bit_compressed_(Lossy_High_Efficiency).NEF",
            "Nikon High Efficiency RAW is not supported yet",
        ),
        (
            "raw/GoPro/HERO6 Black/GOPR8921.GPR",
            "GoPro GPR (VC-5 compression) is not supported",
        ),
    ];
    for (key, why) in cases {
        let Some(bytes) = file(key) else { continue };
        match decode(&bytes, DecodeOptions::default()) {
            Err(DecodeError::Unsupported(FileFormat::Tiff, m)) => assert_eq!(m, why, "{key}"),
            other => panic!("{key}: expected unsupported, got {other:?}"),
        }
    }
}

/// Every file in the corpus is recognized: no UnknownFormat.
#[test]
fn every_corpus_file_is_recognized() {
    fn walk(dir: &std::path::Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for e in entries {
            let path = e.unwrap().path();
            if path.is_dir() {
                walk(&path, out);
            } else if path
                .extension()
                .is_some_and(|x| !["txt", "json", "md", "tiff"].contains(&x.to_str().unwrap_or("")))
            {
                out.push(path);
            }
        }
    }
    let mut files = Vec::new();
    walk(&corpus_dir().join("raw"), &mut files);
    for path in files {
        let head: Vec<u8> = std::fs::read(&path).unwrap().into_iter().take(64).collect();
        assert!(
            genzo_imgio::sniff(&head).is_some(),
            "{} is not recognized",
            path.display()
        );
    }
}
