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

/// A CFA mosaic as one RGB value per pattern cell (2 × 2 for Bayer, 6 × 6
/// for X-Trans): each color's mean over the cell, after the black level of
/// each site and the white level. The masked borders are left in, so the
/// golden covers every sample the decoder returns.
fn superpixels(img: &genzo_imgio::DecodedImage) -> Image {
    let Pixels::Cfa { pattern, data, raw } = &img.pixels else {
        panic!("expected a CFA mosaic, got {:?}", img.pixels);
    };
    let (w, h) = (img.width as usize, img.height as usize);
    let (pw, ph) = (usize::from(pattern.width), usize::from(pattern.height));
    let levels = &raw.levels;
    let (br, bc) = (
        usize::from(levels.black_repeat.0),
        usize::from(levels.black_repeat.1),
    );
    let white = f64::from(levels.white[0]);
    let (cw, ch) = (w / pw, h / ph);
    let mut rgb = Vec::with_capacity(cw * ch);
    for cy in 0..ch {
        for cx in 0..cw {
            let (mut sum, mut n) = ([0.0f64; 3], [0u32; 3]);
            for y in cy * ph..(cy + 1) * ph {
                for x in cx * pw..(cx + 1) * pw {
                    let c = usize::from(pattern.color_at(y, x));
                    let black = levels.black[(y % br) * bc + x % bc];
                    sum[c] += (f64::from(data[y * w + x]) - black) / (white - black);
                    n[c] += 1;
                }
            }
            rgb.push([0, 1, 2].map(|c| (sum[c] / f64::from(n[c].max(1))) as f32));
        }
    }
    Image::new(cw as u32, ch as u32, rgb)
}

/// One golden per CFA layout the CI corpus holds (all CC0, raw.pixls.us):
/// the superpixels downscaled to about 250 px wide. Decoding is integer
/// work, so the result is the same on every machine.
fn cfa_golden(key: &str, name: &str, exposure: f64) {
    let Some(path) = corpus(key) else {
        return;
    };
    let img = genzo_imgio::decode(&std::fs::read(path).unwrap(), DecodeOptions::default()).unwrap();
    let cells = superpixels(&img);
    let factor = (cells.width / 250).max(1);
    let case = Case::new(
        env!("CARGO_MANIFEST_DIR"),
        name,
        Space::Linear(View::IDENTITY.with_exposure(exposure)),
        Producer::Cpu,
    );
    check(&case, &cells.box_downscale(factor));
}

/// Canon CR3 (CRX compression), Bayer.
#[test]
fn canon_r5_ii_cr3() {
    cfa_golden(
        "raw/Canon/Canon EOS R5m2/APS-C_CRAW.CR3",
        "decode/canon-r5-ii-cr3",
        2.0,
    );
}

/// Kodak DCR: a curve-coded format (exact with the patched rawler), GRBG,
/// and no white balance in the file.
#[test]
fn kodak_dcs760c_dcr() {
    cfa_golden(
        "raw/Kodak/DCS760C/86L57188.DCR",
        "decode/kodak-dcs760c-dcr",
        2.0,
    );
}

/// OM System OM-3 ORF: the two greens have different black levels.
#[test]
fn om_3_orf() {
    cfa_golden(
        "raw/OM Digital Solutions/OM-3/P5048182.ORF",
        "decode/om-3-orf",
        2.0,
    );
}

/// Fujifilm X-T5 RAF: X-Trans, 6 × 6.
#[test]
fn fujifilm_x_t5_raf() {
    cfa_golden(
        "raw/Fujifilm/X-T5/DSCF0021.RAF",
        "decode/fujifilm-x-t5-raf",
        2.0,
    );
}

/// Pentax K10D: a CFA DNG (12-bit packed strips) through rawler, with the
/// pattern and levels from Genzo's DNG reader.
#[test]
fn pentax_k10d_cfa_dng() {
    cfa_golden(
        "raw/Pentax/K10D/IMGP5398.DNG",
        "decode/pentax-k10d-cfa-dng",
        2.0,
    );
}
