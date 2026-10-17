//! The linear DNG reader on the real ProRAW files of the 0.C corpus. The
//! corpus is gitignored (personal photos), so each check skips when its file
//! is missing; run them where the corpus is, in release for speed:
//!   cargo test --release -p genzo-imgio --test dng_corpus -- --nocapture
#![cfg(feature = "dng")]

use std::path::PathBuf;

use genzo_imgio::dng::{self, COMPRESSION_JPEG_XL, DngError};

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
    assert!(img.has_gain_table_map);
    assert_eq!(img.as_shot_neutral, Some(vec![1.0; 3]));
    assert!(img.baseline_exposure.is_some());
    assert!(img.color_matrix_1.is_some() && img.color_matrix_2.is_some());
    // A handful of samples past the table is normal; many would mean the
    // samples were read at the wrong scale.
    assert!(img.clamped < 1000, "{} samples clamped", img.clamped);

    let spp = usize::from(img.samples_per_pixel);
    let n = img.pixels.len() / spp;
    let white = f64::from(img.white_level[0]);
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
        img.clamped, img.baseline_exposure
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
                Err(DngError::Unsupported(m)) => assert!(m.contains("`raw` feature"), "{m}"),
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
        assert!(img.has_gain_table_map);
        let exposure = img.baseline_exposure.unwrap();
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
                matches!(result, Err(DngError::Unsupported(_))),
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
        let neutral = img.as_shot_neutral.clone().unwrap();
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
