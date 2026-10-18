//! Criterion benchmarks for stage 0: `imgio::decode` of each linear DNG
//! layout, and of camera RAW and CFA DNG through rawler, on real files (CC0,
//! raw.pixls.us, fetched by scripts/fetch-raw-corpus.py), plus the owner's
//! 48 MP ProRAW when it is there. A missing file is skipped. The file is read once; each iteration
//! is the decode alone, samples and metadata.

use criterion::{Criterion, criterion_group, criterion_main};
use genzo_golden::corpus;
use genzo_imgio::DecodeOptions;

const FILES: &[(&str, &str)] = &[
    (
        "iPhone 12 Pro 12 MP, lossless JPEG tiles",
        "raw/Apple/iPhone 12 Pro/IMG_1361.DNG",
    ),
    (
        "Galaxy S23 Ultra 10 MP, lossless JPEG strip with restarts",
        "raw/Samsung/Galaxy S23 Ultra/20230817_120455.dng",
    ),
    (
        "iPhone 16 Pro 48 MP, JPEG XL tiles",
        "proraw/iPhone 16 Pro/IPhone16proRaw.DNG",
    ),
    (
        "Canon R5 II CR3 19 MP",
        "raw/Canon/Canon EOS R5m2/APS-C_CRAW.CR3",
    ),
    (
        "Nikon D850 NEF 46 MP, 12-bit compressed",
        "raw/Nikon/D850/Nikon-D850-12bit-compressed.NEF",
    ),
    (
        "Sony A7R V ARW 61 MP, lossless",
        "raw/Sony/ILCE-7RM5/7RM5-LosslessCompressedLarge.ARW",
    ),
    (
        "Fujifilm X-T5 RAF 40 MP, X-Trans",
        "raw/Fujifilm/X-T5/DSCF0021.RAF",
    ),
    ("Leica Q2 CFA DNG 47 MP", "raw/Leica/Q2/L1000750.DNG"),
];

fn decode(c: &mut Criterion) {
    let mut group = c.benchmark_group("decode");
    group.sample_size(10);
    for &(name, key) in FILES {
        let Some(path) = corpus(key) else {
            continue;
        };
        let data = std::fs::read(path).unwrap();
        group.bench_function(name, |b| {
            b.iter(|| genzo_imgio::decode(&data, DecodeOptions::default()).unwrap());
        });
    }
    group.finish();
}

/// JPEG, PNG and TIFF of the same 12 MP photo. The corpus has no CC0 JPEG
/// or PNG, so they are made here from Adobe's 16-bit TIFF render of the
/// iPhone 12 Pro file (corpus/reference, local only): a real photo's
/// content, which is what compression speed depends on.
fn non_raw(c: &mut Criterion) {
    let Some(path) = corpus("reference/IMG_1361/IMG_1361.tiff") else {
        return;
    };
    let tiff = std::fs::read(path).unwrap();
    let img = genzo_imgio::decode(&tiff, DecodeOptions::default()).unwrap();
    let genzo_imgio::Pixels::Rgba16 { data, .. } = &img.pixels else {
        unreachable!("a TIFF decodes to RGBA16")
    };
    let (w, h) = (img.width, img.height);
    let rgb8: Vec<u8> = data
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|p| [p[0] >> 8, p[1] >> 8, p[2] >> 8].map(|v| v as u8))
        .collect();
    let mut jpeg = Vec::new();
    jpeg_encoder::Encoder::new(&mut jpeg, 90)
        .encode(&rgb8, w as u16, h as u16, jpeg_encoder::ColorType::Rgb)
        .unwrap();
    let mut png16 = Vec::new();
    {
        let mut e = png::Encoder::new(&mut png16, w, h);
        e.set_color(png::ColorType::Rgb);
        e.set_depth(png::BitDepth::Sixteen);
        let be: Vec<u8> = data
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|p| {
                p[..3]
                    .iter()
                    .flat_map(|v| v.to_be_bytes())
                    .collect::<Vec<_>>()
            })
            .collect();
        e.write_header().unwrap().write_image_data(&be).unwrap();
    }
    let mut group = c.benchmark_group("decode");
    group.sample_size(10);
    for (name, file) in [
        ("TIFF 12 MP, 16-bit uncompressed", &tiff),
        ("JPEG 12 MP, quality 90, 4:2:0", &jpeg),
        ("PNG 12 MP, 16-bit", &png16),
    ] {
        group.bench_function(name, |b| {
            b.iter(|| genzo_imgio::decode(file, DecodeOptions::default()).unwrap());
        });
    }
    group.finish();
}

criterion_group!(benches, decode, non_raw);
criterion_main!(benches);
