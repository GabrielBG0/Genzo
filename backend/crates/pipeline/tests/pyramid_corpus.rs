//! Real files through stages 1 to 4 into the pyramid (a missing file is
//! skipped): each one's own color data gives a transform without a fallback,
//! and every level 1 value is finite. The DNGs bring AnalogBalance (iPhone 16
//! Pro), CameraCalibration (Galaxy S23 Ultra), GainMaps (Pixel 4a) and
//! FixBadPixelsConstant (Leica Q2); the camera RAWs rawler's matrices and its
//! white balance coefficients.

#[allow(dead_code)]
mod sdk_stage2;

use genzo_color::camera::temperature_from_xy;
use genzo_imgio::{DecodeOptions, decode};
use genzo_pipeline::camera_color::camera_transform;
use genzo_pipeline::gpu::raw_stages::{RawStages, TileConfig};
use genzo_pipeline::gpu::{AdapterChoice, Gpu, GpuError};
use genzo_pipeline::raw::RawSource;

const KEYS: &[&str] = &[
    "raw/Google/Pixel 4a/PXL_20201121_100251397.dng",
    "raw/Leica/Q2/L1000750.DNG",
    "raw/Pentax/K10D/IMGP5398.DNG",
    "raw/Samsung/Galaxy S23 Ultra/20230817_120455.dng",
    "raw/Apple/iPhone 12 Pro/IMG_1361.DNG",
    "proraw/iPhone 16 Pro/IPhone16proRaw.DNG",
    "bayer-dng/iphone/Halide_iPhone16Pro_bayer.dng",
    "raw/Canon/Canon EOS R5m2/APS-C_CRAW.CR3",
    "raw/OM Digital Solutions/OM-3/P5048182.ORF",
    "raw/Nikon/D850/Nikon-D850-12bit-compressed.NEF",
    "raw/Kodak/DCS760C/86L57188.DCR",
];

#[test]
fn corpus_files_render_into_the_pyramid() {
    let gpu = match Gpu::open(AdapterChoice::from_env()) {
        Ok(gpu) => gpu,
        Err(GpuError::NoAdapter) if std::env::var("GENZO_REQUIRE_GPU").as_deref() != Ok("1") => {
            eprintln!("skipped: no GPU adapter");
            return;
        }
        Err(e) => panic!("{e}"),
    };
    let mut stages = RawStages::new(&gpu, TileConfig::default());
    let mut rendered = 0;
    for key in KEYS {
        let Some(path) = sdk_stage2::corpus_file(key) else {
            continue;
        };
        let img = decode(&std::fs::read(path).unwrap(), DecodeOptions::default()).unwrap();
        let src = RawSource::from_decoded(img).unwrap();
        let t = camera_transform(&src.color, None);
        assert_eq!(t.fallback, None, "{key}");
        let p = stages.render_pyramid(&gpu, &src, &t).unwrap();
        let level1 = p.read_level(&gpu, 1).unwrap();
        assert!(level1.iter().flatten().all(|v| v.is_finite()), "{key}");
        let mean: f64 =
            level1.iter().flatten().map(|&v| f64::from(v)).sum::<f64>() / (level1.len() * 3) as f64;
        let (temp, tint) = temperature_from_xy(t.white);
        println!(
            "{key}: white {temp:.0} K, tint {tint:+.1}, multipliers {:.3?}, level 1 mean {mean:.3}",
            t.multipliers
        );
        // A photo, not black or blown out.
        assert!(mean > 0.005 && mean < 2.0, "{key}: mean {mean}");
        rendered += 1;
    }
    println!("{rendered} files");
}
