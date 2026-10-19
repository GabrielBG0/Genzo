//! The view on the GPU (DEV-01): the pyramid sampled into it, then stage 9
//! into an RGBA8 frame.
//!
//! - Sampling matches `reference::sampling` within an f16 step, at Fit
//!   scales and in every orientation; a view the size of a level is an
//!   exact copy.
//! - The frame matches `color::output` from the same sampled values, the
//!   gamut compression and the dither included, within 1 code.
//! - Criterion 8, end to end: unedited sRGB (8 and 16 bit) into an sRGB
//!   frame and Display P3 into a P3 frame come back within 1 code, from
//!   images of 2 x 2 blocks so a level 1 texel is one source color.
//! - Criterion 9 on the GPU: out-of-gamut AP1 colors keep their hue.

use std::sync::{Mutex, MutexGuard};

use genzo_color::encoding::InputEncoding;
use genzo_color::output::{Target, pixel};
use genzo_pipeline::camera_color::{camera_transform, encoded_transform};
use genzo_pipeline::gpu::raw_stages::{Pyramid, RawStages, TileConfig};
use genzo_pipeline::gpu::synthetic::{blocks, scene_raw};
use genzo_pipeline::gpu::view::{ViewStages, compresses, dithers};
use genzo_pipeline::gpu::{AdapterChoice, Gpu, GpuError};
use genzo_pipeline::raw::RawSource;
use genzo_pipeline::reference::sampling::{sample, shown};

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

fn pyramid<'a>(gpu: &Gpu, stages: &'a mut RawStages, src: &RawSource) -> &'a Pyramid {
    let t = match &src.encoding {
        Some(e) => encoded_transform(e),
        None => camera_transform(&src.color, None),
    };
    stages.render_pyramid(gpu, src, &t).unwrap()
}

/// Within two f16 steps, plus 1e-5: the GPU's f32 Lanczos weights are off
/// by about 1e-6, which shows where bright neighbors' lobes cancel to a dark
/// value (measured: 2.3e-6 at 1.5e-4, upsampling level 1).
fn f16_close(got: f32, want: f32) -> bool {
    let step = want.abs().max(6.2e-5) / 1024.0;
    (got - want).abs() <= 2.0 * step + 1e-5
}

#[test]
fn sampling_matches_the_reference() {
    let Some((_lock, gpu)) = gpu() else { return };
    let src = scene_raw(1030, 777);
    let mut stages = RawStages::new(&gpu, TileConfig::default());
    let p = pyramid(&gpu, &mut stages, &src);
    let mut view = ViewStages::new(&gpu);
    for o in 1..=8u8 {
        for out in [(500, 375), (333, 251), (257, 194), (130, 98), (700, 520)] {
            let out = shown(out, o);
            let s = view.sample(&gpu, p, o, out);
            let level = p.read_level(&gpu, s.level).unwrap();
            let want = sample(&level, s.level_size, &s, out);
            let got = view.read_sampled(&gpu).unwrap();
            let bad = got
                .iter()
                .flatten()
                .zip(want.iter().flatten())
                .enumerate()
                .filter(|(_, (g, w))| !f16_close(**g, **w))
                .map(|(i, (g, w))| (i / 3, *g, *w))
                .collect::<Vec<_>>();
            assert!(
                bad.is_empty(),
                "orientation {o}, {out:?} from level {}: {:?}",
                s.level,
                &bad[..bad.len().min(4)]
            );
        }
    }
}

#[test]
fn a_view_the_size_of_a_level_is_a_copy() {
    let Some((_lock, gpu)) = gpu() else { return };
    let src = scene_raw(1030, 777);
    let mut stages = RawStages::new(&gpu, TileConfig::default());
    let p = pyramid(&gpu, &mut stages, &src);
    let mut view = ViewStages::new(&gpu);
    for level in 1..=p.levels {
        let size = p.level_size(level);
        let want = p.read_level(&gpu, level).unwrap();
        for o in [1u8, 3, 6] {
            let out = shown(size, o);
            let s = view.sample(&gpu, p, o, out);
            assert_eq!(s.level, level);
            let got = view.read_sampled(&gpu).unwrap();
            let cpu = sample(&want, size, &s, out);
            assert!(
                got.iter()
                    .zip(&cpu)
                    .all(|(a, b)| a.map(f32::to_bits) == b.map(f32::to_bits)),
                "level {level}, orientation {o}"
            );
        }
    }
}

/// The frame against `color::output::pixel` on the GPU's own sampled
/// values: how many channels differ, and by at most how much.
fn frame_against_cpu(
    gpu: &Gpu,
    view: &mut ViewStages,
    target: Target,
    compress: bool,
) -> (usize, i32) {
    view.output(gpu, target, compress, true);
    let frame = view.read_frame(gpu).unwrap();
    let sampled = view.read_sampled(gpu).unwrap();
    let width = view.size().0;
    let (mut off, mut worst) = (0, 0);
    for (i, (f, s)) in frame.iter().zip(&sampled).enumerate() {
        let (x, y) = (i as u32 % width, i as u32 / width);
        let want = pixel(s.map(f64::from), (x, y), target, compress, true);
        for c in 0..3 {
            let d = (i32::from(f[c]) - i32::from(want[c])).abs();
            if d > 0 {
                off += 1;
            }
            worst = worst.max(d);
        }
        assert_eq!(f[3], 255);
    }
    (off, worst)
}

#[test]
fn the_frame_matches_the_reference() {
    let Some((_lock, gpu)) = gpu() else { return };
    // Saturated BT.2020 colors, far outside sRGB and Display P3.
    let colors: Vec<[u16; 3]> = (0..4096u32)
        .map(|i| [(i % 16) * 4369, (i / 16 % 16) * 4369, (i / 256) * 4369].map(|v| v as u16))
        .collect();
    let (size, rgba) = blocks(&colors, 64);
    let enc = InputEncoding::nclx(9, 13).unwrap();
    let src = RawSource::from_encoded(size, &rgba, &enc).unwrap();
    let mut stages = RawStages::new(&gpu, TileConfig::default());
    let p = pyramid(&gpu, &mut stages, &src);
    let mut view = ViewStages::new(&gpu);
    for out in [p.size, (40, 37)] {
        view.sample(&gpu, p, 1, out);
        for target in [Target::Srgb, Target::DisplayP3] {
            for compress in [false, true] {
                let (off, worst) = frame_against_cpu(&gpu, &mut view, target, compress);
                let n = out.0 * out.1 * 3;
                println!(
                    "{out:?} {target:?}, compress {compress}: {off} of {n} channels off, by at most {worst}"
                );
                assert!(worst <= 1, "{target:?} {compress}: {worst} codes");
                assert!(
                    off * 200 < n as usize,
                    "{target:?} {compress}: {off} of {n}"
                );
            }
        }
    }
}

/// Criterion 8's colors: every combination of a few 8-bit levels, the
/// primaries, near-black and near-white among them; or spread 16-bit codes.
fn colors(sixteen: bool) -> Vec<[u16; 3]> {
    if !sixteen {
        let levels = [0u16, 1, 2, 5, 16, 64, 128, 200, 250, 254, 255];
        let mut out = vec![];
        for r in levels {
            for g in levels {
                for b in levels {
                    out.push([r, g, b].map(|v| v * 257));
                }
            }
        }
        out
    } else {
        let mut out = vec![
            [0; 3],
            [65535; 3],
            [65535, 0, 0],
            [0, 65535, 0],
            [0, 0, 65535],
        ];
        let mut x = 0x2545_f491u32;
        while out.len() < 1331 {
            let mut next = || {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                (x >> 16) as u16
            };
            out.push([next(), next(), next()]);
        }
        out
    }
}

/// Criterion 8: an unedited non-RAW photo is not dithered (decided
/// 2026-10-19), so the frame shows its codes, up to the pyramid's f16
/// (11 bits: a channel near 0 beside a bright one moves up to 1.3 codes
/// before rounding). 8-bit sources come back within 1 code; a 16-bit one,
/// whose values lie between codes, within 1.5.
#[test]
fn criterion_8_unedited_non_raw_is_an_identity() {
    let Some((_lock, gpu)) = gpu() else { return };
    let mut stages = RawStages::new(&gpu, TileConfig::default());
    let mut view = ViewStages::new(&gpu);
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let lcms2 =
        std::fs::read(dir.join("imgio/tests/fixtures/images/png-icc-lcms2-srgb.icc")).unwrap();
    // The GIMP sRGB profile in libheif's test HEIC.
    let heic = genzo_imgio::decode(
        &std::fs::read(dir.join("heif/vendor/libheif/tests/data/rainbow-451x461.heic")).unwrap(),
        Default::default(),
    )
    .unwrap();
    let genzo_imgio::Pixels::Rgba16 {
        color: genzo_imgio::ColorEncoding::Icc(gimp),
        ..
    } = heic.pixels
    else {
        panic!("the HEIC carries an ICC profile")
    };
    let p3_icc = moxcms::ColorProfile::new_display_p3().encode().unwrap();
    let icc = |bytes: &[u8]| InputEncoding::icc(bytes).unwrap();
    for (name, enc, target, sixteen, bound) in [
        (
            "sRGB JPEG or PNG",
            InputEncoding::srgb(),
            Target::Srgb,
            false,
            1.0,
        ),
        (
            "sRGB PNG, lcms2's profile",
            icc(&lcms2),
            Target::Srgb,
            false,
            1.0,
        ),
        (
            "sRGB HEIC, GIMP's profile",
            icc(&gimp),
            Target::Srgb,
            false,
            1.0,
        ),
        (
            "Display P3, an ICC profile",
            icc(&p3_icc),
            Target::DisplayP3,
            false,
            1.0,
        ),
        (
            "16-bit sRGB TIFF",
            InputEncoding::srgb(),
            Target::Srgb,
            true,
            1.5,
        ),
        (
            "Display P3 HEIC",
            InputEncoding::display_p3(),
            Target::DisplayP3,
            false,
            1.0,
        ),
    ] {
        let c = colors(sixteen);
        let (size, rgba) = blocks(&c, 77);
        let src = RawSource::from_encoded(size, &rgba, &enc).unwrap();
        let p = pyramid(&gpu, &mut stages, &src);
        let compress = compresses(p, target);
        assert!(!compress, "{name}: inside its target, so no compression");
        assert!(!dithers(p, false) && dithers(p, true));
        view.sample(&gpu, p, 1, p.size);
        view.output(&gpu, target, compress, dithers(p, false));
        let frame = view.read_frame(&gpu).unwrap();
        let (mut worst, mut over) = (0f64, 0);
        for (i, want) in c.iter().enumerate() {
            for ch in 0..3 {
                let d = (f64::from(frame[i][ch]) - f64::from(want[ch]) / 257.0).abs();
                worst = worst.max(d);
                if d > 1.0 {
                    over += 1;
                }
            }
        }
        println!("{name}: worst {worst:.3} codes, {over} channels over 1");
        assert!(
            worst <= bound,
            "{name}: {worst} codes ({over} channels over 1)"
        );
    }
}

#[test]
fn compressed_colors_keep_their_hue_on_the_gpu() {
    let Some((_lock, gpu)) = gpu() else { return };
    use genzo_color::output::{oklch, xyz_to_oklab};
    // AP1 primaries and secondaries at a few brightnesses: through linear
    // AP1 sources (16-bit codes of a linear encoding).
    let mut colors = vec![];
    for k in [0.1f64, 0.3, 0.6, 0.9] {
        for c in [
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 1.0],
            [1.0, 0.0, 1.0],
            [1.0, 0.3, 0.1],
        ] {
            colors.push(c.map(|v: f64| (v * k * 65535.0).round() as u16));
        }
    }
    let mut enc = InputEncoding::nclx(1, 8).unwrap();
    enc.to_working = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    let (size, rgba) = blocks(&colors, colors.len());
    let src = RawSource::from_encoded(size, &rgba, &enc).unwrap();
    let mut stages = RawStages::new(&gpu, TileConfig::default());
    let p = pyramid(&gpu, &mut stages, &src);
    let mut view = ViewStages::new(&gpu);
    for target in [Target::Srgb, Target::DisplayP3] {
        view.sample(&gpu, p, 1, p.size);
        view.output(&gpu, target, true, true);
        let frame = view.read_frame(&gpu).unwrap();
        let sampled = view.read_sampled(&gpu).unwrap();
        let mut worst = 0f64;
        for (f, s) in frame.iter().zip(&sampled) {
            let before = genzo_color::mul(&genzo_color::WORKING_TO_XYZ, s.map(f64::from));
            let after = genzo_color::mul(
                &target.to_xyz(),
                [0, 1, 2].map(|c| genzo_color::srgb_decode(f64::from(f[c]) / 255.0)),
            );
            let (a, b) = (oklch(xyz_to_oklab(before)), oklch(xyz_to_oklab(after)));
            if a[0] < 0.98 && b[1] > 0.05 {
                worst = worst.max(((a[2] - b[2] + 540.0) % 360.0 - 180.0).abs());
            }
        }
        println!("{target:?}: worst hue shift {worst:.2} degrees, quantization included");
        assert!(worst <= 3.0, "{target:?}: {worst}");
    }
}

/// Criterion 8's Adobe RGB part, end to end: the decoder's Adobe RGB JPEG
/// into an sRGB frame against moxcms's conversion (into linear AP1, which
/// holds Adobe RGB, so moxcms clips nothing; reduced the same way; to sRGB
/// and encoded). Colors the gamut compression keeps (below its threshold)
/// match within ΔE2000 0.5; the rest move inward by design.
#[test]
fn an_adobe_rgb_jpeg_matches_moxcms_in_an_srgb_frame() {
    let Some((_lock, gpu)) = gpu() else { return };
    use genzo_color::output::{max_chroma, xyz_to_oklab};
    use genzo_pipeline::reference::stages::reduce;
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../imgio/tests/fixtures/images/jpeg-adobe-rgb-exif.jpg");
    let img = genzo_imgio::decode(&std::fs::read(path).unwrap(), Default::default()).unwrap();
    let (w, h) = (img.width, img.height);
    let genzo_imgio::Pixels::Rgba16 { data, .. } = img.pixels.clone() else {
        panic!()
    };
    let src = RawSource::from_decoded(img).unwrap();
    let mut stages = RawStages::new(&gpu, TileConfig::default());
    let p = pyramid(&gpu, &mut stages, &src);
    assert!(compresses(p, Target::Srgb));
    let mut view = ViewStages::new(&gpu);
    view.sample(&gpu, p, 1, p.size);
    view.output(&gpu, Target::Srgb, true, dithers(p, false));
    let frame = view.read_frame(&gpu).unwrap();
    let mut ap1 = moxcms::ColorProfile::new_srgb();
    ap1.update_rgb_colorimetry(
        moxcms::XyY {
            x: 0.3127,
            y: 0.3290,
            yb: 1.0,
        },
        moxcms::ColorPrimaries::ACES_CG,
    );
    let one = moxcms::ToneReprCurve::Parametric(vec![1.0]);
    ap1.red_trc = Some(one.clone());
    ap1.green_trc = Some(one.clone());
    ap1.blue_trc = Some(one);
    let input: Vec<f32> = data
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|p| [p[0], p[1], p[2]].map(|v| f32::from(v) / 65535.0))
        .collect();
    let mut out = vec![0f32; input.len()];
    moxcms::ColorProfile::new_adobe_rgb()
        .create_transform_f32(
            moxcms::Layout::Rgb,
            &ap1,
            moxcms::Layout::Rgb,
            moxcms::TransformOptions {
                allow_use_cicp_transfer: false,
                ..Default::default()
            },
        )
        .unwrap()
        .transform(&input, &mut out)
        .unwrap();
    let (want, size) = reduce(&out, (w, h), 3);
    assert_eq!(size, p.size);
    let target = Target::Srgb;
    let to_srgb = target.from_working();
    let threshold = target.compression().threshold;
    let (mut kept, mut moved, mut n_kept, mut n_moved) = (0f64, 0f64, 0, 0);
    for (f, m) in frame.iter().zip(want.chunks(3)) {
        let rgb = genzo_color::mul(&to_srgb, [0, 1, 2].map(|c| f64::from(m[c])));
        let lab = xyz_to_oklab(genzo_color::mul(&target.to_xyz(), rgb));
        let chroma = lab[1].hypot(lab[2]);
        let r = if chroma > 1e-9 && lab[0] > 0.0 && lab[0] < 1.0 {
            chroma / max_chroma(lab[0], [lab[1] / chroma, lab[2] / chroma], target)
        } else {
            0.0
        };
        let encoded = rgb.map(|v| genzo_color::srgb_encode(v.clamp(0.0, 1.0)));
        let got = [0, 1, 2].map(|c| f64::from(f[c]) / 255.0);
        let de = genzo_color::delta_e_2000_srgb(got, encoded);
        if r < threshold {
            kept = kept.max(de);
            n_kept += 1;
        } else {
            moved = moved.max(de);
            n_moved += 1;
        }
    }
    println!(
        "kept by the compression: {n_kept} texels, ΔE2000 at most {kept:.3}; compressed: {n_moved}, at most {moved:.2}"
    );
    assert!(n_kept > 50 && n_moved > 0);
    assert!(kept < 0.5, "ΔE {kept}");
}

/// The stored golden of a frame: the synthetic RAW scene sampled to a Fit
/// view between two levels, through stage 9 into sRGB with the gamut
/// compression and the dither. Written on lavapipe.
#[test]
fn golden_fit_frame() {
    let Some((_lock, gpu)) = gpu() else { return };
    let src = scene_raw(1536, 1024);
    let mut stages = RawStages::new(&gpu, TileConfig::default());
    let p = pyramid(&gpu, &mut stages, &src);
    let mut view = ViewStages::new(&gpu);
    let out = (300, 200);
    view.sample(&gpu, p, 1, out);
    view.output(
        &gpu,
        Target::Srgb,
        compresses(p, Target::Srgb),
        dithers(p, false),
    );
    let frame = view.read_frame(&gpu).unwrap();
    let pixels = frame
        .iter()
        .map(|f| [0, 1, 2].map(|c| f32::from(f[c]) / 255.0))
        .collect();
    let case = genzo_golden::Case::new(
        env!("CARGO_MANIFEST_DIR"),
        "view/synthetic-fit-srgb",
        genzo_golden::Space::DisplaySrgb,
        genzo_golden::Producer::Gpu {
            adapter: gpu.info.name.clone(),
        },
    );
    genzo_golden::check(&case, &genzo_golden::Image::new(out.0, out.1, pixels));
}
