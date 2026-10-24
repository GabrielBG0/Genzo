//! A prefetched neighbor's level 1, rendered tile by tile into CPU memory
//! beside the held pyramid, then uploaded on a switch (DEV-01, Prefetched
//! neighbors):
//!
//! - Uploaded, its pyramid equals [`RawStages::render_pyramid`]'s, bit for
//!   bit, at every level, Bayer and X-Trans, at odd sizes, built in one go
//!   or stopped between tiles and resumed, uploaded in one band or in many
//!   with a short last one.
//! - The held pyramid is left as it was while a neighbor builds.
//! - A neighbor's build, interrupted by a rebuild of the held pyramid of
//!   the other kind of mosaic or another X-Trans phase, still ends equal to
//!   its render.
//! - A white balance rebuild of the held pyramid, interrupted by a
//!   neighbor's build of the other kind of mosaic and size, still ends
//!   equal to a new pyramid.

use std::sync::{Mutex, MutexGuard};

use genzo_color::camera::xy_from_temperature;
use genzo_pipeline::camera_color::camera_transform;
use genzo_pipeline::gpu::raw_stages::{Level1, RawStages, TileConfig};
use genzo_pipeline::gpu::synthetic::{scene_raw, scene_xtrans_raw, with_camera_profile};
use genzo_pipeline::gpu::{AdapterChoice, Gpu, GpuError};
use genzo_pipeline::raw::{Cfa, RawSource};

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

/// Fine tiles, so an image has many and a texel placed wrong within or
/// across tiles shows.
const FINE: TileConfig = TileConfig {
    tile: 32,
    xtrans_tile: 32,
    apron: 24,
};

fn bayer() -> RawSource {
    with_camera_profile(scene_raw(1030, 777))
}

fn xtrans() -> RawSource {
    with_camera_profile(scene_xtrans_raw(611, 409))
}

/// `src` with its X-Trans pattern a row and a column on: the same mosaic
/// at another phase.
fn shifted(mut src: RawSource) -> RawSource {
    let Some(Cfa::XTrans(x)) = src.cfa else {
        panic!("an X-Trans source")
    };
    src.cfa = Some(Cfa::XTrans(std::array::from_fn(|r| {
        std::array::from_fn(|c| x[(r + 1) % 6][(c + 1) % 6])
    })));
    src
}

/// Every level of the pyramid `stages` holds.
fn levels(gpu: &Gpu, stages: &RawStages) -> Vec<Vec<[f32; 3]>> {
    let p = stages.pyramid().unwrap();
    (1..=p.levels)
        .map(|l| p.read_level(gpu, l).unwrap())
        .collect()
}

fn assert_same(got: &[Vec<[f32; 3]>], want: &[Vec<[f32; 3]>], what: &str) {
    assert_eq!(got.len(), want.len(), "{what}: levels");
    for (l, (g, w)) in got.iter().zip(want).enumerate() {
        let off = g.iter().zip(w).filter(|(a, b)| a != b).count();
        assert_eq!(off, 0, "{what}: level {} differs at {off} texels", l + 1);
    }
}

/// `src`'s level 1 built on `stages`, stopped after each `stop_every`
/// tiles when given.
fn build(gpu: &Gpu, stages: &mut RawStages, src: &RawSource, stop_every: Option<usize>) -> Level1 {
    let white = camera_transform(&src.color, Some(xy_from_temperature(4300.0, -7.0)));
    let mut b = stages.start_level1(gpu, src, &white);
    let tiles = b.left();
    assert!(tiles > 20, "{tiles} tiles");
    let mut resumes = 0;
    loop {
        let mut asked = 0;
        let done = stages
            .continue_level1(gpu, src, &mut b, || {
                asked += 1;
                stop_every.is_some_and(|n| asked > n)
            })
            .unwrap();
        if done {
            break;
        }
        resumes += 1;
        assert_eq!(b.left(), tiles - resumes * stop_every.unwrap());
    }
    if let Some(n) = stop_every {
        assert_eq!(resumes, (tiles - 1) / n);
    }
    b.finish()
}

#[test]
fn an_uploaded_level1_is_the_rendered_pyramid() {
    let Some((_lock, gpu)) = gpu() else { return };
    for (name, src) in [("Bayer", bayer()), ("X-Trans", xtrans())] {
        let mut fresh = RawStages::new(&gpu, FINE);
        let white = camera_transform(&src.color, Some(xy_from_temperature(4300.0, -7.0)));
        fresh.render_pyramid(&gpu, &src, &white).unwrap();
        let want = levels(&gpu, &fresh);
        for (case, stop_every, band_rows) in [
            ("in one go, one band", None, None),
            ("stopped every 7 tiles, bands of 7 rows", Some(7), Some(7)),
        ] {
            let what = format!("{name}, {case}");
            let mut stages = RawStages::new(&gpu, FINE);
            let level1 = build(&gpu, &mut stages, &src, stop_every);
            if let Some(rows) = band_rows {
                assert_ne!(level1.size.1 % rows, 0, "{what}: a short last band");
                stages.upload_band = level1.size.0 as usize * 8 * rows as usize;
            }
            assert_eq!(level1.size, fresh.pyramid().unwrap().size, "{what}");
            assert_eq!(level1.bytes(), want[0].len() as u64 * 8, "{what}");
            let p = stages.upload_pyramid(&gpu, &level1).unwrap();
            assert_eq!(p.transform.multipliers, white.multipliers, "{what}");
            assert_eq!(p.levels, fresh.pyramid().unwrap().levels, "{what}");
            assert_same(&levels(&gpu, &stages), &want, &what);
        }
    }
}

#[test]
fn a_neighbor_builds_beside_the_held_pyramid() {
    let Some((_lock, gpu)) = gpu() else { return };
    let (current, neighbor) = (bayer(), xtrans());
    let mut stages = RawStages::new(&gpu, FINE);
    let white = camera_transform(&current.color, None);
    stages.render_pyramid(&gpu, &current, &white).unwrap();
    let held = levels(&gpu, &stages);
    let level1 = build(&gpu, &mut stages, &neighbor, Some(5));
    assert_same(&levels(&gpu, &stages), &held, "the held pyramid");

    // On the switch the neighbor's pyramid replaces it.
    let mut fresh = RawStages::new(&gpu, FINE);
    fresh
        .render_pyramid(&gpu, &neighbor, &level1.transform)
        .unwrap();
    stages.upload_pyramid(&gpu, &level1).unwrap();
    assert_same(
        &levels(&gpu, &stages),
        &levels(&gpu, &fresh),
        "after the switch",
    );
}

#[test]
fn a_rebuild_interrupted_by_a_neighbor_still_ends_as_a_new_pyramid() {
    let Some((_lock, gpu)) = gpu() else { return };
    for (name, current, neighbor) in [
        ("Bayer, an X-Trans neighbor", bayer(), xtrans()),
        ("X-Trans, a Bayer neighbor", xtrans(), bayer()),
        // Another phase of the pattern: other hexagons in the shared
        // X-Trans uniform.
        ("X-Trans, an X-Trans neighbor", xtrans(), shifted(xtrans())),
    ] {
        let old = camera_transform(&current.color, None);
        let new = camera_transform(&current.color, Some(xy_from_temperature(3400.0, 12.0)));
        let mut fresh = RawStages::new(&gpu, FINE);
        fresh.render_pyramid(&gpu, &current, &new).unwrap();
        let want = levels(&gpu, &fresh);

        let mut stages = RawStages::new(&gpu, FINE);
        stages.render_pyramid(&gpu, &current, &old).unwrap();
        let mut rebuild = stages.rebuild_first(&gpu, &current, &new, None).unwrap();
        let mut asked = 0;
        assert!(
            !stages
                .rebuild_rest(&gpu, &current, &mut rebuild, || {
                    asked += 1;
                    asked > 9
                })
                .unwrap()
        );
        build(&gpu, &mut stages, &neighbor, None);
        assert!(
            stages
                .rebuild_rest(&gpu, &current, &mut rebuild, || false)
                .unwrap()
        );
        assert_same(&levels(&gpu, &stages), &want, name);
    }
}

#[test]
fn a_neighbor_interrupted_by_a_rebuild_still_ends_as_its_render() {
    let Some((_lock, gpu)) = gpu() else { return };
    for (name, current, neighbor) in [
        ("an X-Trans neighbor, a Bayer rebuild", bayer(), xtrans()),
        ("a Bayer neighbor, an X-Trans rebuild", xtrans(), bayer()),
        (
            "an X-Trans neighbor, another phase's rebuild",
            shifted(xtrans()),
            xtrans(),
        ),
    ] {
        let white = camera_transform(&neighbor.color, Some(xy_from_temperature(4300.0, -7.0)));
        let mut fresh = RawStages::new(&gpu, FINE);
        fresh.render_pyramid(&gpu, &neighbor, &white).unwrap();
        let want = levels(&gpu, &fresh);

        let mut stages = RawStages::new(&gpu, FINE);
        let old = camera_transform(&current.color, None);
        let new = camera_transform(&current.color, Some(xy_from_temperature(3400.0, 12.0)));
        stages.render_pyramid(&gpu, &current, &old).unwrap();
        let mut b = stages.start_level1(&gpu, &neighbor, &white);
        let mut asked = 0;
        assert!(
            !stages
                .continue_level1(&gpu, &neighbor, &mut b, || {
                    asked += 1;
                    asked > 9
                })
                .unwrap()
        );
        // The current photo's work in between, as a request would bring.
        let mut rebuild = stages.rebuild_first(&gpu, &current, &new, None).unwrap();
        assert!(
            stages
                .rebuild_rest(&gpu, &current, &mut rebuild, || false)
                .unwrap()
        );
        assert!(
            stages
                .continue_level1(&gpu, &neighbor, &mut b, || false)
                .unwrap()
        );
        stages.upload_pyramid(&gpu, &b.finish()).unwrap();
        assert_same(&levels(&gpu, &stages), &want, name);
    }
}
