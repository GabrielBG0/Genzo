//! Stage 9, the output transform (DEV-01): linear AP1 to the target's
//! linear RGB, gamut compression toward the achromatic axis (the ACES
//! reference gamut compression method), a clamp, the sRGB transfer function,
//! and 8 bits with a deterministic triangular dither.
//!
//! This is the CPU reference; `pipeline`'s output shader is checked against
//! it.

use std::sync::OnceLock;

use crate::{Mat3, WORKING_TO_XYZ, invert, mat_mul, mul, rgb_to_xyz, srgb_encode};

/// The white both targets and the working space share, CIE xy.
const D65: [f64; 2] = [0.3127, 0.3290];

/// What a frame is encoded for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Srgb,
    DisplayP3,
}

impl Target {
    pub fn primaries(self) -> [[f64; 2]; 3] {
        match self {
            Target::Srgb => [[0.64, 0.33], [0.30, 0.60], [0.15, 0.06]],
            Target::DisplayP3 => [[0.680, 0.320], [0.265, 0.690], [0.150, 0.060]],
        }
    }

    /// Linear target RGB to XYZ, from the primaries and the working space's
    /// own D65, so a source encoded with the same primaries comes back
    /// exactly (`encoding::InputEncoding` builds its matrices the same way).
    pub fn to_xyz(self) -> Mat3 {
        self.matrices().to_xyz
    }

    /// Linear AP1 to linear target RGB.
    pub fn from_working(self) -> Mat3 {
        self.matrices().from_working
    }

    /// The target's matrices, derived once: stage 9 uses them for every
    /// pixel, and inverting them each time cost more than the rest of it.
    fn matrices(self) -> &'static Matrices {
        static SRGB: OnceLock<Matrices> = OnceLock::new();
        static P3: OnceLock<Matrices> = OnceLock::new();
        let cell = match self {
            Target::Srgb => &SRGB,
            Target::DisplayP3 => &P3,
        };
        cell.get_or_init(|| {
            let to_xyz = rgb_to_xyz(self.primaries(), D65);
            let from_xyz = invert(&to_xyz).expect("invertible");
            Matrices {
                to_xyz,
                from_working: mat_mul(&from_xyz, &WORKING_TO_XYZ),
                from_xyz_oklab: FromOklab {
                    lab_to_lms: invert(&OKLAB_M2).expect("invertible"),
                    lms_to_rgb: mat_mul(&from_xyz, &invert(&OKLAB_M1).expect("invertible")),
                },
            }
        })
    }

    /// The gamut compression for this target.
    pub fn compression(self) -> Compression {
        match self {
            Target::Srgb => SRGB_COMPRESSION,
            Target::DisplayP3 => P3_COMPRESSION,
        }
    }
}

/// Hue-preserving gamut compression (decided 2026-10-19, replacing the ACES
/// per-channel method, which moved saturated blues 25 to 40 degrees in
/// OkLCh hue): a color's Oklab chroma is compressed at its own lightness and
/// hue. `r`, its chroma as a fraction of the most the target holds there,
/// is kept below `threshold`; above it, `threshold..limit` is compressed
/// onto `threshold..1` by the ACES curve of the given power, and anything
/// past `limit` lands on the gamut edge.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Compression {
    pub threshold: f64,
    pub limit: f64,
    pub power: f64,
}

/// Set against criterion 9 (the tests below). The limits are about the
/// 99th percentile of `r` over the AP1 gamut (sRGB 2.22, Display P3 1.65),
/// so nearly all of it keeps an order in chroma and the rest lands on the
/// edge. The threshold and power are of the ACES reference values' order.
pub const SRGB_COMPRESSION: Compression = Compression {
    threshold: 0.8,
    limit: 2.25,
    power: 1.2,
};
pub const P3_COMPRESSION: Compression = Compression {
    threshold: 0.8,
    limit: 1.7,
    power: 1.2,
};

/// The ACES reference gamut compression's curve (v1.3, `compress`): `d`
/// below `threshold` is kept, `limit` maps to 1.
pub fn compress_distance(d: f64, threshold: f64, limit: f64, power: f64) -> f64 {
    if d < threshold {
        return d;
    }
    let scale = (limit - threshold)
        / (((1.0 - threshold) / (limit - threshold)).powf(-power) - 1.0).powf(1.0 / power);
    let n = (d - threshold) / scale;
    threshold + scale * n / (1.0 + n.powf(power)).powf(1.0 / power)
}

/// Steps of the search for the gamut edge: chroma to 0.5 / 2^24.
pub const EDGE_STEPS: u32 = 24;
/// No target or working-space color reaches this Oklab chroma.
pub const CHROMA_SEARCH: f64 = 0.5;

/// The largest chroma `target` holds at Oklab lightness `l` and hue
/// direction (`a`, `b` per unit chroma), by bisection.
pub fn max_chroma(l: f64, dir: [f64; 2], target: Target) -> f64 {
    search(&target.from_xyz_oklab(), l, dir, |_| false)
}

/// The bisection of [`max_chroma`], stopped at the first chroma found
/// inside for which `enough` holds.
fn search(to_rgb: &FromOklab, l: f64, dir: [f64; 2], enough: impl Fn(f64) -> bool) -> f64 {
    let (mut lo, mut hi) = (0.0, CHROMA_SEARCH);
    for _ in 0..EDGE_STEPS {
        let mid = 0.5 * (lo + hi);
        let rgb = oklab_to(to_rgb, [l, mid * dir[0], mid * dir[1]]);
        if rgb.iter().all(|v| (0.0..=1.0).contains(v)) {
            lo = mid;
            if enough(lo) {
                break;
            }
        } else {
            hi = mid
        }
    }
    lo
}

/// Gamut compression of one linear target-RGB color.
pub fn compress(rgb: [f64; 3], target: Target, c: &Compression) -> [f64; 3] {
    let lab = xyz_to_oklab(mul(&target.to_xyz(), rgb));
    let (l, chroma) = (lab[0], lab[1].hypot(lab[2]));
    // Black, white and beyond have no chroma to keep: the clamp decides.
    if l <= 0.0 || l >= 1.0 || chroma <= 0.0 {
        return rgb;
    }
    let dir = [lab[1] / chroma, lab[2] / chroma];
    let to_rgb = target.from_xyz_oklab();
    // Most colors lie well inside: the search stops once its chroma inside
    // keeps `r` under the threshold. It only climbs from there, so the
    // whole search would keep the color too, by the same steps.
    let edge = search(&to_rgb, l, dir, |lo| chroma / lo < c.threshold);
    if edge <= 0.0 {
        return rgb;
    }
    let r = chroma / edge;
    if r < c.threshold {
        return rgb;
    }
    let r = compress_distance(r, c.threshold, c.limit, c.power).min(1.0);
    oklab_to(&to_rgb, [l, r * edge * dir[0], r * edge * dir[1]])
}

/// Triangular noise in (−1, 1) for pixel (x, y), from an integer hash, so
/// CPU and GPU draw the same values.
pub fn dither(x: u32, y: u32) -> f32 {
    let a = pcg(x ^ pcg(y));
    let b = pcg(a);
    let unit = |h: u32| (h >> 8) as f32 / 16_777_216.0;
    unit(a) + unit(b) - 1.0
}

/// PCG's output permutation as a hash (Jarzynski and Olano 2020).
pub fn pcg(v: u32) -> u32 {
    let state = v.wrapping_mul(747_796_405).wrapping_add(2_891_336_453);
    let word = ((state >> ((state >> 28) + 4)) ^ state).wrapping_mul(277_803_737);
    (word >> 22) ^ word
}

/// One 8-bit code from an encoded value 0 to 1 and its dither.
pub fn quantize(encoded: f64, noise: f32) -> u8 {
    (encoded * 255.0 + 0.5 + f64::from(noise))
        .floor()
        .clamp(0.0, 255.0) as u8
}

/// Stage 9 for one working-space color, before the dither: the encoded
/// target RGB, 0 to 1.
pub fn encode(working: [f64; 3], target: Target, compress_gamut: bool) -> [f64; 3] {
    let mut rgb = mul(&target.from_working(), working);
    if compress_gamut {
        rgb = compress(rgb, target, &target.compression());
    }
    rgb.map(|v| srgb_encode(v.clamp(0.0, 1.0)))
}

/// One frame pixel: stage 9, and the dither for pixel (x, y) when `dithered`
/// (not for an unedited non-RAW photo, decided 2026-10-19: it shows its own
/// codes).
pub fn pixel(
    working: [f64; 3],
    (x, y): (u32, u32),
    target: Target,
    compress_gamut: bool,
    dithered: bool,
) -> [u8; 3] {
    let noise = if dithered { dither(x, y) } else { 0.0 };
    encode(working, target, compress_gamut).map(|v| quantize(v, noise))
}

/// Whether a source with `to_working` (linear source RGB to AP1) fits in
/// `target`, so a non-RAW photo skips gamut compression (DEV-01): each
/// source primary lands inside the target's 0 to 1, within [`FIT_SLACK`].
pub fn fits(to_working: &Mat3, target: Target) -> bool {
    let m = mat_mul(&target.from_working(), to_working);
    m.iter()
        .flatten()
        .all(|v| (-FIT_SLACK..=1.0 + FIT_SLACK).contains(v))
}

/// How far outside the target a source primary may land and still fit.
/// Real sRGB and Display P3 ICC profiles miss the formulas by up to 5e-5
/// (lcms2's 1.4e-5, moxcms's 5.0e-5: s15Fixed16 colorants and moxcms's
/// D65 of 0.31272, 0.32903); a wider gamut misses by far more (Adobe RGB
/// in sRGB: about 0.2).
pub const FIT_SLACK: f64 = 1e-3;

/// CIE XYZ (D65, Y = 1 white) to Oklab (Ottosson 2020, the XYZ matrices).
pub fn xyz_to_oklab(xyz: [f64; 3]) -> [f64; 3] {
    let lms = mul(&OKLAB_M1, xyz).map(f64::cbrt);
    mul(&OKLAB_M2, lms)
}

pub const OKLAB_M1: Mat3 = [
    [0.818_933_010_1, 0.361_866_742_4, -0.128_859_713_7],
    [0.032_984_543_6, 0.929_311_871_5, 0.036_145_638_7],
    [0.048_200_301_8, 0.264_366_269_1, 0.633_851_707_0],
];
pub const OKLAB_M2: Mat3 = [
    [0.210_454_255_3, 0.793_617_785_0, -0.004_072_046_8],
    [1.977_998_495_1, -2.428_592_205_0, 0.450_593_709_9],
    [0.025_904_037_1, 0.782_771_766_2, -0.808_675_766_0],
];

/// The matrices from Oklab back to linear target RGB: Oklab to cone
/// responses (cubed after), and those to the target.
#[derive(Debug, Clone, Copy)]
pub struct FromOklab {
    pub lab_to_lms: Mat3,
    pub lms_to_rgb: Mat3,
}

impl Target {
    pub fn from_xyz_oklab(self) -> FromOklab {
        self.matrices().from_xyz_oklab
    }
}

#[derive(Debug)]
struct Matrices {
    to_xyz: Mat3,
    from_working: Mat3,
    from_xyz_oklab: FromOklab,
}

/// Oklab to linear target RGB.
pub fn oklab_to(m: &FromOklab, lab: [f64; 3]) -> [f64; 3] {
    let lms = mul(&m.lab_to_lms, lab).map(|v| v * v * v);
    mul(&m.lms_to_rgb, lms)
}

/// Oklab's lightness, chroma and hue in degrees.
pub fn oklch(lab: [f64; 3]) -> [f64; 3] {
    let c = lab[1].hypot(lab[2]);
    let h = lab[2].atan2(lab[1]).to_degrees().rem_euclid(360.0);
    [lab[0], c, h]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The AP1 gamut's surface at unit brightness: every color with one
    /// channel at 1 and the others on a grid from 0 to 1.
    fn ap1_surface(steps: usize) -> Vec<[f64; 3]> {
        let mut out = vec![];
        for i in 0..=steps {
            for j in 0..=steps {
                let (a, b) = (i as f64 / steps as f64, j as f64 / steps as f64);
                out.extend([[1.0, a, b], [a, 1.0, b], [a, b, 1.0]]);
            }
        }
        out
    }

    fn hue_of(rgb: [f64; 3], target: Target) -> [f64; 3] {
        oklch(xyz_to_oklab(mul(&target.to_xyz(), rgb)))
    }

    /// The limits: only about 1 % of the AP1 gamut lies past them, where
    /// colors land on the target's edge.
    #[test]
    fn the_limits_hold_most_of_ap1() {
        for target in [Target::Srgb, Target::DisplayP3] {
            let m = target.from_working();
            let mut ratios = vec![];
            for w in ap1_surface(64) {
                for scale in [0.05, 0.2, 0.5, 0.9] {
                    let lab = xyz_to_oklab(mul(&target.to_xyz(), mul(&m, w.map(|v| v * scale))));
                    let ch = lab[1].hypot(lab[2]);
                    if lab[0] > 0.0 && lab[0] < 1.0 && ch > 1e-9 {
                        let edge = max_chroma(lab[0], [lab[1] / ch, lab[2] / ch], target);
                        ratios.push(ch / edge.max(1e-12));
                    }
                }
            }
            ratios.sort_by(f64::total_cmp);
            let q = |p: f64| ratios[((ratios.len() - 1) as f64 * p) as usize];
            let past = ratios
                .iter()
                .filter(|r| **r > target.compression().limit)
                .count() as f64
                / ratios.len() as f64;
            println!(
                "{target:?}: r p50 {:.2}, p90 {:.2}, p99 {:.2}, max {:.2}; {:.2} % past the limit",
                q(0.5),
                q(0.9),
                q(0.99),
                q(1.0),
                100.0 * past
            );
            assert!(past < 0.015, "{target:?}: {past}");
        }
    }

    /// `compress` as it was before its shortcut: the edge always searched
    /// for. The oracle for the shortcut.
    fn compress_by_search(rgb: [f64; 3], target: Target, c: &Compression) -> [f64; 3] {
        let lab = xyz_to_oklab(mul(&target.to_xyz(), rgb));
        let (l, chroma) = (lab[0], lab[1].hypot(lab[2]));
        if l <= 0.0 || l >= 1.0 || chroma <= 0.0 {
            return rgb;
        }
        let dir = [lab[1] / chroma, lab[2] / chroma];
        let edge = max_chroma(l, dir, target);
        if edge <= 0.0 {
            return rgb;
        }
        let r = chroma / edge;
        if r < c.threshold {
            return rgb;
        }
        let r = compress_distance(r, c.threshold, c.limit, c.power).min(1.0);
        oklab_to(
            &target.from_xyz_oklab(),
            [l, r * edge * dir[0], r * edge * dir[1]],
        )
    }

    /// Linear target RGB of an 8-bit code of Display P3 (the sRGB curve) or
    /// Adobe RGB (gamma 563/256), the wider sources a non-RAW photo has.
    fn source_rgb(code: u32, adobe: bool, target: Target) -> [f64; 3] {
        let primaries = if adobe {
            [[0.64, 0.33], [0.21, 0.71], [0.15, 0.06]]
        } else {
            Target::DisplayP3.primaries()
        };
        let curve = |v: f64| {
            if adobe {
                v.powf(563.0 / 256.0)
            } else {
                crate::srgb_decode(v)
            }
        };
        let to_xyz = rgb_to_xyz(primaries, D65);
        let m = mat_mul(&invert(&target.to_xyz()).unwrap(), &to_xyz);
        let v = [code >> 16, code >> 8 & 255, code & 255].map(|c| curve(f64::from(c) / 255.0));
        mul(&m, v)
    }

    /// The shortcut keeps exactly what the search would: over AP1 (dark,
    /// bright, past white, and near black and white at full chroma, where
    /// rays are shortest), and over every 97th 8-bit code of the wide
    /// sources. `every_code` checks them all.
    #[test]
    fn the_shortcut_keeps_what_the_search_would() {
        for target in [Target::Srgb, Target::DisplayP3] {
            let (m, c) = (target.from_working(), target.compression());
            let mut ap1 = ap1_sweep();
            for w in ap1_surface(48) {
                for scale in [1e-4, 1e-3, 0.002, 0.95, 0.99, 1.02, 1.5, 4.0] {
                    ap1.push(w.map(|v| v * scale));
                }
            }
            for w in ap1 {
                let rgb = mul(&m, w);
                assert_eq!(
                    compress(rgb, target, &c),
                    compress_by_search(rgb, target, &c),
                    "{w:?}"
                );
            }
            for adobe in [false, true] {
                for code in (0..1 << 24).step_by(97) {
                    let rgb = source_rgb(code, adobe, target);
                    let got = compress(rgb, target, &c);
                    assert_eq!(got, compress_by_search(rgb, target, &c), "{code:06x}");
                }
            }
        }
    }

    /// Every 8-bit code of Display P3 and Adobe RGB in both targets (P3 in
    /// P3 is not compressed): a few seconds in release, minutes in debug.
    ///   cargo test --release -p genzo-color every_code -- --ignored
    #[test]
    #[ignore = "minutes in debug"]
    fn every_code() {
        for (adobe, target) in [
            (false, Target::Srgb),
            (true, Target::Srgb),
            (true, Target::DisplayP3),
        ] {
            let c = target.compression();
            std::thread::scope(|s| {
                for t in 0..16u32 {
                    s.spawn(move || {
                        for code in (t..1 << 24).step_by(16) {
                            let rgb = source_rgb(code, adobe, target);
                            let got = compress(rgb, target, &c);
                            assert_eq!(got, compress_by_search(rgb, target, &c), "{code:06x}");
                        }
                    });
                }
            });
        }
    }

    /// AP1 colors from black to the gamut's surface at unit brightness.
    fn ap1_sweep() -> Vec<[f64; 3]> {
        let mut out = vec![];
        for w in ap1_surface(48) {
            for scale in [0.01, 0.05, 0.2, 0.5, 0.75, 1.0] {
                out.push(w.map(|v| v * scale));
            }
        }
        out
    }

    fn hue_shift(a: f64, b: f64) -> f64 {
        ((a - b + 540.0) % 360.0 - 180.0).abs()
    }

    /// Criterion 9, read as: after compression no channel leaves 0 to 1
    /// for a color whose lightness is inside 0 to 1 (the clamp after it only
    /// catches what is brighter than white); colors inside by more than the
    /// threshold are unchanged; compressed colors keep their hue.
    #[test]
    fn criterion_9_over_the_ap1_gamut() {
        for target in [Target::Srgb, Target::DisplayP3] {
            let (m, c) = (target.from_working(), target.compression());
            let (mut outside, mut kept, mut moved, mut dh, mut dl) = (0f64, 0, 0, 0f64, 0f64);
            for w in ap1_sweep() {
                let rgb = mul(&m, w);
                let lab = xyz_to_oklab(mul(&target.to_xyz(), rgb));
                if lab[0] >= 1.0 || lab[0] <= 0.0 {
                    continue;
                }
                let out = compress(rgb, target, &c);
                for v in out {
                    outside = outside.max(-v).max(v - 1.0);
                }
                let ch = lab[1].hypot(lab[2]);
                let r = if ch > 0.0 {
                    ch / max_chroma(lab[0], [lab[1] / ch, lab[2] / ch], target)
                } else {
                    0.0
                };
                if r < c.threshold {
                    assert_eq!(out, rgb, "{target:?} {w:?}: changed inside the threshold");
                    kept += 1;
                } else {
                    moved += 1;
                    let (a, b) = (hue_of(rgb, target), hue_of(out, target));
                    if a[1] > 1e-3 {
                        dh = dh.max(hue_shift(a[2], b[2]));
                    }
                    dl = dl.max((a[0] - b[0]).abs());
                }
            }
            println!(
                "{target:?}: {kept} kept, {moved} compressed; hue {dh:.2e} deg, lightness {dl:.1e}, outside 0..1 by {outside:.1e}"
            );
            assert!(moved > 1000 && kept > 1000);
            assert!(outside <= 1e-6, "{target:?}: {outside}");
            assert!(dh <= 3.0, "{target:?}: hue {dh}");
            assert!(dl <= 1e-9, "{target:?}: lightness {dl}");
        }
    }

    #[test]
    fn the_curve_is_continuous_and_monotonic() {
        let c = SRGB_COMPRESSION;
        let f = |d| compress_distance(d, c.threshold, c.limit, c.power);
        assert!((f(c.limit) - 1.0).abs() < 1e-12);
        assert!((f(c.threshold + 1e-9) - c.threshold).abs() < 1e-8);
        let mut last = 0.0;
        for i in 0..=1000 {
            let v = f(i as f64 / 250.0);
            assert!(v >= last, "{i}");
            last = v;
        }
    }

    #[test]
    fn which_sources_fit() {
        use crate::encoding::InputEncoding;
        let fits_in = |e: InputEncoding, t| fits(&e.to_working, t);
        assert!(fits_in(InputEncoding::srgb(), Target::Srgb));
        assert!(fits_in(InputEncoding::srgb(), Target::DisplayP3));
        assert!(fits_in(InputEncoding::display_p3(), Target::DisplayP3));
        assert!(!fits_in(InputEncoding::display_p3(), Target::Srgb));
        assert!(!fits_in(InputEncoding::adobe_rgb(), Target::DisplayP3));
        assert!(!fits_in(
            InputEncoding::nclx(9, 1).unwrap(),
            Target::DisplayP3
        ));
        let gray = moxcms::ColorProfile::new_gray_with_gamma(2.2)
            .encode()
            .unwrap();
        assert!(fits_in(InputEncoding::icc(&gray).unwrap(), Target::Srgb));
        // Real profiles, a little off the formulas, still fit their own
        // target; a wider one does not.
        let lcms2 = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../imgio/tests/fixtures/images/png-icc-lcms2-srgb.icc"),
        )
        .unwrap();
        let icc = |p: moxcms::ColorProfile| InputEncoding::icc(&p.encode().unwrap()).unwrap();
        assert!(fits_in(InputEncoding::icc(&lcms2).unwrap(), Target::Srgb));
        assert!(fits_in(icc(moxcms::ColorProfile::new_srgb()), Target::Srgb));
        assert!(fits_in(
            icc(moxcms::ColorProfile::new_display_p3()),
            Target::DisplayP3
        ));
        assert!(!fits_in(
            icc(moxcms::ColorProfile::new_display_p3()),
            Target::Srgb
        ));
        assert!(!fits_in(
            icc(moxcms::ColorProfile::new_adobe_rgb()),
            Target::Srgb
        ));
    }

    #[test]
    fn an_srgb_source_comes_back_exactly() {
        use crate::encoding::InputEncoding;
        let e = InputEncoding::srgb();
        let mut worst = 0f64;
        for r in (0..=255u16).step_by(5) {
            for g in [0u16, 1, 128, 254, 255] {
                for b in [0u16, 7, 255] {
                    let codes = [r, g, b];
                    let out = encode(e.decode(codes.map(|v| v * 257)), Target::Srgb, false);
                    for k in 0..3 {
                        worst = worst.max((out[k] * 255.0 - f64::from(codes[k])).abs());
                    }
                }
            }
        }
        assert!(worst < 1e-4, "{worst} codes");
    }

    #[test]
    fn dither_is_triangular_and_deterministic() {
        let n: Vec<f32> = (0..256)
            .flat_map(|y| (0..256).map(move |x| dither(x, y)))
            .collect();
        assert!(n.iter().all(|v| *v > -1.0 && *v < 1.0));
        let mean = n.iter().map(|v| f64::from(*v)).sum::<f64>() / n.len() as f64;
        assert!(mean.abs() < 0.01, "mean {mean}");
        // A triangle on (-1, 1) holds 75 % of its mass within 0.5 of 0.
        let inner = n.iter().filter(|v| v.abs() < 0.5).count() as f64 / n.len() as f64;
        assert!((inner - 0.75).abs() < 0.01, "{inner}");
        assert_eq!(dither(17, 42), dither(17, 42));
        assert_ne!(dither(17, 42), dither(42, 17));
        for v in [0.0, 0.3, 0.5, 1.0] {
            for x in 0..64 {
                let q = i32::from(quantize(v, dither(x, 3)));
                assert!((q - (v * 255.0).round() as i32).abs() <= 1);
            }
        }
    }
}
