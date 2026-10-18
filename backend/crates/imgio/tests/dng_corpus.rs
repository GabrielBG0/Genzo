//! The linear DNG reader, and `imgio::decode` over it, on the real DNG files
//! of the corpus. The
//! corpus is gitignored (personal photos), so each check skips when its file
//! is missing; run them where the corpus is, in release for speed:
//!   cargo test --release -p genzo-imgio --test dng_corpus -- --nocapture
#![cfg(feature = "dng")]

use std::path::PathBuf;

use genzo_imgio::dng::{self, COMPRESSION_JPEG_XL, DngError};
use genzo_imgio::raw::{AsShot, OpcodeKind};
use genzo_imgio::{DecodeOptions, Pixels};

/// Every DNG in corpus/<folder>/, if the corpus has any.
fn corpus_files(folder: &str) -> Vec<(PathBuf, Vec<u8>)> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../corpus")
        .join(folder);
    // A missing folder means the corpus is not here: skip. Any other error
    // is a real problem and fails the test.
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            println!("skipped: no corpus/{folder}");
            return Vec::new();
        }
        Err(e) => panic!("{}: {e}", dir.display()),
    };
    let mut paths: Vec<PathBuf> = entries
        .map(|e| {
            e.unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
                .path()
        })
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("dng")))
        .collect();
    paths.sort();
    paths
        .into_iter()
        .map(|p| {
            let data = std::fs::read(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
            (p, data)
        })
        .collect()
}

/// Facts recorded for known corpus files, told apart by size: the clamp count
/// and the rounded per-channel means. Means are checked within 1, since the
/// tiles are lossy JPEG XL and decoders may round the last step differently.
const KNOWN_16_PRO: &[(usize, u64, [f64; 3])] = &[
    // IMG_6884.DNG (sha256 97f76203...b5b378), the file of the 0.C
    // prototype. Against libjxl: 0.017 % of samples differ, almost all by one
    // table step; 15 saturated pixels by 2 to 5 steps (lossy XYB rounding).
    (28_591_661, 3, [3901.0, 4229.0, 4705.0]),
    // IMG_6889.DNG (iOS 27.2), checked against libjxl: 0.018 % of samples
    // differ, each by one LinearizationTable step.
    (20_306_343, 7, [6789.0, 6669.0, 6715.0]),
];

/// The iPhone 16 Pro 48 MP ProRAW with JPEG XL tiles.
#[test]
fn iphone_16_pro_jpeg_xl() {
    for (path, data) in corpus_files("proraw/iPhone 16 Pro") {
        println!("{}", path.display());
        check_16_pro(&data);
    }
}

fn check_16_pro(data: &[u8]) {
    let started = std::time::Instant::now();
    let img = dng::decode_linear(data).unwrap();
    println!("decoded in {:?}", started.elapsed());
    assert_eq!(
        (img.width, img.height, img.samples_per_pixel),
        (8064, 6048, 3)
    );
    assert_eq!(img.compression, COMPRESSION_JPEG_XL);
    assert_eq!(img.linearization_entries, 1024);
    assert!(img.meta.gain_table_map.is_some());
    assert_eq!(img.meta.color.as_shot, Some(AsShot::Neutral([1.0; 3])));
    assert!(img.meta.color.baseline_exposure != 0.0);
    let color = &img.meta.color;
    assert!(color.color_matrix_1.is_some() && color.color_matrix_2.is_some());
    // A handful of samples past the table is normal; many would mean the
    // samples were read at the wrong scale.
    assert!(img.clamped < 1000, "{} samples clamped", img.clamped);

    let spp = usize::from(img.samples_per_pixel);
    let n = img.pixels.len() / spp;
    let white = f64::from(img.meta.levels.white[0]);
    let means: Vec<f64> = (0..spp)
        .map(|c| {
            img.pixels
                .iter()
                .skip(c)
                .step_by(spp)
                .map(|&v| u64::from(v))
                .sum::<u64>() as f64
                / n as f64
        })
        .collect();
    println!(
        "clamped {}, baseline exposure {:?}, means {means:.0?}",
        img.clamped, img.meta.color.baseline_exposure
    );
    for &m in &means {
        assert!(m > 0.0 && m < white / 2.0, "mean {m} of white {white}");
    }
    if let Some((_, clamped, want)) = KNOWN_16_PRO.iter().find(|(len, ..)| *len == data.len()) {
        assert_eq!(img.clamped, *clamped);
        for (m, w) in means.iter().zip(want) {
            assert!((m - w).abs() <= 1.0, "mean {m}, recorded {w}");
        }
    } else {
        println!("not a recorded file: only the general checks ran");
    }
}

/// The iPhone 13 Pro Max ProRAW has lossless JPEG tiles, decoded with
/// rawler's decompressor under the `raw` feature. Its recorded values were
/// checked against libjpeg-turbo: every sample identical.
#[test]
fn iphone_13_pro_max_lossless_jpeg() {
    for (path, data) in corpus_files("proraw/iPhone 13 Pro Max") {
        println!("{}", path.display());
        let result = dng::decode_linear(&data);
        if !cfg!(feature = "raw") {
            match result {
                Err(DngError::NotBuiltIn(_, "raw")) => {}
                other => panic!("expected Unsupported, got {other:?}"),
            }
            continue;
        }
        let img = result.unwrap();
        assert_eq!(
            (img.width, img.height, img.samples_per_pixel),
            (4032, 3024, 3)
        );
        assert_eq!(img.compression, dng::COMPRESSION_LOSSLESS_JPEG);
        assert_eq!(img.linearization_entries, 4096);
        assert!(img.meta.gain_table_map.is_some());
        let exposure = img.meta.color.baseline_exposure;
        assert!(
            (exposure - 2.50).abs() < 0.01,
            "BaselineExposure {exposure}"
        );
        // IMG_8104.dng; lossless, so the means are exact.
        if data.len() == 36_083_682 {
            assert_eq!(img.clamped, 0);
            let n = (img.pixels.len() / 3) as u64;
            let means: Vec<u64> = (0..3)
                .map(|c| {
                    let sum: u64 = img
                        .pixels
                        .iter()
                        .skip(c)
                        .step_by(3)
                        .map(|&v| u64::from(v))
                        .sum();
                    (sum + n / 2) / n
                })
                .collect();
            assert_eq!(means, [1288, 1274, 1268]);
        }
    }
}

/// The iPhone 12 Pro ProRAW from raw.pixls.us (fetched by
/// scripts/fetch-raw-corpus.py), lossless JPEG like the 13 Pro Max. Unlike
/// the newer files it is not white balanced in the camera: AsShotNeutral is
/// a real neutral. Checked against libjpeg (imagecodecs) plus the table:
/// every sample identical.
#[test]
fn iphone_12_pro_lossless_jpeg() {
    for (path, data) in corpus_files("raw/Apple/iPhone 12 Pro") {
        println!("{}", path.display());
        let result = dng::decode_linear(&data);
        if !cfg!(feature = "raw") {
            assert!(
                matches!(result, Err(DngError::NotBuiltIn(_, "raw"))),
                "{result:?}"
            );
            continue;
        }
        let img = result.unwrap();
        assert_eq!(
            (img.width, img.height, img.samples_per_pixel),
            (4032, 3024, 3)
        );
        assert_eq!(img.compression, dng::COMPRESSION_LOSSLESS_JPEG);
        assert_eq!(img.linearization_entries, 4096);
        let Some(AsShot::Neutral(neutral)) = img.meta.color.as_shot else {
            panic!("no AsShotNeutral: {:?}", img.meta.color.as_shot);
        };
        assert!(
            neutral[0] < 0.9 && neutral[2] < 0.9,
            "AsShotNeutral {neutral:?}"
        );
        // IMG_1361.DNG (sha256 e91e77a4...fdbdfc2); lossless, so the means are exact.
        if data.len() == 29_195_592 {
            assert_eq!(img.clamped, 0);
            let n = (img.pixels.len() / 3) as u64;
            let means: Vec<u64> = (0..3)
                .map(|c| {
                    let sum: u64 = img
                        .pixels
                        .iter()
                        .skip(c)
                        .step_by(3)
                        .map(|&v| u64::from(v))
                        .sum();
                    (sum + n / 2) / n
                })
                .collect();
            assert_eq!(means, [4988, 12571, 7970]);
        }
    }
}

/// Wiring, not ground truth: `imgio::decode`, the path the pipeline uses,
/// hands over exactly what the linear DNG reader read (the same samples,
/// levels, color data and opcodes). The per-file tests above check the
/// samples themselves.
#[test]
fn decode_matches_the_linear_reader() {
    let folders = [
        "proraw/iPhone 16 Pro",
        "proraw/iPhone 13 Pro Max",
        "raw/Apple/iPhone 12 Pro",
        "raw/Samsung/Galaxy S23 Ultra",
    ];
    for (path, data) in folders.into_iter().flat_map(corpus_files) {
        println!("{}", path.display());
        let Ok(want) = dng::decode_linear(&data) else {
            assert!(genzo_imgio::decode(&data, DecodeOptions::default()).is_err());
            continue;
        };
        let img = genzo_imgio::decode(&data, DecodeOptions::default()).unwrap();
        assert_eq!((img.width, img.height), (want.width, want.height));
        assert_eq!(img.orientation, want.orientation);
        assert_eq!(u16::from(img.source_bits), want.bits_per_sample);
        let Pixels::Linear {
            samples_per_pixel,
            data,
            raw,
        } = img.pixels
        else {
            panic!("expected linear samples");
        };
        assert_eq!(samples_per_pixel, want.samples_per_pixel);
        assert!(data == want.pixels, "samples differ");
        // decode also plans the opcodes; the reader leaves that empty.
        let mut want_meta = want.meta;
        want_meta.opcode_plan = genzo_imgio::opcodes::plan(&want_meta.opcodes).unwrap();
        assert_eq!(*raw, want_meta);
    }
}

/// The Galaxy S23 Ultra DNG from raw.pixls.us (CC0): linear RGB in one strip
/// of lossless JPEG with a restart every 16 rows, and four optional GainMap
/// opcodes in OpcodeList2. Checked against libjpeg-turbo 3.2.0 (imagecodecs
/// 2026.10.10 `jpeg8_decode`): every sample identical, recorded as the exact
/// means and a position-weighted checksum. (lj92, imagecodecs'
/// `ljpeg_decode`, also ignores the restarts.)
#[test]
fn galaxy_s23_ultra_strips() {
    for (path, data) in corpus_files("raw/Samsung/Galaxy S23 Ultra") {
        println!("{}", path.display());
        let result = dng::decode_linear(&data);
        if !cfg!(feature = "raw") {
            assert!(
                matches!(result, Err(DngError::NotBuiltIn(_, "raw"))),
                "{result:?}"
            );
            continue;
        }
        let img = result.unwrap();
        assert_eq!(
            (img.width, img.height, img.samples_per_pixel),
            (3648, 2736, 3)
        );
        assert_eq!(img.compression, dng::COMPRESSION_LOSSLESS_JPEG);
        let gain_maps = &img.meta.opcodes.list2;
        assert_eq!(gain_maps.len(), 4);
        assert!(
            gain_maps
                .iter()
                .all(|op| op.kind == OpcodeKind::GainMap && op.optional)
        );
        // 20230817_120455.dng (sha256 4f9d1b08...e5ef3da4).
        if data.len() == 31_311_854 {
            let n = (img.pixels.len() / 3) as u64;
            let means: Vec<u64> = (0..3)
                .map(|c| {
                    let sum: u64 = img
                        .pixels
                        .iter()
                        .skip(c)
                        .step_by(3)
                        .map(|&v| u64::from(v))
                        .sum();
                    (sum + n / 2) / n
                })
                .collect();
            assert_eq!(means, [331, 716, 294]);
            let checksum = img.pixels.iter().enumerate().fold(0u64, |acc, (i, &v)| {
                acc.wrapping_add(u64::from(v).wrapping_mul((i % 65521) as u64 + 1))
            });
            assert_eq!(checksum, 438_548_270_903_595);
        }
    }
}

/// The Galaxy S23 Ultra's four GainMaps are no-ops (1 x 1, gain 1.0) whose
/// area runs past the image (left over from another sensor mode): they parse
/// and the file decodes.
#[test]
fn samsung_gain_maps_are_planned_no_ops() {
    for (_, data) in corpus_files("raw/Samsung/Galaxy S23 Ultra") {
        let img = genzo_imgio::decode(&data, DecodeOptions::default()).unwrap();
        let Pixels::Linear { raw, .. } = &img.pixels else {
            panic!("expected linear samples");
        };
        let maps = &raw.opcode_plan.list2;
        assert_eq!(maps.len(), 4);
        for op in maps {
            let genzo_imgio::opcodes::StageOpcode::GainMap(g) = op else {
                panic!("{op:?}")
            };
            assert_eq!(
                (g.points_v, g.points_h, g.gains.as_slice()),
                (1, 1, &[1.0f32][..])
            );
            assert!(g.area.right > img.width, "the area runs past the image");
        }
    }
}
