//! OUT-01 criterion 2 on the GPU: a resized export, its regions rendered
//! through stages 1 to 7 and reduced on the CPU, matches the reference (the
//! whole image's stages 1–7 as one region, each output pixel the mean of
//! the pixels it covers, then stage 9) code for code, at sizes whose output
//! pixels span several regions; and it is close to the Fit frame of the
//! same size (mean ΔE2000, recorded).

use std::sync::{Mutex, MutexGuard};

use genzo_color::output::{Target, pixel};
use genzo_export::resize::Reducer;
use genzo_model::edit::{Edit, WhiteBalanceMode};
use genzo_model::frame::Rect;
use genzo_pipeline::gpu::develop::{Develop, Phase, Photo};
use genzo_pipeline::gpu::export::{ExportStages, resized};
use genzo_pipeline::gpu::raw_stages::TileConfig;
use genzo_pipeline::gpu::synthetic::{scene_raw, scene_xtrans_raw, with_camera_profile};
use genzo_pipeline::gpu::{AdapterChoice, Gpu, GpuError};
use genzo_pipeline::reference::sampling::ViewSpec;

static ONE_DEVICE: Mutex<()> = Mutex::new(());

fn gpu() -> Option<(MutexGuard<'static, ()>, Gpu)> {
    let lock = ONE_DEVICE.lock().unwrap_or_else(|e| e.into_inner());
    match Gpu::open(AdapterChoice::from_env()) {
        Ok(gpu) => Some((lock, gpu)),
        Err(GpuError::NoAdapter) if std::env::var("GENZO_REQUIRE_GPU").as_deref() != Ok("1") => {
            eprintln!("skipped: no GPU adapter");
            None
        }
        Err(e) => panic!("{e}"),
    }
}

const TILES: TileConfig = TileConfig {
    tile: 128,
    xtrans_tile: 128,
    apron: 24,
};

fn edited(defaults: &Edit) -> Edit {
    let mut e = defaults.clone();
    e.white_balance.mode = WhiteBalanceMode::Custom;
    e.white_balance.temp = 5600.0;
    e.white_balance.tint = 6.0;
    e.exposure = 0.4;
    e.highlights = -50.0;
    e.shadows = 40.0;
    e.vibrance = 20.0;
    e.crop.angle = 4.0;
    e
}

fn settle(gpu: &Gpu, d: &mut Develop) {
    while d.rebuilding() {
        d.continue_rebuild(gpu, || false).unwrap();
    }
}

#[test]
fn a_resized_export_is_the_mean_of_its_full_resolution_pixels() {
    let Some((_lock, gpu)) = gpu() else { return };
    let photos = [
        ("bayer", with_camera_profile(scene_raw(1030, 777)), 1),
        ("x-trans", scene_xtrans_raw(1030, 777), 6),
    ];
    for (name, src, orientation) in photos {
        let mut d = Develop::new(&gpu, TILES);
        d.open(&gpu, Photo { src, orientation }).unwrap();
        let edit = edited(&Edit::defaults(d.kind().unwrap()));
        let inputs = d.export_inputs(&gpu, &edit, Target::Srgb).unwrap();
        settle(&gpu, &mut d);
        let source = inputs.size();
        let stage9 = inputs.stage9();
        let mut export = ExportStages::new(&gpu);
        // The whole image as one region: the reference's input.
        let whole_rect = Rect {
            x: 0,
            y: 0,
            width: source.0,
            height: source.1,
        };
        let whole = d
            .render_export_linear(&gpu, &mut export, &inputs, whole_rect)
            .unwrap();
        for long_edge in [500, 211, 40] {
            let size = resized(source, long_edge).unwrap();
            // The reference: each output pixel's mean, then stage 9.
            let starts = |len: u32, n: u32| -> Vec<u32> {
                (0..=n)
                    .map(|i| (u64::from(i) * u64::from(len) / u64::from(n)) as u32)
                    .collect()
            };
            let (cs, rs) = (starts(source.0, size.0), starts(source.1, size.1));
            let mut want = Vec::new();
            for oy in 0..size.1 {
                for ox in 0..size.0 {
                    let mut sum = [0.0f64; 3];
                    let mut n = 0.0;
                    for y in rs[oy as usize]..rs[oy as usize + 1] {
                        for x in cs[ox as usize]..cs[ox as usize + 1] {
                            let px = whole[(y * source.0 + x) as usize];
                            for k in 0..3 {
                                sum[k] += f64::from(px[k]);
                            }
                            n += 1.0;
                        }
                    }
                    let [r, g, b] = pixel(
                        sum.map(|s| s / n),
                        (ox, oy),
                        stage9.target,
                        stage9.compress,
                        stage9.dither,
                    );
                    want.extend_from_slice(&[r, g, b, 255]);
                }
            }
            // The export: regions of 300 px, so output pixels straddle them.
            let mut reducer = Reducer::new(source, size, stage9);
            let mut got = Vec::new();
            for ry in (0..source.1).step_by(300) {
                let h = 300.min(source.1 - ry);
                for rx in (0..source.0).step_by(300) {
                    let rect = Rect {
                        x: rx,
                        y: ry,
                        width: 300.min(source.0 - rx),
                        height: h,
                    };
                    let rgb = d
                        .render_export_linear(&gpu, &mut export, &inputs, rect)
                        .unwrap();
                    reducer.add(rect, &rgb);
                }
                got.extend(reducer.finish(ry + h).1);
            }
            let off = got
                .chunks(4)
                .zip(want.chunks(4))
                .filter(|(a, b)| a != b)
                .count();
            assert_eq!(got.len(), want.len(), "{name} {size:?}");
            assert_eq!(
                off, 0,
                "{name} {size:?}: {off} pixels differ from the reference"
            );

            // Against the Fit frame of the same size.
            d.render(&gpu, &edit, ViewSpec::Fit(size), Target::Srgb, Phase::End)
                .unwrap();
            settle(&gpu, &mut d);
            let fit = d.view().read_frame(&gpu).unwrap();
            assert_eq!(
                fit.len() as u32,
                size.0 * size.1,
                "{name}: the Fit frame's size"
            );
            let unit = |p: &[u8]| [p[0], p[1], p[2]].map(|v| f64::from(v) / 255.0);
            let mut des: Vec<f64> = got
                .chunks(4)
                .zip(&fit)
                .map(|(a, b)| genzo_color::delta_e_2000_srgb(unit(a), unit(b)))
                .collect();
            let mean = des.iter().sum::<f64>() / des.len() as f64;
            des.sort_by(f64::total_cmp);
            let (median, p99) = (des[des.len() / 2], des[des.len() * 99 / 100]);
            eprintln!(
                "{name}: {size:?} matches the reference; against Fit, ΔE2000 median {median:.2}, mean {mean:.2}, p99 {p99:.1}"
            );
            // The two differ where the image has hard edges: the export
            // averages pixels the tone curve and local tone mapping already
            // ran on, with a box over each output pixel's exact span; Fit
            // samples a box-filtered pyramid level bilinearly and tones that.
            // Elsewhere they agree: the median pixel under ΔE2000 1, about
            // the smallest visible difference (OUT-01, criterion 2). At a
            // 40 px long edge most output pixels hold an edge: 0.49.
            assert!(
                median < 1.0,
                "{name} {size:?}: median ΔE2000 {median:.2} against Fit"
            );
        }
    }
}
