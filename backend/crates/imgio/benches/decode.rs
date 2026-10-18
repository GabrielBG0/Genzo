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

criterion_group!(benches, decode);
criterion_main!(benches);
