//! How a non-RAW image's encoded values become working-space light (DEV-01,
//! stage 4): per channel, a curve from the 16-bit code to linear light,
//! then a matrix into the working space (AP1, D65).
//!
//! The curves are tabulated for every 16-bit code, so applying them is a
//! lookup. Encodings: sRGB, Display P3 and Adobe RGB (1998) by formula; the
//! ITU-T H.273 code points HEIF uses (NCLX); and ICC profiles of the
//! matrix-shaper and gray kinds, read with moxcms. HDR transfers (PQ, HLG),
//! ICC profiles built on lookup tables and CMYK are unsupported in M1.

use crate::{Mat3, WORKING_TO_XYZ, invert, mat_mul, mul, rgb_to_xyz, srgb_decode};

/// Entries in each channel's table: one per 16-bit code.
pub const CODES: usize = 65536;

/// A channel's code to linear light.
#[derive(Debug, Clone, PartialEq)]
pub enum Curve {
    /// IEC 61966-2-1.
    Srgb,
    /// The inverse of the BT.709 OETF (H.273 transfer 1, 6, 14, 15).
    Bt709,
    /// A pure power: linear = v^gamma.
    Gamma(f64),
    Linear,
    /// A table over the 16-bit codes, linear light per code (an ICC curve).
    Table(Box<[f32]>),
}

impl Curve {
    /// Linear light for a value 0 to 1.
    pub fn eval(&self, v: f64) -> f64 {
        match self {
            Curve::Srgb => srgb_decode(v),
            Curve::Bt709 => {
                // H.273, table 3: alpha 1.099..., beta 0.0180...
                const ALPHA: f64 = 1.099_296_826_809_442;
                const BETA: f64 = 0.018_053_968_510_807;
                if v < 4.5 * BETA {
                    v / 4.5
                } else {
                    ((v + ALPHA - 1.0) / ALPHA).powf(1.0 / 0.45)
                }
            }
            Curve::Gamma(g) => v.max(0.0).powf(*g),
            Curve::Linear => v,
            Curve::Table(t) => {
                let i = (v.clamp(0.0, 1.0) * 65535.0).round() as usize;
                f64::from(t[i])
            }
        }
    }

    /// Linear light for a 16-bit code.
    pub fn code(&self, code: u16) -> f32 {
        match self {
            Curve::Table(t) => t[usize::from(code)],
            _ => self.eval(f64::from(code) / 65535.0) as f32,
        }
    }
}

/// An input encoding: three curves and the matrix from their linear light
/// to the working space.
#[derive(Debug, Clone, PartialEq)]
pub struct InputEncoding {
    pub curves: [Curve; 3],
    /// Linear source RGB to the working space.
    pub to_working: Mat3,
    /// For messages: what the encoding is.
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EncodingError {
    #[error("unsupported in M1: {0}")]
    Unsupported(String),
    #[error("unreadable ICC profile: {0}")]
    Icc(String),
}

/// The D65 white the working space uses, as xy.
const D65: [f64; 2] = [0.3127, 0.3290];

fn working_from_xyz() -> Mat3 {
    invert(&WORKING_TO_XYZ).expect("the working space is invertible")
}

/// The linearized Bradford transform from XYZ white `from` to `to`.
pub fn bradford(from: [f64; 3], to: [f64; 3]) -> Mat3 {
    const MB: Mat3 = [
        [0.8951, 0.2664, -0.1614],
        [-0.7502, 1.7135, 0.0367],
        [0.0389, -0.0685, 1.0296],
    ];
    let (a, b) = (mul(&MB, from), mul(&MB, to));
    let d = [
        [b[0] / a[0], 0.0, 0.0],
        [0.0, b[1] / a[1], 0.0],
        [0.0, 0.0, b[2] / a[2]],
    ];
    mat_mul(&mat_mul(&invert(&MB).expect("invertible"), &d), &MB)
}

fn xy_to_xyz([x, y]: [f64; 2]) -> [f64; 3] {
    [x / y, 1.0, (1.0 - x - y) / y]
}

impl InputEncoding {
    /// RGB with these primaries and white (xy) and one curve.
    pub fn from_primaries(
        name: &str,
        primaries: [[f64; 2]; 3],
        white: [f64; 2],
        curve: Curve,
    ) -> Self {
        let to_xyz = rgb_to_xyz(primaries, white);
        // A white other than D65 is adapted to it.
        let adapt = bradford(xy_to_xyz(white), xy_to_xyz(D65));
        InputEncoding {
            curves: [curve.clone(), curve.clone(), curve],
            to_working: mat_mul(&working_from_xyz(), &mat_mul(&adapt, &to_xyz)),
            name: name.into(),
        }
    }

    pub fn srgb() -> Self {
        Self::from_primaries("sRGB", BT709, D65, Curve::Srgb)
    }

    pub fn display_p3() -> Self {
        Self::from_primaries("Display P3", P3, D65, Curve::Srgb)
    }

    /// Adobe RGB (1998): its gamma is 563/256, not 2.2.
    pub fn adobe_rgb() -> Self {
        Self::from_primaries(
            "Adobe RGB (1998)",
            [[0.64, 0.33], [0.21, 0.71], [0.15, 0.06]],
            D65,
            Curve::Gamma(563.0 / 256.0),
        )
    }

    /// The ITU-T H.273 colour primaries and transfer characteristics a
    /// HEIF image's NCLX box gives. The matrix coefficients and range do not
    /// enter: libheif has converted the samples to full-range RGB already.
    pub fn nclx(primaries: u16, transfer: u16) -> Result<Self, EncodingError> {
        let (p, white, pname) = match primaries {
            // 2: unspecified, taken as BT.709.
            1 | 2 => (BT709, D65, "BT.709"),
            5 => (
                [[0.64, 0.33], [0.29, 0.60], [0.15, 0.06]],
                D65,
                "BT.601 625",
            ),
            6 | 7 => (
                [[0.630, 0.340], [0.310, 0.595], [0.155, 0.070]],
                D65,
                "BT.601 525",
            ),
            9 => (
                [[0.708, 0.292], [0.170, 0.797], [0.131, 0.046]],
                D65,
                "BT.2020",
            ),
            11 => (P3, [0.314, 0.351], "DCI-P3"),
            12 => (P3, D65, "P3 D65"),
            other => {
                return Err(EncodingError::Unsupported(format!(
                    "H.273 colour primaries {other}"
                )));
            }
        };
        let curve = match transfer {
            // 2: unspecified, taken as sRGB.
            2 | 13 => Curve::Srgb,
            1 | 6 | 14 | 15 => Curve::Bt709,
            4 => Curve::Gamma(2.2),
            5 => Curve::Gamma(2.8),
            8 => Curve::Linear,
            16 | 18 => {
                return Err(EncodingError::Unsupported(format!(
                    "an HDR transfer (H.273 transfer {transfer}, {})",
                    if transfer == 16 { "PQ" } else { "HLG" }
                )));
            }
            other => {
                return Err(EncodingError::Unsupported(format!(
                    "H.273 transfer {other}"
                )));
            }
        };
        Ok(Self::from_primaries(
            &format!("{pname}, H.273 transfer {transfer}"),
            p,
            white,
            curve,
        ))
    }

    /// An embedded ICC profile: a matrix-shaper RGB profile or a gray one.
    pub fn icc(bytes: &[u8]) -> Result<Self, EncodingError> {
        use moxcms::{ColorProfile, DataColorSpace};
        let profile = ColorProfile::new_from_slice(bytes)
            .map_err(|e| EncodingError::Icc(format!("{e:?}")))?;
        let table =
            |trc: Result<Box<[f32; CODES]>, moxcms::CmsError>| -> Result<Curve, EncodingError> {
                let t = trc.map_err(|e| EncodingError::Icc(format!("a tone curve: {e:?}")))?;
                if t.iter().any(|v| !v.is_finite()) {
                    return Err(EncodingError::Icc(
                        "a tone curve with non-finite values".into(),
                    ));
                }
                Ok(Curve::Table(t.to_vec().into_boxed_slice()))
            };
        match profile.color_space {
            DataColorSpace::Rgb if profile.is_matrix_shaper() => {
                let curves = [
                    table(profile.build_r_linearize_table::<u16, CODES, 16>(false))?,
                    table(profile.build_g_linearize_table::<u16, CODES, 16>(false))?,
                    table(profile.build_b_linearize_table::<u16, CODES, 16>(false))?,
                ];
                // The colorants, as columns: linear RGB to the PCS (XYZ, D50).
                let c = [
                    profile.red_colorant,
                    profile.green_colorant,
                    profile.blue_colorant,
                ];
                let to_pcs = [
                    [c[0].x, c[1].x, c[2].x],
                    [c[0].y, c[1].y, c[2].y],
                    [c[0].z, c[1].z, c[2].z],
                ];
                if to_pcs.iter().flatten().any(|v| !v.is_finite()) || invert(&to_pcs).is_none() {
                    return Err(EncodingError::Icc("colorants that do not span RGB".into()));
                }
                // Adapted from the profile's own white (its colorants' sum),
                // so a neutral lands exactly on D65 whatever D50 it used.
                let white = mul(&to_pcs, [1.0; 3]);
                if white[1] <= 0.0 {
                    return Err(EncodingError::Icc("a white with no luminance".into()));
                }
                let white = white.map(|v| v / white[1]);
                let adapt = bradford(white, xy_to_xyz(D65));
                let to_working = mat_mul(&working_from_xyz(), &mat_mul(&adapt, &to_pcs));
                // A white far from any real one can null a cone response.
                if to_working.iter().flatten().any(|v| !v.is_finite()) {
                    return Err(EncodingError::Icc("a white that cannot be adapted".into()));
                }
                Ok(InputEncoding {
                    curves,
                    to_working,
                    name: "an ICC matrix-shaper profile".into(),
                })
            }
            DataColorSpace::Gray => {
                let curve = table(profile.build_gray_linearize_table::<u16, CODES, 16>())?;
                // Gray keeps its neutral: equal channels in, equal out.
                Ok(InputEncoding {
                    curves: [curve.clone(), curve.clone(), curve],
                    to_working: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
                    name: "an ICC gray profile".into(),
                })
            }
            DataColorSpace::Rgb => Err(EncodingError::Unsupported(
                "an RGB ICC profile built on lookup tables".into(),
            )),
            other => Err(EncodingError::Unsupported(format!(
                "an ICC profile for {other:?}"
            ))),
        }
    }

    /// Every channel's table, channel after channel: linear light per code.
    pub fn lut(&self) -> Vec<f32> {
        let mut out = Vec::with_capacity(3 * CODES);
        for c in &self.curves {
            out.extend((0..=u16::MAX).map(|code| c.code(code)));
        }
        out
    }

    /// One pixel's codes in the working space.
    pub fn decode(&self, rgb: [u16; 3]) -> [f64; 3] {
        let lin = [0, 1, 2].map(|c| f64::from(self.curves[c].code(rgb[c])));
        mul(&self.to_working, lin)
    }
}

const BT709: [[f64; 2]; 3] = [[0.64, 0.33], [0.30, 0.60], [0.15, 0.06]];
const P3: [[f64; 2]; 3] = [[0.680, 0.320], [0.265, 0.690], [0.150, 0.060]];

#[cfg(test)]
mod tests {
    use super::*;
    use moxcms::{
        ColorPrimaries, ColorProfile, DataColorSpace, Layout, ToneReprCurve, TransformOptions, XyY,
    };

    /// The working space as a moxcms profile: linear AP1, D65 white.
    fn working_profile() -> ColorProfile {
        let mut p = ColorProfile::new_srgb();
        p.update_rgb_colorimetry(
            XyY {
                x: 0.3127,
                y: 0.3290,
                yb: 1.0,
            },
            ColorPrimaries::ACES_CG,
        );
        let linear = ToneReprCurve::Parametric(vec![1.0]);
        p.red_trc = Some(linear.clone());
        p.green_trc = Some(linear.clone());
        p.blue_trc = Some(linear);
        p
    }

    /// Codes on a grid through the cube, 8-bit steps widened as imgio does.
    fn grid() -> Vec<[u16; 3]> {
        let steps: Vec<u16> = [0u16, 1, 3, 10, 40, 64, 100, 128, 160, 200, 230, 250, 255]
            .iter()
            .map(|v| v * 257)
            .collect();
        let mut out = vec![];
        for &r in &steps {
            for &g in &steps {
                for &b in &steps {
                    out.push([r, g, b]);
                }
            }
        }
        out
    }

    /// ΔE2000 between two working-space colors.
    fn delta_e(a: [f64; 3], b: [f64; 3]) -> f64 {
        let lab = |v| crate::xyz_to_lab(mul(&WORKING_TO_XYZ, v), crate::D65_WHITE);
        crate::delta_e_2000(lab(a), lab(b))
    }

    /// The largest difference between our decode and moxcms's own transform,
    /// in linear light and in ΔE2000.
    fn against_moxcms(enc: &InputEncoding, src: &ColorProfile) -> (f64, f64) {
        let options = TransformOptions {
            allow_use_cicp_transfer: false,
            ..Default::default()
        };
        let t = src
            .create_transform_f32(Layout::Rgb, &working_profile(), Layout::Rgb, options)
            .unwrap();
        let codes = grid();
        let input: Vec<f32> = codes
            .iter()
            .flatten()
            .map(|&c| f32::from(c) / 65535.0)
            .collect();
        let mut output = vec![0f32; input.len()];
        t.transform(&input, &mut output).unwrap();
        let (mut worst, mut de) = (0f64, 0f64);
        for (i, rgb) in codes.iter().enumerate() {
            let ours = enc.decode(*rgb);
            let theirs = [0, 1, 2].map(|c| f64::from(output[3 * i + c]));
            for c in 0..3 {
                worst = worst.max((ours[c] - theirs[c]).abs());
            }
            de = de.max(delta_e(ours, theirs));
        }
        (worst, de)
    }

    #[test]
    fn analytic_encodings_match_moxcms() {
        for (enc, src) in [
            (InputEncoding::srgb(), ColorProfile::new_srgb()),
            (InputEncoding::display_p3(), ColorProfile::new_display_p3()),
            (InputEncoding::adobe_rgb(), ColorProfile::new_adobe_rgb()),
        ] {
            // moxcms takes D65 as (0.31272, 0.32903), the working space as
            // (0.3127, 0.3290): that, and its f32 path, is most of the gap.
            let (worst, de) = against_moxcms(&enc, &src);
            println!("{}: from moxcms {worst:.2e}, ΔE2000 {de:.3}", enc.name);
            assert!(worst < 5e-4, "{}: {worst}", enc.name);
            // DEV-01, criterion 8 asks this of Adobe RGB; all three meet it.
            assert!(de < 0.5, "{}: ΔE {de}", enc.name);
        }
    }

    #[test]
    fn icc_profiles_match_moxcms_and_the_analytic_route() {
        for (analytic, src) in [
            (InputEncoding::srgb(), ColorProfile::new_srgb()),
            (InputEncoding::display_p3(), ColorProfile::new_display_p3()),
            (InputEncoding::adobe_rgb(), ColorProfile::new_adobe_rgb()),
        ] {
            let bytes = src.encode().unwrap();
            let icc = InputEncoding::icc(&bytes).unwrap();
            let reread = ColorProfile::new_from_slice(&bytes).unwrap();
            let (worst, de) = against_moxcms(&icc, &reread);
            let (mut apart, mut apart_de) = (0f64, 0f64);
            for rgb in grid() {
                let (a, b) = (icc.decode(rgb), analytic.decode(rgb));
                apart = (0..3).fold(apart, |m, c| m.max((a[c] - b[c]).abs()));
                apart_de = apart_de.max(delta_e(a, b));
            }
            println!(
                "{} as ICC: from moxcms {worst:.2e} (ΔE {de:.3}), from the formula {apart:.2e} (ΔE {apart_de:.3})",
                analytic.name
            );
            assert!(
                worst < 5e-4 && de < 0.5,
                "{}: {worst}, ΔE {de}",
                analytic.name
            );
            assert!(
                apart < 5e-4 && apart_de < 0.5,
                "{}: {apart}, ΔE {apart_de}",
                analytic.name
            );
        }
    }

    #[test]
    fn neutrals_stay_neutral() {
        let mut all = vec![
            InputEncoding::srgb(),
            InputEncoding::display_p3(),
            InputEncoding::adobe_rgb(),
            InputEncoding::nclx(11, 13).unwrap(),
            InputEncoding::nclx(9, 1).unwrap(),
            InputEncoding::nclx(5, 6).unwrap(),
        ];
        for p in [
            ColorProfile::new_srgb(),
            ColorProfile::new_display_p3(),
            ColorProfile::new_adobe_rgb(),
        ] {
            all.push(InputEncoding::icc(&p.encode().unwrap()).unwrap());
        }
        all.push(
            InputEncoding::icc(&ColorProfile::new_gray_with_gamma(2.2).encode().unwrap()).unwrap(),
        );
        for enc in &all {
            for code in [257u16, 1000, 30000, 65535] {
                let v = enc.decode([code; 3]);
                let spread = v.iter().cloned().fold(f64::MIN, f64::max)
                    - v.iter().cloned().fold(f64::MAX, f64::min);
                assert!(
                    spread <= 1e-6 * v[1].max(1e-6),
                    "{} at {code}: {v:?}",
                    enc.name
                );
            }
            let white = enc.decode([65535; 3]);
            assert!(
                (white[1] - 1.0).abs() < 1e-4,
                "{}: white {white:?}",
                enc.name
            );
        }
    }

    #[test]
    fn nclx_code_points() {
        assert_eq!(
            InputEncoding::nclx(1, 13).unwrap().to_working,
            InputEncoding::srgb().to_working
        );
        assert_eq!(
            InputEncoding::nclx(12, 13).unwrap().to_working,
            InputEncoding::display_p3().to_working
        );
        assert_eq!(InputEncoding::nclx(1, 8).unwrap().curves[0], Curve::Linear);
        for transfer in [16, 18, 3, 99] {
            assert!(matches!(
                InputEncoding::nclx(1, transfer),
                Err(EncodingError::Unsupported(_))
            ));
        }
        assert!(matches!(
            InputEncoding::nclx(22, 13),
            Err(EncodingError::Unsupported(_))
        ));
        // BT.709's curve meets its linear segment continuously.
        let c = Curve::Bt709;
        let knee = 4.5 * 0.018_053_968_510_807;
        assert!((c.eval(knee - 1e-9) - c.eval(knee + 1e-9)).abs() < 1e-7);
        assert!((c.eval(1.0) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn gray_profiles_follow_their_curve() {
        let enc =
            InputEncoding::icc(&ColorProfile::new_gray_with_gamma(2.2).encode().unwrap()).unwrap();
        for code in [0u16, 100, 5000, 32768, 65535] {
            // An ICC gamma is u8Fixed8: 2.2 is stored as 563/256.
            let want = (f64::from(code) / 65535.0).powf(563.0 / 256.0);
            let got = enc.decode([code; 3]);
            assert!(
                (got[0] - want).abs() < 1e-4,
                "{code}: {got:?} against {want}"
            );
        }
    }

    #[test]
    fn unsupported_and_broken_profiles_are_errors() {
        let mut cmyk = ColorProfile::new_srgb();
        cmyk.color_space = DataColorSpace::Cmyk;
        if let Ok(bytes) = cmyk.encode() {
            assert!(InputEncoding::icc(&bytes).is_err());
        }
        let mut no_curves = ColorProfile::new_srgb();
        no_curves.red_trc = None;
        if let Ok(bytes) = no_curves.encode() {
            assert!(InputEncoding::icc(&bytes).is_err());
        }
        let good = ColorProfile::new_srgb().encode().unwrap();
        assert!(InputEncoding::icc(&[]).is_err());
        assert!(InputEncoding::icc(&good[..good.len() / 2]).is_err());
        let mut flipped = good.clone();
        for i in (0..flipped.len()).step_by(7) {
            flipped[i] ^= 0x5a;
        }
        let _ = InputEncoding::icc(&flipped);
    }

    /// The icc fuzz target's seeds: `cargo test -p genzo-color -- --ignored
    /// write_fuzz_seeds`.
    #[test]
    #[ignore]
    fn write_fuzz_seeds() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fuzz/seeds/icc");
        std::fs::create_dir_all(&dir).unwrap();
        for (name, p) in [
            ("srgb", ColorProfile::new_srgb()),
            ("display-p3", ColorProfile::new_display_p3()),
            ("adobe-rgb", ColorProfile::new_adobe_rgb()),
            ("bt2020", ColorProfile::new_bt2020()),
            ("gray-2.2", ColorProfile::new_gray_with_gamma(2.2)),
        ] {
            std::fs::write(dir.join(format!("{name}.icc")), p.encode().unwrap()).unwrap();
        }
    }

    #[test]
    fn lut_holds_every_code() {
        let enc = InputEncoding::adobe_rgb();
        let lut = enc.lut();
        assert_eq!(lut.len(), 3 * CODES);
        assert_eq!(lut[CODES + 65535], 1.0);
        assert_eq!(lut[2 * CODES + 1234], enc.curves[2].code(1234));
    }
}
