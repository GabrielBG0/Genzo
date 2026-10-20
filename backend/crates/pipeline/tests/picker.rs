//! The white balance picker (DEV-02), on the CPU:
//!
//! - a neutral area lit by a known white gives back that white's Temp and
//!   Tint, RAW and non-RAW;
//! - clipped, too dark and colorless areas are refused;
//! - the point goes through orientation and a straightened crop to the
//!   right pixels;
//! - `stage1_window` equals the whole image's stage 1 at its pixels, with
//!   FixBadPixelsConstant and a GainMap, and on the corpus files that carry
//!   them when they are present.

#[allow(dead_code)]
mod sdk_stage2;

use genzo_color::camera::{ColorSpec, xy_from_temperature};
use genzo_color::encoding::InputEncoding;
use genzo_color::white_balance::relative_adaptation;
use genzo_imgio::opcodes::{AreaSpec, FixBadPixelsConstant, GainMap};
use genzo_imgio::{DecodeOptions, decode};
use genzo_model::frame::Rect;
use genzo_pipeline::camera_color::dng_profile;
use genzo_pipeline::geometry::Crop;
use genzo_pipeline::gpu::synthetic::{blocks, scene_raw, with_camera_profile};
use genzo_pipeline::picker::{PickError, pick};
use genzo_pipeline::raw::RawSource;
use genzo_pipeline::reference::sampling::Frame;
use genzo_pipeline::reference::stages::{stage1, stage1_window};

/// The camera neutral of a white `(temp, tint)` for the synthetic profile,
/// largest channel 1.
fn neutral(src: &RawSource, temp: f64, tint: f64) -> [f64; 3] {
    let spec = ColorSpec::new(&dng_profile(&src.color).unwrap()).unwrap();
    spec.at_white(xy_from_temperature(temp, tint))
        .unwrap()
        .camera_white
}

/// The profiled synthetic mosaic with every pixel set to the camera value
/// `value(x, y)` (white 1), through its linearization.
fn painted(w: u32, h: u32, value: impl Fn(u32, u32) -> [f64; 3]) -> RawSource {
    let mut src = with_camera_profile(scene_raw(w, h));
    for y in 0..h {
        for x in 0..w {
            let c = usize::from(src.color_at(y, x).unwrap());
            let v = value(x, y)[c];
            let lin = &src.linearization;
            let code = (v + f64::from(lin.black(y, x, 0))) / f64::from(lin.scale[0]);
            src.data[(y * w + x) as usize] = code.round().clamp(0.0, 65535.0) as u16;
        }
    }
    src
}

#[test]
fn a_neutral_area_gives_its_white_back() {
    for (t, tint) in [
        (2800.0, 5.0),
        (3500.0, 10.0),
        (5500.0, 0.0),
        (8000.0, -20.0),
    ] {
        let n = neutral(&src_profile(), t, tint);
        let src = painted(400, 300, |_, _| n.map(|v| 0.4 * v));
        let (got_t, got_tint) = pick(&src, Frame::from(1), (0.5, 0.5)).unwrap();
        println!("{t} K {tint:+}: picked {got_t:.2} K {got_tint:+.3}");
        assert!((got_t - t).abs() <= 3.0, "{t}: {got_t}");
        assert!((got_tint - tint).abs() <= 0.3, "{tint}: {got_tint}");
    }
}

fn src_profile() -> RawSource {
    with_camera_profile(scene_raw(8, 8))
}

#[test]
fn clipped_dark_and_colorless_areas_are_refused() {
    let n = neutral(&src_profile(), 4000.0, 0.0);
    let clipped = painted(64, 64, |_, _| n.map(|v| 0.97 * v));
    assert_eq!(
        pick(&clipped, Frame::from(1), (0.5, 0.5)),
        Err(PickError::Clipped)
    );
    // One hot sample in the window is enough.
    let one = painted(64, 64, |x, y| {
        if (x, y) == (32, 32) {
            [0.96; 3]
        } else {
            n.map(|v| 0.4 * v)
        }
    });
    assert_eq!(
        pick(&one, Frame::from(1), (0.5, 0.5)),
        Err(PickError::Clipped)
    );
    let dark = painted(64, 64, |_, _| n.map(|v| 0.003 * v));
    assert_eq!(
        pick(&dark, Frame::from(1), (0.5, 0.5)),
        Err(PickError::TooDark)
    );
    let mut gray = painted(64, 64, |_, _| n.map(|v| 0.4 * v));
    gray.cfa = None;
    assert_eq!(
        pick(&gray, Frame::from(1), (0.5, 0.5)),
        Err(PickError::NoColor)
    );
    let unprofiled = scene_raw(64, 64);
    assert_eq!(
        pick(&unprofiled, Frame::from(1), (0.5, 0.5)),
        Err(PickError::NoColor)
    );
}

/// The stored image's left half lit at 3000 K, its right half at 7000 K:
/// through a quarter turn, a flip and a straightened crop, each point
/// picks the half it lands on.
#[test]
fn the_point_goes_through_the_geometry() {
    let (w, h) = (600, 400);
    let (left, right) = (
        neutral(&src_profile(), 3000.0, 0.0),
        neutral(&src_profile(), 7000.0, 0.0),
    );
    let src = painted(w, h, |x, _| {
        if x < w / 2 { left } else { right }.map(|v| 0.3 * v)
    });
    let crop = Crop {
        cx: 0.5,
        cy: 0.45,
        w: 0.6,
        h: 0.7,
        angle: 10.0,
    };
    let mut sides = [0, 0];
    for o in [1u8, 6, 7] {
        let frame = Frame {
            orientation: o,
            crop: Some(crop),
        };
        for p in [(0.2, 0.3), (0.8, 0.7), (0.3, 0.8), (0.7, 0.2)] {
            let (x, _) = frame.stored_point((w, h), p);
            // Points within the window of the seam would mix the halves.
            if (x - f64::from(w / 2)).abs() < 12.0 {
                continue;
            }
            let want = if x < f64::from(w / 2) { 3000.0 } else { 7000.0 };
            sides[usize::from(x >= f64::from(w / 2))] += 1;
            let (t, _) = pick(&src, frame, p).unwrap();
            assert!((t - want).abs() < 5.0, "o {o}, {p:?} at x {x:.0}: {t}");
        }
    }
    assert!(sides[0] > 0 && sides[1] > 0, "{sides:?}");
}

/// The windows the checks compare: corners, edges and the middle.
fn rects(w: u32, h: u32) -> Vec<Rect> {
    let r = |x, y| Rect {
        x,
        y,
        width: 10,
        height: 10,
    };
    vec![
        r(0, 0),
        r(w - 10, h - 10),
        r(1, h / 2),
        r(w / 2 + 1, 3),
        r(w / 2, h / 2),
    ]
}

fn assert_window_matches(src: &RawSource, name: &str) {
    let whole = stage1(src);
    let (w, h) = src.size();
    let n = src.samples as usize;
    for rect in rects(w, h) {
        let got = stage1_window(src, rect);
        let mut i = 0;
        for row in rect.y..rect.y + rect.height {
            for col in rect.x..rect.x + rect.width {
                for p in 0..n {
                    let want = whole[(row as usize * w as usize + col as usize) * n + p];
                    assert_eq!(
                        got[i].to_bits(),
                        want.to_bits(),
                        "{name}: {rect:?} at ({col}, {row}), plane {p}"
                    );
                    i += 1;
                }
            }
        }
    }
}

#[test]
fn a_window_of_stage_1_equals_the_whole_image() {
    let mut src = with_camera_profile(scene_raw(300, 200));
    // Bad pixels at the corners, the edges and inside, next to each other
    // too, with two constants so both ops act.
    for (x, y, v) in [
        (0, 0, 0),
        (299, 199, 0),
        (5, 100, 0),
        (6, 101, 0),
        (151, 2, 7),
        (150, 100, 7),
        (152, 100, 0),
    ] {
        src.data[(y * 300 + x) as usize] = v;
    }
    src.fix_bad_pixels = vec![
        FixBadPixelsConstant {
            constant: 0,
            bayer_phase: 1,
        },
        FixBadPixelsConstant {
            constant: 7,
            bayer_phase: 1,
        },
    ];
    // A 3 x 4 map over a pitched area, every other row.
    src.gain_maps = vec![GainMap {
        area: AreaSpec {
            top: 10,
            left: 20,
            bottom: 180,
            right: 290,
            plane: 0,
            planes: 1,
            row_pitch: 2,
            col_pitch: 1,
        },
        points_v: 3,
        points_h: 4,
        spacing_v: 0.4,
        spacing_h: 0.3,
        origin_v: 0.05,
        origin_h: 0.02,
        map_planes: 1,
        gains: vec![
            1.0, 1.1, 1.2, 1.3, 0.9, 1.05, 1.15, 1.25, 1.4, 1.0, 0.95, 1.5,
        ],
    }];
    assert_window_matches(&src, "synthetic");
    // The window's pixels changed where the opcodes act.
    assert_ne!(
        stage1(&src),
        stage1(&with_camera_profile(scene_raw(300, 200)))
    );
}

#[test]
fn a_window_of_stage_1_equals_the_whole_image_on_the_corpus() {
    // The Leica carries FixBadPixelsConstant, the Pixel 4a GainMaps.
    for key in [
        "raw/Leica/Q2/L1000750.DNG",
        "raw/Google/Pixel 4a/PXL_20201121_100251397.dng",
    ] {
        let Some(path) = sdk_stage2::corpus_file(key) else {
            continue;
        };
        let img = decode(&std::fs::read(path).unwrap(), DecodeOptions::default()).unwrap();
        let src = RawSource::from_decoded(img).unwrap();
        assert!(
            !src.fix_bad_pixels.is_empty() || !src.gain_maps.is_empty(),
            "{key}"
        );
        assert_window_matches(&src, key);
    }
}

/// A non-RAW gray area seen under a shifted white: the shift that makes it
/// neutral again.
#[test]
fn a_non_raw_area_gives_its_shift_back() {
    let e = InputEncoding::srgb();
    let from_working = genzo_color::invert(&e.to_working).unwrap();
    for (temp, tint) in [(-60.0, 0.0), (35.0, -20.0), (80.0, 45.0), (0.0, 0.0)] {
        // Gray, then the inverse of the shift, then the sRGB encoding.
        let undo = genzo_color::invert(&relative_adaptation(temp, tint)).unwrap();
        let working = genzo_color::mul(&undo, [0.3; 3]);
        let linear = genzo_color::mul(&from_working, working);
        let code = linear.map(|v| (genzo_color::srgb_encode(v) * 65535.0).round() as u16);
        let (size, rgba) = blocks(&vec![code; 64 * 48], 64);
        let src = RawSource::from_encoded(size, &rgba, &e).unwrap();
        let (got_temp, got_tint) = pick(&src, Frame::from(1), (0.5, 0.5)).unwrap();
        println!("{temp:+} {tint:+}: picked {got_temp:+.3} {got_tint:+.3}");
        assert!((got_temp - temp).abs() < 0.2, "{temp}: {got_temp}");
        assert!((got_tint - tint).abs() < 0.2, "{tint}: {got_tint}");
    }
}

/// OkLCh chroma of a working space color.
fn chroma(working: [f64; 3]) -> f64 {
    use genzo_color::output::{oklch, xyz_to_oklab};
    oklch(xyz_to_oklab(genzo_color::mul(
        &genzo_color::WORKING_TO_XYZ,
        working,
    )))[1]
}

/// DEV-02 criterion 8, RAW: a synthetic sensor with the Pixel 4a's profile
/// shows a gray lit by illuminant A, As Shot at D65. Its pixels are the
/// synthetic mosaic's, so the Pixel 4a's own GainMaps do not apply.
#[test]
fn criterion_8_raw_illuminant_a() {
    let key = "raw/Google/Pixel 4a/PXL_20201121_100251397.dng";
    let Some(path) = sdk_stage2::corpus_file(key) else {
        return;
    };
    let img = decode(&std::fs::read(path).unwrap(), DecodeOptions::default()).unwrap();
    let pixel = RawSource::from_decoded(img).unwrap();
    let mut color = pixel.color.clone();
    let spec = ColorSpec::new(&dng_profile(&color).unwrap()).unwrap();
    let d65 = spec.at_white(genzo_color::camera::D65_XY).unwrap();
    color.as_shot = Some(genzo_imgio::raw::AsShot::Neutral(d65.camera_white));
    // The camera's values for a gray under illuminant A.
    let a = spec.at_white([0.44757, 0.40745]).unwrap().camera_white;
    let mut src = painted(400, 300, |_, _| a.map(|v| 0.35 * v));
    src.color = color.clone();
    let (t, tint) = pick(&src, Frame::from(1), (0.5, 0.5)).unwrap();
    println!("illuminant A on the Pixel 4a profile: picked {t:.1} K {tint:+.2}");
    assert!((t - 2856.0).abs() <= 20.0, "{t}");
    assert!(tint.abs() <= 2.0, "{tint}");
    // Rendered at the picked white, the patch is neutral.
    let picked =
        genzo_pipeline::camera_color::camera_transform(&color, Some(xy_from_temperature(t, tint)));
    let camera = a.map(|v| (0.35 * v) as f32);
    let balanced = [0, 1, 2].map(|c| camera[c] * picked.multipliers[c]);
    let working = genzo_pipeline::reference::stages::stage4(balanced, &picked).map(f64::from);
    let c = chroma(working);
    println!("  rendered chroma {c:.5}");
    assert!(c <= 0.005, "{c}");
}

/// DEV-02 criterion 8, non-RAW: a gray rendered warmer at temp +40 and
/// stored as 8-bit sRGB is picked back to neutral.
#[test]
fn criterion_8_non_raw_warmed_gray() {
    let e = InputEncoding::srgb();
    let from_working = genzo_color::invert(&e.to_working).unwrap();
    let warmed = genzo_color::mul(&relative_adaptation(40.0, 0.0), [0.3; 3]);
    let linear = genzo_color::mul(&from_working, warmed);
    // An 8-bit PNG's codes, as 16-bit samples.
    let code = linear.map(|v| {
        let byte = (genzo_color::srgb_encode(v.clamp(0.0, 1.0)) * 255.0).round() as u16;
        byte * 257
    });
    let (size, rgba) = blocks(&vec![code; 64 * 48], 64);
    let src = RawSource::from_encoded(size, &rgba, &e).unwrap();
    let (temp, tint) = pick(&src, Frame::from(1), (0.5, 0.5)).unwrap();
    println!("gray warmed at +40: picked temp {temp:+.2}, tint {tint:+.2}");
    assert!(temp < 0.0, "{temp}");
    let decoded = genzo_color::mul(&e.to_working, code.map(|c| f64::from(e.curves[0].code(c))));
    let c = chroma(genzo_color::mul(&relative_adaptation(temp, tint), decoded));
    println!("  rendered chroma {c:.5}");
    assert!(c <= 0.01, "{c}");
}
