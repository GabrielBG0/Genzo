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

/// The iPhone 16 Pro 48 MP ProRAW with JPEG XL tiles. The numbers are the
/// ones the 0.C prototype recorded in docs/m0-results.md.
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
    assert_eq!(img.clamped, 3);
    assert!(img.has_gain_table_map);
    assert_eq!(img.opcode_lists, [false; 3]);
    assert_eq!(img.as_shot_neutral, Some(vec![1.0; 3]));
    let exposure = img.baseline_exposure.unwrap();
    assert!(
        (exposure - 0.51).abs() < 0.01,
        "BaselineExposure {exposure}"
    );
    assert!(img.color_matrix_1.is_some() && img.color_matrix_2.is_some());

    let spp = usize::from(img.samples_per_pixel);
    let n = img.pixels.len() / spp;
    for c in 0..spp {
        let mean = img
            .pixels
            .iter()
            .skip(c)
            .step_by(spp)
            .map(|&v| u64::from(v))
            .sum::<u64>() as f64
            / n as f64;
        println!("channel {c}: mean {mean:.0}");
        assert!((3500.0..5000.0).contains(&mean), "channel {c} mean {mean}");
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
