//! The linear DNG reader on the real ProRAW files of the 0.C corpus. The
//! corpus is gitignored (personal photos), so each check skips when its file
//! is missing; run them where the corpus is, in release for speed:
//!   cargo test --release -p genzo-imgio --test dng_corpus -- --nocapture
#![cfg(feature = "dng")]

use std::path::PathBuf;

use genzo_imgio::dng::{self, COMPRESSION_JPEG_XL, DngError};

/// The first DNG in corpus/proraw/<model>/, if the corpus has one.
fn corpus_file(model: &str) -> Option<Vec<u8>> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../corpus/proraw")
        .join(model);
    let path = std::fs::read_dir(&dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("dng")))?;
    println!("reading {}", path.display());
    std::fs::read(path).ok()
}

/// Facts recorded for known corpus files, told apart by size: the clamp count
/// and the rounded per-channel means. Means are checked within 1, since the
/// tiles are lossy JPEG XL and decoders may round the last step differently.
const KNOWN_16_PRO: &[(usize, u64, [f64; 3])] = &[
    // IMG_6889.DNG (iOS 27.2), checked against libjxl: 0.018 % of samples
    // differ, each by one LinearizationTable step.
    (20_306_343, 7, [6789.0, 6669.0, 6715.0]),
];

/// The iPhone 16 Pro 48 MP ProRAW with JPEG XL tiles.
#[test]
fn iphone_16_pro_jpeg_xl() {
    let Some(data) = corpus_file("iPhone 16 Pro") else {
        println!("skipped: no iPhone 16 Pro file in corpus/proraw");
        return;
    };
    let started = std::time::Instant::now();
    let img = dng::decode_linear(&data).unwrap();
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

/// The iPhone 13 Pro Max ProRAW uses lossless JPEG tiles, which stay with
/// rawler: the reader must say so, not misread them.
#[test]
fn iphone_13_pro_max_is_left_to_rawler() {
    let Some(data) = corpus_file("iPhone 13 Pro Max") else {
        println!("skipped: no iPhone 13 Pro Max file in corpus/proraw");
        return;
    };
    match dng::decode_linear(&data) {
        Err(DngError::Unsupported(m)) => assert!(m.contains("lossless JPEG"), "{m}"),
        other => panic!("expected Unsupported, got {other:?}"),
    }
}
