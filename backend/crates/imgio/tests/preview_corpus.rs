//! Embedded previews on the real files of the corpus (DEV-01, stage 0).
//!
//! The expectations come from scripts/preview-reference.py: an independent
//! parser applies the same rule (the smallest JPEG with a long edge of at
//! least 1280 px, else the largest) and libjpeg-turbo decodes that exact
//! stream. Genzo's preview must have the same size and, within JPEG decoder
//! tolerance, the same per-channel means, which proves the same stream was
//! decoded. Corpus files skip when missing, unless GENZO_REQUIRE_CORPUS=1 and
//! corpus/golden-inputs.txt lists them (CI).
//!   cargo test --release -p genzo-imgio --test preview_corpus -- --nocapture

use std::path::PathBuf;

use genzo_imgio::{Preview, decode_preview};

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

fn means(p: &Preview) -> [f64; 3] {
    let n = (p.rgba.len() / 4) as f64;
    [0, 1, 2].map(|c| {
        p.rgba
            .iter()
            .skip(c)
            .step_by(4)
            .map(|&v| f64::from(v))
            .sum::<f64>()
            / n
    })
}

/// Corpus key, then the preview's size and libjpeg-turbo's per-channel means
/// (None: no preview), and the orientation it must report.
type Reference = (&'static str, Option<(u32, u32, [f64; 3])>, u8);

const PREVIEWS: &[Reference] = &[
    (
        "raw/Apple/iPhone 12 Pro/IMG_1361.DNG",
        Some((2016, 1512, [23.98, 23.98, 23.98])),
        6,
    ),
    (
        "raw/Canon/Canon EOS R5m2/APS-C_CRAW.CR3",
        Some((1620, 1080, [101.665, 113.878, 105.866])),
        1,
    ),
    (
        "raw/Fujifilm/GFX100S/Fujifilm-GFX100S-14bits-compress-4_3.RAF",
        Some((4000, 3000, [105.986, 114.677, 104.586])),
        1,
    ),
    (
        "raw/Fujifilm/X-T5/DSCF0021.RAF",
        Some((4416, 2944, [114.164, 114.514, 121.744])),
        1,
    ),
    ("raw/GoPro/HERO6 Black/GOPR8921.GPR", None, 0),
    ("raw/Hasselblad/H3D/A9391120.3FR", None, 0),
    ("raw/Kodak/DCS760C/86L57188.DCR", None, 0),
    (
        "raw/Leica/Q2/L1000750.DNG",
        Some((1620, 1080, [99.973, 112.272, 122.384])),
        1,
    ),
    (
        "raw/Nikon/D850/Nikon-D850-12bit-compressed.NEF",
        Some((1620, 1080, [54.47, 69.065, 57.35])),
        1,
    ),
    // Nikon High Efficiency: the image cannot be decoded, the preview can.
    (
        "raw/Nikon/Z 8/Nikon_Z8_high_efficiency_low.NEF",
        Some((1620, 1080, [89.357, 102.598, 63.581])),
        1,
    ),
    (
        "raw/Nikon/Z 9/Nikon_-_NIKON_Z_9_-_14bit_compressed_(Lossless).NEF",
        Some((1620, 1080, [95.156, 95.479, 99.38])),
        1,
    ),
    (
        "raw/Nikon/Z 9/Nikon_-_NIKON_Z_9_-_14bit_compressed_(Lossy_High_Efficiency).NEF",
        Some((1620, 1080, [87.461, 88.141, 91.383])),
        1,
    ),
    (
        "raw/Olympus/E-M1MarkII/Olympus_EM1mk2_Standard_20MP.ORF",
        Some((3200, 2400, [79.439, 75.781, 68.167])),
        1,
    ),
    (
        "raw/OM Digital Solutions/OM-3/P5048182.ORF",
        Some((3200, 2400, [107.678, 108.028, 72.635])),
        1,
    ),
    (
        "raw/Panasonic/DC-S5M2/P1126458_mechanical.RW2",
        Some((1920, 1280, [130.196, 128.576, 88.688])),
        1,
    ),
    (
        "raw/Pentax/K10D/IMGP5398.DNG",
        Some((3872, 2592, [76.765, 71.968, 71.605])),
        1,
    ),
    ("raw/Phase One/P65+/CF027310.IIQ", None, 0),
    (
        "raw/Samsung/Galaxy S23 Ultra/20230817_120455.dng",
        Some((3648, 2736, [105.33, 117.792, 74.191])),
        1,
    ),
    (
        "raw/Sony/ILCE-7RM5/7RM5-LosslessCompressedLarge.ARW",
        Some((1616, 1080, [122.075, 133.521, 62.962])),
        1,
    ),
    // Not on the CI list: local only.
    (
        "proraw/iPhone 16 Pro/IPhone16proRaw.DNG",
        Some((2016, 1512, [94.027, 94.027, 94.027])),
        6,
    ),
    (
        "bayer-dng/iphone/Halide_iPhone16Pro_bayer.dng",
        Some((4032, 3024, [130.915, 114.544, 97.609])),
        6,
    ),
];

/// The largest difference allowed from libjpeg-turbo's mean, per channel,
/// in 8-bit steps: decoders differ by up to 4 steps at chroma edges (#39),
/// which moves a whole image's mean by far less.
const MEAN_TOLERANCE: f64 = 0.5;

#[test]
fn previews_match_libjpeg_turbo() {
    for &(key, want, orientation) in PREVIEWS {
        let Some(bytes) = file(key) else { continue };
        let got = decode_preview(&bytes).unwrap_or_else(|e| panic!("{key}: {e}"));
        match (want, got) {
            (None, None) => println!("{key}: no preview"),
            (Some((w, h, ref_means)), Some(p)) => {
                assert_eq!((p.width, p.height), (w, h), "{key}: size");
                // Stored unrotated: landscape pixels, the file's orientation.
                assert_eq!(p.orientation, orientation, "{key}: orientation");
                let m = means(&p);
                println!("{key}: {w}x{h}, means {m:.3?} vs {ref_means:?}");
                for c in 0..3 {
                    assert!(
                        (m[c] - ref_means[c]).abs() <= MEAN_TOLERANCE,
                        "{key}: channel {c} mean {} vs libjpeg-turbo {}",
                        m[c],
                        ref_means[c]
                    );
                }
            }
            (want, got) => panic!(
                "{key}: expected {want:?}, got {:?}",
                got.map(|p| (p.width, p.height))
            ),
        }
    }
}

/// A damaged file still shows a preview that lies in its intact part: the
/// first half of a NEF holds the 1620 px preview, of a CR3 the 1620 px one.
#[test]
fn truncated_files_keep_their_early_previews() {
    for (key, size) in [
        (
            "raw/Nikon/D850/Nikon-D850-12bit-compressed.NEF",
            (1620, 1080),
        ),
        ("raw/Canon/Canon EOS R5m2/APS-C_CRAW.CR3", (1620, 1080)),
    ] {
        let Some(bytes) = file(key) else { continue };
        let half = &bytes[..bytes.len() / 2];
        let p = decode_preview(half)
            .unwrap_or_else(|e| panic!("{key}: {e}"))
            .unwrap_or_else(|| panic!("{key}: no preview in the first half"));
        assert_eq!((p.width, p.height), size, "{key}");
    }
}
