//! Golden images of decoded RAW data (DEV-01, criterion 5): what `imgio` hands
//! the pipeline, before any stage runs.
//!
//! The image is the decoder's linear output, normalized by the black and white
//! levels to 0 at black and 1 at white, then box-downscaled to keep the golden
//! small. Decoding is integer work, so the result is the same on every
//! machine; any change in it is a decoder change.

#![cfg(feature = "raw")]

use genzo_golden::{Case, Image, Producer, Space, View, check, corpus};
use genzo_imgio::{DecodeOptions, Pixels};

/// The iPhone 12 Pro ProRAW (CC0, raw.pixls.us), lossless JPEG, 4032 × 3024,
/// reduced 16 times to 252 × 189. Its linear values are low (means of 0.08 to
/// 0.19 of white), so the ΔE view adds 2 stops. Decoded through
/// `imgio::decode`, the path the pipeline uses.
#[test]
fn iphone_12_pro_linear_dng() {
    let Some(path) = corpus("raw/Apple/iPhone 12 Pro/IMG_1361.DNG") else {
        return;
    };
    let img = genzo_imgio::decode(&std::fs::read(path).unwrap(), DecodeOptions::default()).unwrap();
    let Pixels::Linear {
        samples_per_pixel,
        data,
        raw,
    } = &img.pixels
    else {
        panic!("expected linear samples, got {:?}", img.pixels);
    };
    let levels = &raw.levels;
    assert_eq!(levels.black_repeat, (1, 1));
    let rgb = data
        .chunks_exact(usize::from(*samples_per_pixel))
        .map(|p| {
            [0, 1, 2].map(|c| {
                let (b, w) = (levels.black[c], f64::from(levels.white[c]));
                ((f64::from(p[c]) - b) / (w - b)) as f32
            })
        })
        .collect();
    let image = Image::new(img.width, img.height, rgb).box_downscale(16);

    let case = Case::new(
        env!("CARGO_MANIFEST_DIR"),
        "decode/iphone-12-pro",
        Space::Linear(View::IDENTITY.with_exposure(2.0)),
        Producer::Cpu,
    );
    check(&case, &image);
}
