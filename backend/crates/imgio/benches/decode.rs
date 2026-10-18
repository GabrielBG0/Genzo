//! Criterion benchmarks for stage 0: `imgio::decode` of each linear DNG
//! layout on a real file (CC0, raw.pixls.us, fetched by
//! scripts/fetch-raw-corpus.py), plus the owner's 48 MP ProRAW when it is
//! there. A missing file is skipped. The file is read once; each iteration
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
