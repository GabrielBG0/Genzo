//! The settled white balance rebuild, visible area first (DEV-01,
//! Settled): the pyramid is rebuilt in place, first the tiles the view and
//! the base layer read, then the rest.
//!
//! - Finished, in one go or stopped between tiles and resumed, the pyramid
//!   equals a new one rendered with the new white, bit for bit, at every
//!   level, X-Trans included.
//! - Right after the first part, the frame (base layer rebuilt, every stage
//!   on, the dither) equals the frame from a whole new pyramid, bit for
//!   bit: in a turned, flipped and straightened tight crop at four Fit
//!   sizes (levels 0 to 3), and for the whole image, where the
//!   first part is every tile.
//! - The level-0 reach is exactly the pixels a view's texels read, level by
//!   level down, per texel, at odd and even sizes.
//! - The rebuild allocates no second pyramid: its peak is the first
//!   render's, and it leaves nothing behind.

use std::sync::{Mutex, MutexGuard};

use genzo_color::camera::xy_from_temperature;
use genzo_color::output::Target;
use genzo_model::frame::Rect;
use genzo_pipeline::camera_color::{CameraTransform, camera_transform};
use genzo_pipeline::geometry::{Crop, Geometry, fit};
use genzo_pipeline::gpu::base::BaseLayer;
use genzo_pipeline::gpu::level0::Level0Cache;
use genzo_pipeline::gpu::raw_stages::{RawStages, TileConfig, level0_reach};
use genzo_pipeline::gpu::synthetic::{scene_raw, scene_xtrans_raw, with_camera_profile};
use genzo_pipeline::gpu::view::ViewStages;
use genzo_pipeline::gpu::{AdapterChoice, Gpu, GpuError};
use genzo_pipeline::look::{BaseCurve, Look, Profile};
use genzo_pipeline::raw::RawSource;
use genzo_pipeline::reference::sampling::{Frame, ViewSpec, plan};
use genzo_pipeline::reference::stages::{half, span};

static ONE_DEVICE: Mutex<()> = Mutex::new(());

fn gpu() -> Option<(MutexGuard<'static, ()>, Gpu)> {
    let lock = ONE_DEVICE.lock().unwrap_or_else(|e| e.into_inner());
    match Gpu::open(AdapterChoice::from_env()) {
        Ok(gpu) => {
            eprintln!("adapter: {} ({})", gpu.info.name, gpu.info.backend);
            Some((lock, gpu))
        }
        Err(GpuError::NoAdapter) if std::env::var("GENZO_REQUIRE_GPU").as_deref() != Ok("1") => {
            eprintln!("skipped: no GPU adapter");
            None
        }
        Err(e) => panic!("{e}"),
    }
}

/// Small tiles, so a 1030 x 777 image has many and a crop meets few. A
/// tile's pixels do not depend on the tile size (tests/raw_stages.rs).
const TILES: TileConfig = TileConfig {
    tile: 128,
    xtrans_tile: 128,
    apron: 24,
};

/// Tiles finer than any filter's reach, so a reach a few pixels short
/// misses a tile and shows in the frame.
const FINE: TileConfig = TileConfig {
    tile: 32,
    xtrans_tile: 32,
    apron: 24,
};

fn whites(src: &RawSource) -> (CameraTransform, CameraTransform) {
    (
        camera_transform(&src.color, None),
        camera_transform(&src.color, Some(xy_from_temperature(3400.0, 12.0))),
    )
}

/// Every level of the pyramid `stages` holds.
fn levels(gpu: &Gpu, stages: &RawStages) -> Vec<Vec<[f32; 3]>> {
    let p = stages.pyramid().unwrap();
    (1..=p.levels)
        .map(|l| p.read_level(gpu, l).unwrap())
        .collect()
}

#[test]
fn a_finished_rebuild_is_a_new_pyramid() {
    let Some((_lock, gpu)) = gpu() else { return };
    for (name, src) in [
        ("Bayer", with_camera_profile(scene_raw(1030, 777))),
        ("X-Trans", with_camera_profile(scene_xtrans_raw(611, 409))),
    ] {
        let (old, new) = whites(&src);
        let mut fresh = RawStages::new(&gpu, TILES);
        fresh.render_pyramid(&gpu, &src, &new).unwrap();
        let want = levels(&gpu, &fresh);
        let first = Rect {
            x: 300,
            y: 200,
            width: 151,
            height: 97,
        };
        // In one go; stopped after three tiles and resumed; with no first
        // part.
        for (case, first, stop_after) in [
            ("in one go", Some(first), None),
            ("stopped and resumed", Some(first), Some(3)),
            ("no first part", None, None),
        ] {
            let mut stages = RawStages::new(&gpu, TILES);
            stages.render_pyramid(&gpu, &src, &old).unwrap();
            let mut rebuild = stages.rebuild_first(&gpu, &src, &new, first).unwrap();
            let left = rebuild.left();
            assert!(left > 3, "{name}, {case}: {left} tiles left");
            if let Some(n) = stop_after {
                let mut asked = 0;
                let done = stages
                    .rebuild_rest(&gpu, &src, &mut rebuild, || {
                        asked += 1;
                        asked > n
                    })
                    .unwrap();
                assert!(!done, "{name}, {case}");
                assert_eq!(rebuild.left(), left - n, "{name}, {case}");
            }
            assert!(
                stages
                    .rebuild_rest(&gpu, &src, &mut rebuild, || false)
                    .unwrap()
            );
            assert_eq!(rebuild.left(), 0);
            let p = stages.pyramid().unwrap();
            assert_eq!(p.transform.multipliers, new.multipliers, "{name}, {case}");
            let got = levels(&gpu, &stages);
            for (l, (g, w)) in got.iter().zip(&want).enumerate() {
                let off = g.iter().zip(w).filter(|(a, b)| a != b).count();
                assert_eq!(off, 0, "{name}, {case}: level {} differs", l + 1);
            }
        }
    }
}

/// Every stage on: local tone, contrast, stage 7, Genzo Standard.
fn look() -> Look {
    Look {
        exposure: 0.4,
        highlights: -70.0,
        shadows: 60.0,
        contrast: 25.0,
        whites: -15.0,
        vibrance: 30.0,
        saturation: -10.0,
        ..Look::with_profile(Profile::GenzoStandard)
    }
}

fn union(a: Option<Rect>, b: Option<Rect>) -> Option<Rect> {
    match (a, b) {
        (Some(a), Some(b)) => {
            let (x, y) = (a.x.min(b.x), a.y.min(b.y));
            let right = (a.x + a.width).max(b.x + b.width);
            let bottom = (a.y + a.height).max(b.y + b.height);
            Some(Rect {
                x,
                y,
                width: right - x,
                height: bottom - y,
            })
        }
        (a, b) => a.or(b),
    }
}

/// Shows `spec` of `frame` with every stage on and returns the frame's
/// codes and the view's sampling.
#[allow(clippy::too_many_arguments)]
fn render(
    gpu: &Gpu,
    stages: &mut RawStages,
    cache: &mut Level0Cache,
    layer: &BaseLayer,
    src: &RawSource,
    key: u64,
    frame: Frame,
    spec: ViewSpec,
) -> Vec<[u8; 4]> {
    let look = look();
    let curve = BaseCurve::new(&src.color);
    let mut view = ViewStages::new(gpu);
    view.show(gpu, stages, cache, src, key, frame, spec)
        .unwrap();
    view.tone(gpu, &look, Some(&curve), Some(layer));
    view.color(gpu, &look);
    view.output(gpu, Target::Srgb, true, true);
    view.read_frame(gpu).unwrap()
}

#[test]
fn the_first_part_serves_the_view() {
    let Some((_lock, gpu)) = gpu() else { return };
    let src = with_camera_profile(scene_raw(1030, 777));
    let size = src.size();
    let (old, new) = whites(&src);
    let g = Geometry {
        quarter_turns: 1,
        flip_horizontal: true,
        crop: Crop {
            cx: 0.62,
            cy: 0.41,
            w: 0.33,
            h: 0.29,
            angle: 7.5,
        },
    };
    let crop = fit(g.crop, size, g.orientation(6)).expect("fits");
    let tight = Frame::of(&Geometry { crop, ..g }, 6);
    let mut fresh = RawStages::new(&gpu, TILES);
    fresh.render_pyramid(&gpu, &src, &new).unwrap();
    for (name, frame, out, level) in [
        ("tight crop", tight, (520, 470), 0),
        ("tight crop", tight, (100, 90), 1),
        ("tight crop", tight, (37, 33), 2),
        ("tight crop", tight, (17, 15), 3),
        ("whole image", Frame::from(6), (301, 399), 1),
    ] {
        let spec = ViewSpec::Fit(out);
        let mut fresh_layer = BaseLayer::new(&gpu);
        fresh_layer.build(&gpu, fresh.pyramid().unwrap(), size, frame);
        let mut fresh_cache = Level0Cache::new(&gpu);
        let want = render(
            &gpu,
            &mut fresh,
            &mut fresh_cache,
            &fresh_layer,
            &src,
            2,
            frame,
            spec,
        );

        // The old white's pyramid, base layer and view, then the first part
        // of the rebuild over what both read.
        let mut stages = RawStages::new(&gpu, FINE);
        stages.render_pyramid(&gpu, &src, &old).unwrap();
        let mut layer = BaseLayer::new(&gpu);
        layer.build(&gpu, stages.pyramid().unwrap(), size, frame);
        let mut cache = Level0Cache::new(&gpu);
        let mut view = ViewStages::new(&gpu);
        let s = view
            .show(&gpu, &mut stages, &mut cache, &src, 1, frame, spec)
            .unwrap();
        let reach = union(
            level0_reach(&s, out, size),
            level0_reach(layer.sampling().unwrap(), layer.size(), size),
        );
        let rebuild = stages.rebuild_first(&gpu, &src, &new, reach).unwrap();
        layer.build(&gpu, stages.pyramid().unwrap(), size, frame);
        let got = render(&gpu, &mut stages, &mut cache, &layer, &src, 2, frame, spec);
        let off = got.iter().zip(&want).filter(|(a, b)| a != b).count();
        assert_eq!(s.level, level, "{name}, {out:?}");
        assert_eq!(off, 0, "{name}: view level {level}, {off} pixels differ");
        let total = size.0.div_ceil(FINE.tile) * size.1.div_ceil(FINE.tile);
        eprintln!(
            "{name}: view level {level}, first part {} of {total} tiles",
            total as usize - rebuild.left()
        );
        if name == "whole image" {
            assert_eq!(rebuild.left(), 0, "the whole image reads every tile");
        } else {
            assert!(rebuild.left() > 0, "{name}: a tight crop leaves tiles");
        }
    }
}

#[test]
fn a_rebuild_allocates_no_second_pyramid() {
    let Some((_lock, gpu)) = gpu() else { return };
    let src = with_camera_profile(scene_raw(2048, 1536));
    let (old, new) = whites(&src);
    let mut stages = RawStages::new(&gpu, TileConfig::default());
    stages.sample_memory = true;
    stages.render_pyramid(&gpu, &src, &old).unwrap();
    let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
    let (Some(render_peak), Some(before)) = (
        stages.peak_allocated.take(),
        gpu.device
            .generate_allocator_report()
            .map(|r| r.total_allocated_bytes),
    ) else {
        eprintln!("skipped: the backend reports no allocations");
        return;
    };
    let first = Rect {
        x: 0,
        y: 0,
        width: 600,
        height: 400,
    };
    let mut rebuild = stages.rebuild_first(&gpu, &src, &new, Some(first)).unwrap();
    stages
        .rebuild_rest(&gpu, &src, &mut rebuild, || false)
        .unwrap();
    drop(rebuild);
    let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
    let peak = stages.peak_allocated.unwrap();
    let after = gpu
        .device
        .generate_allocator_report()
        .unwrap()
        .total_allocated_bytes;
    const MB: f64 = 1e6;
    eprintln!(
        "render peak {:.1} MB, rebuild peak {:.1} MB, {:+.2} MB after",
        render_peak as f64 / MB,
        peak as f64 / MB,
        (after as f64 - before as f64) / MB
    );
    // The image's small buffers and the level uniforms are allocated again
    // (64 KiB each on D3D12); a second pyramid would be 6 MB more here.
    assert!(peak <= render_peak + 1_000_000, "{render_peak} then {peak}");
    assert!(after <= before + 1_000_000, "{before} then {after}");
}

#[test]
fn the_reach_is_what_the_texels_read() {
    for level0 in [(1030u32, 777u32), (1031, 778), (999, 1001)] {
        let mut sizes = vec![level0];
        while *sizes.last().unwrap() != (1, 1) {
            sizes.push(half(*sizes.last().unwrap()));
        }
        let at = |level: usize, axis: usize| [sizes[level].0, sizes[level].1][axis];
        for out in [(600, 450), (301, 227), (100, 90), (37, 33), (9, 7), (3, 2)] {
            for o in [1u8, 6] {
                let s = plan(&sizes[1..], o, out);
                let reach = level0_reach(&s, out, level0).expect("a pyramid level");
                let r = s.source_rect(out);
                let got = [(reach.x, reach.width), (reach.y, reach.height)];
                for (axis, (lo, len)) in [(r.x, r.width), (r.y, r.height)].into_iter().enumerate() {
                    // Per texel: downsample.wgsl's clamped pair, then
                    // tile_stage4.wgsl's span.
                    let mut read: Vec<u32> = (lo..lo + len).collect();
                    for level in (2..=s.level as usize).rev() {
                        let n = at(level - 1, axis);
                        read = read
                            .iter()
                            .flat_map(|&p| [(2 * p).min(n - 1), (2 * p + 1).min(n - 1)])
                            .collect();
                    }
                    read = read
                        .iter()
                        .flat_map(|&i| span(i, at(1, axis), at(0, axis)))
                        .collect();
                    read.sort_unstable();
                    read.dedup();
                    let (first, last) = (read[0], *read.last().unwrap());
                    assert_eq!(read.len() as u32, last - first + 1, "contiguous");
                    assert_eq!(
                        got[axis],
                        (first, last - first + 1),
                        "{level0:?}, {out:?}, orientation {o}, level {}, axis {axis}",
                        s.level
                    );
                }
            }
        }
    }
}
