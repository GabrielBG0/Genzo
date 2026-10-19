//! Non-RAW images through stages 1 to 4 on the GPU (DEV-01, stage 4): each
//! code through its encoding's curve, its matrix into the working space,
//! reduced into the pyramid.
//!
//! - Tiled equals one tile, bit for bit, at every level.
//! - Each level matches the CPU reference within one f16 step.
//! - Round trip (criterion 8): images of 2 x 2 blocks, so a level 1 texel is
//!   one source color, come back to their codes within 1 (8-bit) code when
//!   the texel is taken back into the source's own space, saturated
//!   primaries and near-black included, from 8-bit and 16-bit codes.
//! - Neutrals stay neutral: a gray block's working-space channels agree.
//! - The decoder's own fixtures (imgio/tests/fixtures/images) match moxcms,
//!   an independent color engine, within ΔE2000 0.5.

use std::sync::{Mutex, MutexGuard};

use genzo_color::encoding::{CODES, EncodingError, InputEncoding};
use genzo_color::{D65_WHITE, WORKING_TO_XYZ, delta_e_2000, invert, mul, xyz_to_lab};
use genzo_imgio::{ColorEncoding, DecodeError, DecodeOptions, Pixels, decode};
use genzo_pipeline::camera_color::encoded_transform;
use genzo_pipeline::gpu::raw_stages::{RawStages, TileConfig, Until};
use genzo_pipeline::gpu::{AdapterChoice, Gpu, GpuError};
use genzo_pipeline::raw::{RawError, RawSource, input_encoding};
use genzo_pipeline::reference::stages::{reduce, stage1, stage2, stage4};

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

const ONE_TILE: TileConfig = TileConfig {
    tile: 4096,
    xtrans_tile: 4096,
    apron: 24,
};
const SMALL: TileConfig = TileConfig {
    tile: 64,
    xtrans_tile: 60,
    apron: 24,
};

/// Every encoding a decoder hands over, as one image each: the four
/// shortcuts and NCLX by formula, an RGB and a gray ICC profile. The flag:
/// whether the curve has a linear toe. A pure power curve (Adobe RGB, gamma
/// 2.2) has an infinite slope at black, where any rounding moves the code
/// far, so criterion 8 asks a ΔE of those, not codes.
fn encodings() -> Vec<(&'static str, InputEncoding, bool)> {
    let icc = |p: moxcms::ColorProfile| InputEncoding::icc(&p.encode().unwrap()).unwrap();
    vec![
        ("sRGB", InputEncoding::srgb(), true),
        ("Display P3", InputEncoding::display_p3(), true),
        ("Adobe RGB", InputEncoding::adobe_rgb(), false),
        (
            "NCLX BT.2020, BT.709 transfer",
            InputEncoding::nclx(9, 1).unwrap(),
            true,
        ),
        (
            "ICC Display P3",
            icc(moxcms::ColorProfile::new_display_p3()),
            true,
        ),
        (
            "ICC gray 2.2",
            icc(moxcms::ColorProfile::new_gray_with_gamma(2.2)),
            false,
        ),
    ]
}

/// Source colors: every combination of a few 8-bit levels (primaries,
/// near-black and near-white among them), widened as imgio widens them; or
/// spread 16-bit codes with the same extremes.
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
            [1, 1, 1],
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

/// An image of 2 x 2 blocks, one per color, `across` blocks per row: RGBA.
fn blocks(colors: &[[u16; 3]], across: usize) -> ((u32, u32), Vec<u16>) {
    let down = colors.len().div_ceil(across);
    let (w, h) = (2 * across, 2 * down);
    let mut rgba = vec![0u16; w * h * 4];
    for (i, c) in colors.iter().enumerate() {
        let (bx, by) = (2 * (i % across), 2 * (i / across));
        for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
            let o = ((by + dy) * w + bx + dx) * 4;
            rgba[o..o + 3].copy_from_slice(c);
            rgba[o + 3] = 65535;
        }
    }
    ((w as u32, h as u32), rgba)
}

fn source(enc: &InputEncoding, sixteen: bool) -> (Vec<[u16; 3]>, RawSource) {
    let c = colors(sixteen);
    // 77 blocks across: 154 px, an odd number of 64 px tiles' worth.
    let (size, rgba) = blocks(&c, 77);
    (c, RawSource::from_encoded(size, &rgba, enc).unwrap())
}

fn levels(gpu: &Gpu, config: TileConfig, src: &RawSource) -> Vec<Vec<[f32; 3]>> {
    let mut stages = RawStages::new(gpu, config);
    let t = encoded_transform(src.encoding.as_ref().unwrap());
    let p = stages.render_pyramid(gpu, src, &t).unwrap();
    (1..=p.levels)
        .map(|l| p.read_level(gpu, l).unwrap())
        .collect()
}

#[test]
fn tiles_equal_one_tile_bit_for_bit() {
    let Some((_lock, gpu)) = gpu() else { return };
    for (name, enc, _) in encodings() {
        for sixteen in [false, true] {
            let (_, src) = source(&enc, sixteen);
            let whole = levels(&gpu, ONE_TILE, &src);
            let tiled = levels(&gpu, SMALL, &src);
            for (l, (a, b)) in whole.iter().zip(&tiled).enumerate() {
                let first = a
                    .iter()
                    .zip(b)
                    .position(|(x, y)| x.map(f32::to_bits) != y.map(f32::to_bits));
                assert_eq!(first, None, "{name} level {}", l + 1);
            }
        }
    }
}

fn f16_close(got: f32, want: f32) -> bool {
    let step = want.abs().max(6.2e-5) / 1024.0;
    (got - want).abs() <= step * 1.01 + 1e-8
}

#[test]
fn levels_match_the_cpu_reference() {
    let Some((_lock, gpu)) = gpu() else { return };
    for (name, enc, _) in encodings() {
        for sixteen in [false, true] {
            let (_, src) = source(&enc, sixteen);
            let t = encoded_transform(&enc);
            // Stages 1 to 3 are a lookup: the GPU's equals the CPU's exactly.
            let mut want3 = stage1(&src);
            stage2(&mut want3, &src, t.multipliers);
            let mut stages = RawStages::new(&gpu, SMALL);
            let got3 = stages
                .render(&gpu, &src, t.multipliers, Until::Stage3)
                .unwrap();
            assert!(
                got3.iter()
                    .zip(&want3)
                    .all(|(a, b)| a.to_bits() == b.to_bits()),
                "{name}"
            );
            let s4: Vec<f32> = want3
                .chunks(3)
                .flat_map(|c| stage4([c[0], c[1], c[2]], &t))
                .collect();
            let p = stages.render_pyramid(&gpu, &src, &t).unwrap();
            assert_eq!(p.encoding.as_ref(), Some(&enc), "{name}");
            let (mut above, mut above_size) = (s4, src.size());
            for level in 1..=p.levels {
                let (want, want_size) = reduce(&above, above_size, 3);
                assert_eq!(want_size, p.level_size(level));
                let got = p.read_level(&gpu, level).unwrap();
                let bad = got
                    .iter()
                    .flatten()
                    .zip(&want)
                    .filter(|(g, w)| !f16_close(**g, **w))
                    .count();
                assert_eq!(bad, 0, "{name} level {level}");
                above = got.iter().flatten().copied().collect();
                above_size = want_size;
            }
        }
    }
}

/// The code whose linear light is nearest `v` on a monotone curve, as a
/// real number between codes (linear interpolation), so the round trip
/// measures the render, not the search.
fn code_of(enc: &InputEncoding, channel: usize, v: f64) -> f64 {
    let c = &enc.curves[channel];
    let at = |i: usize| f64::from(c.code(i as u16));
    if v <= at(0) {
        return 0.0;
    }
    if v >= at(CODES - 1) {
        return (CODES - 1) as f64;
    }
    let (mut lo, mut hi) = (0usize, CODES - 1);
    while hi - lo > 1 {
        let mid = (lo + hi) / 2;
        if at(mid) <= v { lo = mid } else { hi = mid }
    }
    let span = at(hi) - at(lo);
    lo as f64 + if span > 0.0 { (v - at(lo)) / span } else { 0.0 }
}

#[test]
fn colors_come_back_within_one_code() {
    let Some((_lock, gpu)) = gpu() else { return };
    for (name, enc, toe) in encodings() {
        let back = invert(&enc.to_working).unwrap();
        let t = encoded_transform(&enc);
        for sixteen in [false, true] {
            let (colors, src) = source(&enc, sixteen);
            let gpu1 = &levels(&gpu, SMALL, &src)[0];
            // The same texels in f32, before the pyramid's f16.
            let mut s3 = stage1(&src);
            stage2(&mut s3, &src, t.multipliers);
            let s4: Vec<f32> = s3
                .chunks(3)
                .flat_map(|c| stage4([c[0], c[1], c[2]], &t))
                .collect();
            let (cpu1, _) = reduce(&s4, src.size(), 3);
            // Back into the source's space, in 8-bit codes (257 16-bit
            // codes each), unrounded.
            let codes = |working: [f32; 3]| {
                let lin = mul(&back, working.map(f64::from));
                [0, 1, 2].map(|ch| code_of(&enc, ch, lin[ch]) / 257.0)
            };
            let (mut exact, mut stored, mut rounded, mut spread, mut de) =
                (0f64, 0f64, 0f64, 0f64, 0f64);
            // Equal channels, up to f16 rounding (2^-11, either side), checked
            // below with the ΔE.
            for (i, c) in colors.iter().enumerate() {
                let want = c.map(|v| f64::from(v) / 257.0);
                let f32_codes = codes([0, 1, 2].map(|ch| cpu1[3 * i + ch]));
                let f16_codes = codes(gpu1[i]);
                let truth = enc.decode(*c);
                de = de.max(delta_e_2000(lab(truth), lab(gpu1[i].map(f64::from))));
                for ch in 0..3 {
                    exact = exact.max((f32_codes[ch] - want[ch]).abs());
                    stored = stored.max((f16_codes[ch] - want[ch]).abs());
                    rounded = rounded.max((f16_codes[ch].round() - want[ch].round()).abs());
                }
                if c[0] == c[1] && c[1] == c[2] {
                    let w = gpu1[i].map(f64::from);
                    let (hi, lo) = (w[0].max(w[1]).max(w[2]), w[0].min(w[1]).min(w[2]));
                    spread = spread.max((hi - lo) / hi.max(1e-4));
                }
            }
            let bits = if sixteen { 16 } else { 8 };
            println!(
                "{name}, {bits}-bit: f32 {exact:.4} code, after f16 {stored:.3} ({rounded} rounded), ΔE2000 {de:.4}, neutral spread {spread:.1e}"
            );
            // f16's 11 bits on a saturated color's AP1 channels.
            assert!(de < 0.1, "{name}, {bits}-bit: ΔE {de}");
            assert!(
                spread <= 1.0 / 1024.0,
                "{name}, {bits}-bit: neutral spread {spread}"
            );
            if !toe {
                continue;
            }
            // The math is an identity: what is left is f32 rounding.
            assert!(exact <= 0.01, "{name}, {bits}-bit: f32 {exact} codes");
            // RGBA16F keeps 11 bits: a channel near 0 beside a bright one
            // comes back up to about 1.2 codes off (sRGB (2, 250, 0)). An
            // output frame's codes are whole, so that still rounds to
            // within 1 (criterion 8).
            assert!(stored < 1.5, "{name}, {bits}-bit: {stored} codes after f16");
            assert!(rounded <= 1.0, "{name}, {bits}-bit: {rounded} whole codes");
        }
    }
}

/// The working space as a moxcms profile: linear AP1, D65 white.
fn working_profile() -> moxcms::ColorProfile {
    let mut p = moxcms::ColorProfile::new_srgb();
    p.update_rgb_colorimetry(
        moxcms::XyY {
            x: 0.3127,
            y: 0.3290,
            yb: 1.0,
        },
        moxcms::ColorPrimaries::ACES_CG,
    );
    let linear = moxcms::ToneReprCurve::Parametric(vec![1.0]);
    p.red_trc = Some(linear.clone());
    p.green_trc = Some(linear.clone());
    p.blue_trc = Some(linear);
    p
}

fn moxcms_profile(color: &ColorEncoding) -> moxcms::ColorProfile {
    match color {
        ColorEncoding::Srgb => moxcms::ColorProfile::new_srgb(),
        ColorEncoding::DisplayP3 => moxcms::ColorProfile::new_display_p3(),
        ColorEncoding::AdobeRgb => moxcms::ColorProfile::new_adobe_rgb(),
        ColorEncoding::Icc(bytes) => moxcms::ColorProfile::new_from_slice(bytes).unwrap(),
        ColorEncoding::Nclx { .. } => panic!("no NCLX fixture"),
    }
}

fn lab(v: [f64; 3]) -> [f64; 3] {
    xyz_to_lab(mul(&WORKING_TO_XYZ, v), D65_WHITE)
}

#[test]
fn decoder_fixtures_match_moxcms() {
    let Some((_lock, gpu)) = gpu() else { return };
    let dir =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../imgio/tests/fixtures/images");
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .filter(|n| n.ends_with(".rgba16"))
        .map(|n| dir.join(n.trim_end_matches(".rgba16")))
        .collect();
    files.sort();
    assert!(files.len() >= 25, "{} fixtures", files.len());
    // HEIC: a Display P3 one tagged as iPhones tag theirs (make-heic.sh),
    // and libheif's own test image, which carries GIMP's sRGB profile.
    files.push(dir.join("heic-p3-nclx.heic"));
    files.push(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../heif/vendor/libheif/tests/data/rainbow-451x461.heic"),
    );
    // Profiles from other color engines than moxcms, which also reads them
    // here: their result against the sRGB formula is independent of it.
    let srgb_profiles = ["png-icc-lcms2-srgb.png", "rainbow-451x461.heic"];
    // Test bytes, not ICC (make-images.py).
    let junk_profiles = ["jpeg-icc-multi.jpg", "png-iccp.png", "tiff-icc-orient3.tif"];
    let mut stages = RawStages::new(&gpu, SMALL);
    let mut kinds = std::collections::BTreeSet::new();
    for path in &files {
        let name = path.file_name().unwrap().to_str().unwrap();
        let img = match decode(&std::fs::read(path).unwrap(), DecodeOptions::default()) {
            Ok(img) => img,
            // imgio's own tests expect the refusal (an RGB palette TIFF).
            Err(DecodeError::Unsupported(..)) => continue,
            Err(e) => panic!("{name}: {e}"),
        };
        let (w, h) = (img.width, img.height);
        let Pixels::Rgba16 { data, color } = img.pixels.clone() else {
            panic!("{name}: not RGBA")
        };
        let gray = data.chunks(4).all(|p| p[0] == p[1] && p[1] == p[2]);
        if junk_profiles.contains(&name) {
            // A profile moxcms cannot read is refused, not guessed at.
            let e = RawSource::from_decoded(img).unwrap_err();
            assert!(
                matches!(e, RawError::Encoding(EncodingError::Icc(_))),
                "{name}: {e}"
            );
            println!("{name}: refused, {e}");
            continue;
        }
        let src = RawSource::from_decoded(img).unwrap();
        kinds.insert(match &color {
            ColorEncoding::Srgb => "sRGB",
            ColorEncoding::DisplayP3 => "Display P3",
            ColorEncoding::AdobeRgb => "Adobe RGB",
            ColorEncoding::Icc(_) => "ICC",
            ColorEncoding::Nclx { .. } => "NCLX",
        });
        let enc = src.encoding.clone().unwrap();
        assert_eq!(enc, input_encoding(&color).unwrap(), "{name}");
        let t = encoded_transform(&enc);
        let p = stages.render_pyramid(&gpu, &src, &t).unwrap();
        let got = p.read_level(&gpu, 1).unwrap();
        // moxcms per pixel into linear AP1, then the same 2 x 2 reduction.
        // A gray image's ICC profile is gray: moxcms takes gray in.
        let profile = moxcms_profile(&color);
        let is_gray_profile = profile.color_space == moxcms::DataColorSpace::Gray;
        let options = moxcms::TransformOptions {
            allow_use_cicp_transfer: false,
            ..Default::default()
        };
        let layout = if is_gray_profile {
            moxcms::Layout::Gray
        } else {
            moxcms::Layout::Rgb
        };
        let input: Vec<f32> = data
            .chunks(4)
            .flat_map(|p| {
                let n = if is_gray_profile { 1 } else { 3 };
                p[..n]
                    .iter()
                    .map(|&v| f32::from(v) / 65535.0)
                    .collect::<Vec<_>>()
            })
            .collect();
        let mut out = vec![0f32; (w * h * 3) as usize];
        profile
            .create_transform_f32(layout, &working_profile(), moxcms::Layout::Rgb, options)
            .unwrap()
            .transform(&input, &mut out)
            .unwrap();
        let (want, size) = reduce(&out, (w, h), 3);
        assert_eq!(size, p.size);
        let mut de = 0f64;
        for (g, wnt) in got.iter().zip(want.chunks(3)) {
            let (g, wnt) = (g.map(f64::from), [0, 1, 2].map(|c| f64::from(wnt[c])));
            de = de.max(delta_e_2000(lab(g), lab(wnt)));
        }
        println!(
            "{name}: {} ({w}x{h}{}), ΔE2000 from moxcms {de:.3}",
            enc.name,
            if gray { ", gray" } else { "" }
        );
        assert!(de < 0.5, "{name}: ΔE {de}");
        if name == "heic-p3-nclx.heic" {
            assert_eq!(color, ColorEncoding::DisplayP3);
        }
        if srgb_profiles.contains(&name) {
            assert!(matches!(color, ColorEncoding::Icc(_)), "{name}");
            let formula = InputEncoding::srgb();
            let mut apart = 0f64;
            for p in data.chunks(4) {
                let rgb = [p[0], p[1], p[2]];
                apart = apart.max(delta_e_2000(lab(enc.decode(rgb)), lab(formula.decode(rgb))));
            }
            println!("{name}: its profile against the sRGB formula, ΔE2000 {apart:.3}");
            assert!(apart < 0.1, "{name}: ΔE {apart} from the sRGB formula");
        }
    }
    println!("encodings seen: {kinds:?}");
    for kind in ["sRGB", "Display P3", "Adobe RGB", "ICC"] {
        assert!(kinds.contains(kind), "no {kind} fixture");
    }
}

/// The stored golden of a non-RAW pyramid's first level: the synthetic
/// scene, read as Display P3 values and stored as 16-bit codes, through
/// stages 1 to 4, viewed as sRGB. Written on lavapipe.
#[test]
fn golden_display_p3_level1() {
    let Some((_lock, gpu)) = gpu() else { return };
    let (w, h) = (768u32, 512u32);
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let v = genzo_pipeline::gpu::synthetic::scene(x, y, w, h);
            for c in v {
                let code = genzo_color::srgb_encode(f64::from(c).clamp(0.0, 1.0)) * 65535.0;
                rgba.push(code.round() as u16);
            }
            rgba.push(65535);
        }
    }
    let enc = InputEncoding::display_p3();
    let src = RawSource::from_encoded((w, h), &rgba, &enc).unwrap();
    let level1 = &levels(&gpu, SMALL, &src)[0];
    let to_srgb =
        genzo_color::mat_mul(&invert(&genzo_color::SRGB_TO_XYZ).unwrap(), &WORKING_TO_XYZ);
    let case = genzo_golden::Case::new(
        env!("CARGO_MANIFEST_DIR"),
        "pyramid/encoded-display-p3-level1",
        genzo_golden::Space::Linear(genzo_golden::View {
            matrix: to_srgb,
            exposure_ev: 0.0,
        }),
        genzo_golden::Producer::Gpu {
            adapter: gpu.info.name.clone(),
        },
    );
    genzo_golden::check(
        &case,
        &genzo_golden::Image::new(w / 2, h / 2, level1.clone()),
    );
}
