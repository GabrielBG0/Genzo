//! Level-0 tiles on the GPU (DEV-01, At 1:1): full-resolution stage 4 in
//! 256 px cache tiles, and views read from them.
//!
//! - Each tile matches the CPU's stage 4 on the GPU's own stage 3 within one
//!   f16 step, for Bayer, X-Trans, linear and non-RAW sources.
//! - A tile is the same, bit for bit, rendered alone, with others, or after
//!   evictions.
//! - 1:1 is an exact copy of level 0 in every orientation, at pans that put
//!   tile boundaries inside the view; a Fit view larger than level 1 reads
//!   level 0 (decided 2026-10-19); a view the cache cannot hold falls back
//!   to level 1.
//! - Criterion 6: 1:1 through 1024 px tiles equals one tile, code for code.
//! - Criterion 7: Fit matches 1:1 downsampled, at default edits and with
//!   Highlights −100 and Shadows +100 (local tone mapping, one base layer
//!   for both), the latter on what the Fit view resolves.
//! - Criterion 8 per pixel at 1:1, on the decoder's own fixtures.

use std::sync::{Mutex, MutexGuard};

use genzo_color::output::Target;
use genzo_model::frame::Rect;
use genzo_pipeline::camera_color::{CameraTransform, camera_transform, encoded_transform};
use genzo_pipeline::gpu::base::BaseLayer;
use genzo_pipeline::gpu::level0::{Level0Cache, TILE};
use genzo_pipeline::gpu::raw_stages::{RawStages, TileConfig, Until};
use genzo_pipeline::gpu::synthetic::{scene_raw, scene_xtrans_raw};
use genzo_pipeline::gpu::view::{ViewStages, compresses, dithers};
use genzo_pipeline::gpu::{AdapterChoice, Gpu, GpuError};
use genzo_pipeline::look::{BaseCurve, Look, Profile};
use genzo_pipeline::raw::RawSource;
use genzo_pipeline::reference::sampling::{ViewSpec, sample, shown};
use genzo_pipeline::reference::stages::stage4;

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

fn transform(src: &RawSource) -> CameraTransform {
    match &src.encoding {
        Some(e) => encoded_transform(e),
        None => {
            let mut t = camera_transform(&src.color, None);
            // A white balance and a desaturating clip, so stage 4 shows.
            t.multipliers = [2.0, 1.0, 1.5];
            t
        }
    }
}

/// A non-RAW source: a 16-bit Display P3 gradient.
fn encoded(size: (u32, u32)) -> RawSource {
    let rgba: Vec<u16> = (0..size.0 * size.1)
        .flat_map(|i| {
            let (x, y) = (i % size.0, i / size.0);
            [
                x * 65535 / size.0,
                y * 65535 / size.1,
                (x * 7 + y * 3) % 65536,
                65535,
            ]
            .map(|v| v as u16)
        })
        .collect();
    RawSource::from_encoded(
        size,
        &rgba,
        &genzo_color::encoding::InputEncoding::display_p3(),
    )
    .unwrap()
}

fn linear(size: (u32, u32)) -> RawSource {
    // The Bayer scene's stored values, read as linear RGB, would need its
    // levels; a non-RAW source with a linear curve stands in for linear
    // input here, since stage 1 is not what is tested.
    let rgba: Vec<u16> = (0..size.0 * size.1)
        .flat_map(|i| {
            [
                (i * 13 % 65536) as u16,
                (i * 7 % 50000) as u16,
                20000,
                65535,
            ]
        })
        .collect();
    let enc = genzo_color::encoding::InputEncoding::nclx(9, 8).unwrap();
    RawSource::from_encoded(size, &rgba, &enc).unwrap()
}

fn sources(size: (u32, u32)) -> Vec<(&'static str, RawSource)> {
    vec![
        ("bayer", scene_raw(size.0, size.1)),
        ("x-trans", scene_xtrans_raw(size.0, size.1)),
        ("linear", linear(size)),
        ("display p3", encoded(size)),
    ]
}

/// Level 0 assembled from the cache's tiles, row by row.
fn level0_image(gpu: &Gpu, cache: &Level0Cache, size: (u32, u32)) -> Vec<[f32; 3]> {
    let mut out = vec![[f32::NAN; 3]; (size.0 * size.1) as usize];
    for (tx, ty) in Level0Cache::tiles_for(
        Rect {
            x: 0,
            y: 0,
            width: size.0,
            height: size.1,
        },
        size,
    ) {
        let tile = cache.read_tile(gpu, (tx, ty)).unwrap().expect("held");
        for y in 0..TILE.min(size.1 - ty * TILE) {
            for x in 0..TILE.min(size.0 - tx * TILE) {
                out[((ty * TILE + y) * size.0 + tx * TILE + x) as usize] =
                    tile[(y * TILE + x) as usize];
            }
        }
    }
    out
}

fn all_tiles(size: (u32, u32)) -> Vec<(u32, u32)> {
    Level0Cache::tiles_for(
        Rect {
            x: 0,
            y: 0,
            width: size.0,
            height: size.1,
        },
        size,
    )
}

fn f16_close(got: f32, want: f32) -> bool {
    let step = want.abs().max(6.2e-5) / 1024.0;
    (got - want).abs() <= step * 1.01 + 1e-8
}

#[test]
fn tiles_match_the_cpu_stage_4() {
    let Some((_lock, gpu)) = gpu() else { return };
    let size = (700, 530);
    for (name, src) in sources(size) {
        let t = transform(&src);
        let mut stages = RawStages::new(&gpu, TileConfig::default());
        let s3 = stages
            .render(&gpu, &src, t.multipliers, Until::Stage3)
            .unwrap();
        let mut cache = Level0Cache::new(&gpu);
        cache
            .ensure(&gpu, &mut stages, &src, &t, 1, &all_tiles(size))
            .unwrap();
        let got = level0_image(&gpu, &cache, size);
        let bad = got
            .iter()
            .zip(s3.chunks(3))
            .flat_map(|(g, c)| {
                let w = stage4([c[0], c[1], c[2]], &t);
                (0..3).map(move |k| (g[k], w[k]))
            })
            .filter(|(g, w)| !f16_close(*g, *w))
            .count();
        assert_eq!(bad, 0, "{name}");
    }
}

#[test]
fn a_tile_is_the_same_however_it_is_rendered() {
    let Some((_lock, gpu)) = gpu() else { return };
    let size = (900, 600);
    for (name, src) in sources(size) {
        let t = transform(&src);
        let mut stages = RawStages::new(&gpu, TileConfig::default());
        let mut whole = Level0Cache::new(&gpu);
        whole
            .ensure(&gpu, &mut stages, &src, &t, 1, &all_tiles(size))
            .unwrap();
        // A cache of 4 slots, filled one tile at a time and in pairs, so
        // tiles are evicted and rendered again.
        let mut small = Level0Cache::with_slots(&gpu, (2, 2));
        let tiles = all_tiles(size);
        for round in 0..2 {
            for (i, &tile) in tiles.iter().enumerate() {
                let want = if round == 0 {
                    vec![tile]
                } else {
                    vec![tile, tiles[(i + 5) % tiles.len()]]
                };
                small.ensure(&gpu, &mut stages, &src, &t, 1, &want).unwrap();
                let a = whole.read_tile(&gpu, tile).unwrap().unwrap();
                let b = small.read_tile(&gpu, tile).unwrap().unwrap();
                // Only the image's part of an edge tile is defined.
                let (w, h) = (
                    TILE.min(size.0 - tile.0 * TILE),
                    TILE.min(size.1 - tile.1 * TILE),
                );
                for y in 0..h {
                    for x in 0..w {
                        let k = (y * TILE + x) as usize;
                        assert_eq!(
                            a[k].map(f32::to_bits),
                            b[k].map(f32::to_bits),
                            "{name} tile {tile:?} round {round}"
                        );
                    }
                }
            }
        }
        assert!(small.held() <= 4);
    }
}

#[test]
fn one_to_one_is_a_copy_of_level_0() {
    let Some((_lock, gpu)) = gpu() else { return };
    let src = scene_raw(1030, 777);
    let size = src.size();
    let t = transform(&src);
    let mut stages = RawStages::new(&gpu, TileConfig::default());
    stages.render_pyramid(&gpu, &src, &t).unwrap();
    let mut cache = Level0Cache::new(&gpu);
    let mut view = ViewStages::new(&gpu);
    cache
        .ensure(&gpu, &mut stages, &src, &t, 7, &all_tiles(size))
        .unwrap();
    let level0 = level0_image(&gpu, &cache, size);
    for o in 1..=8u8 {
        let full = shown(size, o);
        // Pans that put tile boundaries inside the view, up to the far edge.
        for origin in [
            (0.0, 0.0),
            (37.0, 251.0),
            (f64::from(full.0 - 300), f64::from(full.1 - 200)),
        ] {
            let spec = ViewSpec::Region {
                out: (300, 200),
                zoom: 1.0,
                origin,
            };
            let s = view
                .show(&gpu, &mut stages, &mut cache, &src, 7, o, spec)
                .unwrap();
            assert_eq!(s.level, 0);
            let got = view.read_sampled(&gpu).unwrap();
            let want = sample(&level0, size, &s, (300, 200));
            let same = got
                .iter()
                .zip(&want)
                .all(|(a, b)| a.map(f32::to_bits) == b.map(f32::to_bits));
            assert!(same, "orientation {o}, origin {origin:?}");
        }
    }
}

#[test]
fn fit_above_level_1_reads_level_0_and_falls_back_when_full() {
    let Some((_lock, gpu)) = gpu() else { return };
    let src = scene_raw(1000, 800);
    let size = src.size();
    let t = transform(&src);
    let mut stages = RawStages::new(&gpu, TileConfig::default());
    stages.render_pyramid(&gpu, &src, &t).unwrap();
    let mut view = ViewStages::new(&gpu);
    // Level 1 is 500 x 400; a 700 x 560 Fit reads level 0.
    let mut cache = Level0Cache::new(&gpu);
    let s = view
        .show(
            &gpu,
            &mut stages,
            &mut cache,
            &src,
            1,
            1,
            ViewSpec::Fit((700, 560)),
        )
        .unwrap();
    assert_eq!(s.level, 0);
    let level0 = level0_image(&gpu, &cache, size);
    let want = sample(&level0, size, &s, (700, 560));
    let got = view.read_sampled(&gpu).unwrap();
    let step = |w: f32| 2.0 * w.abs().max(6.2e-5) / 1024.0 + 1e-5;
    assert!(
        got.iter()
            .flatten()
            .zip(want.iter().flatten())
            .all(|(g, w)| (g - w).abs() <= step(*w))
    );
    // A cache of 4 slots cannot hold the 16 tiles: level 1, upsampled.
    let mut small = Level0Cache::with_slots(&gpu, (2, 2));
    let s = view
        .show(
            &gpu,
            &mut stages,
            &mut small,
            &src,
            1,
            1,
            ViewSpec::Fit((700, 560)),
        )
        .unwrap();
    assert_eq!((s.level, small.held()), (1, 0));
}

#[test]
fn the_cache_keeps_tiles_until_the_key_changes() {
    let Some((_lock, gpu)) = gpu() else { return };
    let src = scene_raw(600, 600);
    let t = transform(&src);
    let mut stages = RawStages::new(&gpu, TileConfig::default());
    let mut cache = Level0Cache::new(&gpu);
    cache
        .ensure(&gpu, &mut stages, &src, &t, 1, &[(0, 0), (1, 1)])
        .unwrap();
    assert_eq!(cache.held(), 2);
    cache
        .ensure(&gpu, &mut stages, &src, &t, 1, &[(1, 1), (2, 2)])
        .unwrap();
    assert_eq!(cache.held(), 3);
    // A new white balance is a new key: the old tiles go.
    let mut warmer = t.clone();
    warmer.multipliers = [2.5, 1.0, 1.2];
    cache
        .ensure(&gpu, &mut stages, &src, &warmer, 2, &[(0, 0)])
        .unwrap();
    assert_eq!(cache.held(), 1);
    let a = cache.read_tile(&gpu, (0, 0)).unwrap().unwrap();
    let mut fresh = Level0Cache::new(&gpu);
    fresh
        .ensure(&gpu, &mut stages, &src, &warmer, 9, &[(0, 0)])
        .unwrap();
    let b = fresh.read_tile(&gpu, (0, 0)).unwrap().unwrap();
    assert!(
        a.iter()
            .zip(&b)
            .all(|(x, y)| x.map(f32::to_bits) == y.map(f32::to_bits))
    );
}

/// Criterion 6: a 3000 x 2000 image at 1:1, rendered with 1024 px tiles and
/// as one tile, gives the same frame.
#[test]
fn criterion_6_no_tile_seams() {
    let Some((_lock, gpu)) = gpu() else { return };
    let src = scene_raw(3000, 2000);
    let t = transform(&src);
    let mut frames = vec![];
    for config in [
        TileConfig::default(),
        TileConfig {
            tile: 4096,
            xtrans_tile: 4096,
            apron: 24,
        },
    ] {
        let mut stages = RawStages::new(&gpu, config);
        stages.render_pyramid(&gpu, &src, &t).unwrap();
        let mut cache = Level0Cache::new(&gpu);
        let mut view = ViewStages::new(&gpu);
        // Across the 1024 px tile boundary at x 1024 and y 1024.
        let spec = ViewSpec::Region {
            out: (640, 480),
            zoom: 1.0,
            origin: (800.0, 700.0),
        };
        view.show(&gpu, &mut stages, &mut cache, &src, 1, 1, spec)
            .unwrap();
        view.output(&gpu, Target::Srgb, true, true);
        frames.push(view.read_frame(&gpu).unwrap());
    }
    let off = frames[0]
        .iter()
        .zip(&frames[1])
        .filter(|(a, b)| a != b)
        .count();
    assert_eq!(off, 0, "{off} pixels differ");
}

fn lab_srgb(rgb: [u8; 4]) -> [f64; 3] {
    let lin = [0, 1, 2].map(|c| genzo_color::srgb_decode(f64::from(rgb[c]) / 255.0));
    genzo_color::xyz_to_lab(
        genzo_color::mul(&genzo_color::SRGB_TO_XYZ, lin),
        genzo_color::D65_WHITE,
    )
}

/// Criterion 7: the Fit frame against the 1:1 frame averaged down to Fit
/// (in linear light), within mean ΔE2000 1.0.
///
/// - At default edits, on the whole frame.
/// - With Highlights at −100 and Shadows at +100, where local tone mapping
///   shows, on the scene's gradients and patches (owner, 2026-10-20). The
///   band of fine lines below them (3.3 stops, an 8 px period) is texture
///   the Fit view cannot resolve: any non-linear tone diverges there, the
///   base curve alone measuring 2.95 over the whole frame.
#[test]
fn criterion_7_fit_matches_one_to_one() {
    let Some((_lock, gpu)) = gpu() else { return };
    let src = scene_raw(1024, 768);
    let k = 4;
    let fit = (src.size().0 / k, src.size().1 / k);
    // The lines start at 80 % of the height; Fit's Lanczos-2 reaches 2 px.
    let resolved = fit.1 * 8 / 10 - 2;
    let local = Look {
        highlights: -100.0,
        shadows: 100.0,
        ..Look::with_profile(Profile::GenzoStandard)
    };
    for (name, look, rows) in [
        ("default edits", None, fit.1),
        ("Highlights and Shadows", Some(local), resolved),
    ] {
        let mean = fit_against_one_to_one(&gpu, &src, look, k, rows);
        println!("{name}: Fit against 1:1 averaged down: mean ΔE2000 {mean:.3}");
        assert!(mean <= 1.0, "{name}: {mean}");
    }
}

/// Criterion 7's measure for `src` with `look` (stages 6 and 7, one base
/// layer for both views), or none (stage 9 on the sampled cache): Fit at
/// 1/`k`, over its first `rows` rows.
fn fit_against_one_to_one(
    gpu: &Gpu,
    src: &RawSource,
    look: Option<Look>,
    k: u32,
    rows: u32,
) -> f64 {
    let size = src.size();
    let t = transform(src);
    let mut stages = RawStages::new(gpu, TileConfig::default());
    stages.render_pyramid(gpu, src, &t).unwrap();
    let mut layer = BaseLayer::new(gpu);
    layer.build(gpu, stages.pyramid().unwrap(), size, 1);
    let curve = BaseCurve::new(&src.color);
    let mut cache = Level0Cache::new(gpu);
    let mut view = ViewStages::new(gpu);
    let mut render = |spec: ViewSpec| {
        view.show(gpu, &mut stages, &mut cache, src, 1, 1, spec)
            .unwrap();
        if let Some(look) = &look {
            view.tone(gpu, look, Some(&curve), Some(&layer));
            view.color(gpu, look);
        }
        view.output(gpu, Target::Srgb, true, false);
        view.read_frame(gpu).unwrap()
    };
    let full = render(ViewSpec::Region {
        out: size,
        zoom: 1.0,
        origin: (0.0, 0.0),
    });
    let fit = (size.0 / k, size.1 / k);
    let small = render(ViewSpec::Fit(fit));
    let mut total = 0.0;
    for y in 0..rows {
        for x in 0..fit.0 {
            let mut lin = [0.0; 3];
            for dy in 0..k {
                for dx in 0..k {
                    let p = full[((y * k + dy) * size.0 + x * k + dx) as usize];
                    for c in 0..3 {
                        lin[c] +=
                            genzo_color::srgb_decode(f64::from(p[c]) / 255.0) / f64::from(k * k);
                    }
                }
            }
            let down = lin.map(|v| (genzo_color::srgb_encode(v) * 255.0).round() as u8);
            total += genzo_color::delta_e_2000(
                lab_srgb([down[0], down[1], down[2], 255]),
                lab_srgb(small[(y * fit.0 + x) as usize]),
            );
        }
    }
    total / f64::from(fit.0 * rows)
}

/// Criterion 8 per pixel at 1:1: the decoder's own fixtures, unedited, come
/// back within 1 code of their decoded values (1.5 for 16-bit ones), sRGB
/// into an sRGB frame and Display P3 into a P3 frame.
#[test]
fn criterion_8_per_pixel_at_one_to_one() {
    let Some((_lock, gpu)) = gpu() else { return };
    let dir =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../imgio/tests/fixtures/images");
    let mut stages = RawStages::new(&gpu, TileConfig::default());
    let mut cache = Level0Cache::new(&gpu);
    let mut view = ViewStages::new(&gpu);
    let mut checked = 0;
    for (key, name) in [
        "jpeg-444.jpg",
        "jpeg-420.jpg",
        "png-rgb8.png",
        "png-rgba16.png",
        "png-gray2.png",
        "png-icc-lcms2-srgb.png",
        "png-cicp-p3.png",
        "tiff-rgb16-deflate-predictor.tif",
        "tiff-gray16-miniswhite.tif",
        "heic-p3-nclx.heic",
    ]
    .iter()
    .enumerate()
    {
        let img = genzo_imgio::decode(&std::fs::read(dir.join(name)).unwrap(), Default::default())
            .unwrap();
        let bits = img.source_bits;
        let genzo_imgio::Pixels::Rgba16 { data, color } = img.pixels.clone() else {
            panic!("{name}")
        };
        let target = match color {
            genzo_imgio::ColorEncoding::DisplayP3 => Target::DisplayP3,
            _ => Target::Srgb,
        };
        let src = RawSource::from_decoded(img).unwrap();
        let size = src.size();
        let t = transform(&src);
        stages.render_pyramid(&gpu, &src, &t).unwrap();
        let p = stages.pyramid().unwrap();
        let (compress, dither) = (compresses(p, target), dithers(p, false));
        assert!(!compress && !dither, "{name}");
        let spec = ViewSpec::Region {
            out: size,
            zoom: 1.0,
            origin: (0.0, 0.0),
        };
        view.show(
            &gpu,
            &mut stages,
            &mut cache,
            &src,
            key as u64 + 100,
            1,
            spec,
        )
        .unwrap();
        view.output(&gpu, target, compress, dither);
        let frame = view.read_frame(&gpu).unwrap();
        let mut worst = 0f64;
        for (f, d) in frame.iter().zip(data.as_chunks::<4>().0) {
            for c in 0..3 {
                worst = worst.max((f64::from(f[c]) - f64::from(d[c]) / 257.0).abs());
            }
        }
        let bound = if bits > 8 { 1.5 } else { 1.0 };
        println!("{name} ({bits}-bit, {target:?}): worst {worst:.3} codes");
        assert!(worst <= bound, "{name}: {worst} codes");
        checked += 1;
    }
    assert_eq!(checked, 10);
}

/// The stored golden of a 1:1 frame: the synthetic RAW scene at full
/// resolution, a 300 x 200 view across cache-tile boundaries (x 256,
/// y 256), through stage 9 into sRGB. Written on lavapipe.
#[test]
fn golden_one_to_one_frame() {
    let Some((_lock, gpu)) = gpu() else { return };
    let src = scene_raw(1030, 777);
    let t = transform(&src);
    let mut stages = RawStages::new(&gpu, TileConfig::default());
    stages.render_pyramid(&gpu, &src, &t).unwrap();
    let mut cache = Level0Cache::new(&gpu);
    let mut view = ViewStages::new(&gpu);
    let out = (300, 200);
    let spec = ViewSpec::Region {
        out,
        zoom: 1.0,
        origin: (180.0, 170.0),
    };
    view.show(&gpu, &mut stages, &mut cache, &src, 1, 1, spec)
        .unwrap();
    view.output(&gpu, Target::Srgb, true, true);
    let pixels = view
        .read_frame(&gpu)
        .unwrap()
        .iter()
        .map(|f| [0, 1, 2].map(|c| f32::from(f[c]) / 255.0))
        .collect();
    let case = genzo_golden::Case::new(
        env!("CARGO_MANIFEST_DIR"),
        "view/synthetic-one-to-one-srgb",
        genzo_golden::Space::DisplaySrgb,
        genzo_golden::Producer::Gpu {
            adapter: gpu.info.name.clone(),
        },
    );
    genzo_golden::check(&case, &genzo_golden::Image::new(out.0, out.1, pixels));
}
