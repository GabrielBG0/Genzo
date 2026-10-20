//! Color math: transfer functions, primaries and matrices.
//!
//! This is the CPU reference for the color math only. The GPU shaders are
//! checked against it; no other stage has a CPU version. It also holds the
//! metrics the golden-image tests use (CIELAB and ΔE2000).
//! [`encoding`] reads non-RAW encodings, ICC profiles through moxcms
//! (cleared in docs/licenses.md).

pub mod camera;
pub mod encoding;
pub mod look;
pub mod output;
pub mod tone;
pub mod white_balance;

/// A 3×3 matrix in row-major order.
pub type Mat3 = [[f64; 3]; 3];

/// Linear sRGB (D65) to CIE XYZ (D65), from IEC 61966-2-1.
pub const SRGB_TO_XYZ: Mat3 = [
    [0.412_456_4, 0.357_576_1, 0.180_437_5],
    [0.212_672_9, 0.715_152_2, 0.072_175_0],
    [0.019_333_9, 0.119_192_0, 0.950_304_1],
];

/// Linear Display P3 (D65) to CIE XYZ (D65).
pub const DISPLAY_P3_TO_XYZ: Mat3 = [
    [0.486_570_9, 0.265_667_7, 0.198_217_3],
    [0.228_974_6, 0.691_738_5, 0.079_286_9],
    [0.000_000_0, 0.045_113_4, 1.043_944_4],
];

/// Linear Rec. 2020 (D65) to CIE XYZ (D65), from the ITU-R BT.2020 primaries.
/// Not the working space: Display P3's red lies just outside it.
pub const REC2020_TO_XYZ: Mat3 = [
    [0.636_958_0, 0.144_616_9, 0.168_881_0],
    [0.262_700_2, 0.677_998_1, 0.059_301_7],
    [0.000_000_0, 0.028_072_7, 1.060_985_1],
];

/// The working space to CIE XYZ (D65): linear light on the ACES AP1
/// primaries with a D65 white, so no chromatic adaptation is needed. It fully
/// contains Display P3 and Rec. 2020.
/// Full f64 precision, so the D65 white maps to (1, 1, 1) exactly enough
/// for the camera color tests.
pub const WORKING_TO_XYZ: Mat3 = [
    [
        0.647_507_198_707_060_6,
        0.134_379_140_087_524_44,
        0.168_569_588_257_086_57,
    ],
    [
        0.266_086_408_444_837,
        0.675_967_795_591_789_5,
        0.057_945_795_963_373_51,
    ],
    [
        -0.005_448_868_432_317_436,
        0.004_072_095_154_167_411,
        1.090_434_524_038_028_7,
    ],
];

/// The RGB to XYZ matrix of a space given its primaries and white point as
/// CIE xy chromaticities.
pub fn rgb_to_xyz(primaries: [[f64; 2]; 3], white: [f64; 2]) -> Mat3 {
    let col = |[x, y]: [f64; 2]| [x / y, 1.0, (1.0 - x - y) / y];
    let [r, g, b] = primaries.map(col);
    let p = [[r[0], g[0], b[0]], [r[1], g[1], b[1]], [r[2], g[2], b[2]]];
    let s = mul(
        &invert(&p).expect("primaries are not collinear"),
        col(white),
    );
    p.map(|row| [row[0] * s[0], row[1] * s[1], row[2] * s[2]])
}

/// The D65 white point in XYZ, with Y = 1.
pub const D65_WHITE: [f64; 3] = [0.950_47, 1.0, 1.088_83];

/// sRGB encoding (also used by Display P3): linear light to code value.
pub fn srgb_encode(linear: f64) -> f64 {
    if linear <= 0.003_130_8 {
        12.92 * linear
    } else {
        1.055 * linear.powf(1.0 / 2.4) - 0.055
    }
}

/// sRGB decoding: code value to linear light.
pub fn srgb_decode(encoded: f64) -> f64 {
    if encoded <= 0.040_45 {
        encoded / 12.92
    } else {
        ((encoded + 0.055) / 1.055).powf(2.4)
    }
}

pub fn mul(m: &Mat3, v: [f64; 3]) -> [f64; 3] {
    [
        m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
        m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
        m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
    ]
}

pub fn mat_mul(a: &Mat3, b: &Mat3) -> Mat3 {
    let mut out = [[0.0; 3]; 3];
    for (i, row) in out.iter_mut().enumerate() {
        for (j, cell) in row.iter_mut().enumerate() {
            *cell = (0..3).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    out
}

/// Inverse of a 3×3 matrix, or `None` if it is singular.
pub fn invert(m: &Mat3) -> Option<Mat3> {
    let [[a, b, c], [d, e, f], [g, h, i]] = *m;
    let det = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g);
    if det.abs() < 1e-12 {
        return None;
    }
    let r = 1.0 / det;
    Some([
        [
            (e * i - f * h) * r,
            (c * h - b * i) * r,
            (b * f - c * e) * r,
        ],
        [
            (f * g - d * i) * r,
            (a * i - c * g) * r,
            (c * d - a * f) * r,
        ],
        [
            (d * h - e * g) * r,
            (b * g - a * h) * r,
            (a * e - b * d) * r,
        ],
    ])
}

/// CIE XYZ to CIELAB, relative to `white`.
pub fn xyz_to_lab(xyz: [f64; 3], white: [f64; 3]) -> [f64; 3] {
    const E: f64 = 216.0 / 24389.0;
    const K: f64 = 24389.0 / 27.0;
    let f = |t: f64| {
        if t > E {
            t.cbrt()
        } else {
            (K * t + 16.0) / 116.0
        }
    };
    let [fx, fy, fz] = [0, 1, 2].map(|i| f(xyz[i] / white[i]));
    [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
}

/// The CIEDE2000 color difference between two CIELAB colors, with
/// kL = kC = kH = 1.
pub fn delta_e_2000(lab1: [f64; 3], lab2: [f64; 3]) -> f64 {
    use std::f64::consts::PI;
    let [l1, a1, b1] = lab1;
    let [l2, a2, b2] = lab2;
    let c_bar = (a1.hypot(b1) + a2.hypot(b2)) / 2.0;
    let c7 = c_bar.powi(7);
    let g = 0.5 * (1.0 - (c7 / (c7 + 25f64.powi(7))).sqrt());
    let (a1p, a2p) = ((1.0 + g) * a1, (1.0 + g) * a2);
    let (c1p, c2p) = (a1p.hypot(b1), a2p.hypot(b2));
    let hue = |b: f64, a: f64| {
        if a == 0.0 && b == 0.0 {
            0.0
        } else {
            b.atan2(a).rem_euclid(2.0 * PI)
        }
    };
    let (h1p, h2p) = (hue(b1, a1p), hue(b2, a2p));

    let dl = l2 - l1;
    let dc = c2p - c1p;
    let dh = if c1p * c2p == 0.0 {
        0.0
    } else {
        let d = h2p - h1p;
        if d > PI {
            d - 2.0 * PI
        } else if d < -PI {
            d + 2.0 * PI
        } else {
            d
        }
    };
    let dh_big = 2.0 * (c1p * c2p).sqrt() * (dh / 2.0).sin();

    let l_bar = (l1 + l2) / 2.0;
    let c_bar_p = (c1p + c2p) / 2.0;
    let h_bar = if c1p * c2p == 0.0 {
        h1p + h2p
    } else if (h1p - h2p).abs() <= PI {
        (h1p + h2p) / 2.0
    } else if h1p + h2p < 2.0 * PI {
        (h1p + h2p + 2.0 * PI) / 2.0
    } else {
        (h1p + h2p - 2.0 * PI) / 2.0
    };
    let t = 1.0 - 0.17 * (h_bar - PI / 6.0).cos()
        + 0.24 * (2.0 * h_bar).cos()
        + 0.32 * (3.0 * h_bar + PI / 30.0).cos()
        - 0.20 * (4.0 * h_bar - 63.0 * PI / 180.0).cos();
    let l50 = (l_bar - 50.0).powi(2);
    let s_l = 1.0 + 0.015 * l50 / (20.0 + l50).sqrt();
    let s_c = 1.0 + 0.045 * c_bar_p;
    let s_h = 1.0 + 0.015 * c_bar_p * t;
    let d_theta = 30.0 * (-((h_bar.to_degrees() - 275.0) / 25.0).powi(2)).exp();
    let cp7 = c_bar_p.powi(7);
    let r_c = 2.0 * (cp7 / (cp7 + 25f64.powi(7))).sqrt();
    let r_t = -(2.0 * d_theta.to_radians()).sin() * r_c;

    let (tl, tc, th) = (dl / s_l, dc / s_c, dh_big / s_h);
    (tl * tl + tc * tc + th * th + r_t * tc * th).sqrt()
}

/// ΔE2000 between two encoded sRGB colors (0 to 1 per channel).
pub fn delta_e_2000_srgb(a: [f64; 3], b: [f64; 3]) -> f64 {
    let lab = |c: [f64; 3]| xyz_to_lab(mul(&SRGB_TO_XYZ, c.map(srgb_decode)), D65_WHITE);
    delta_e_2000(lab(a), lab(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64, eps: f64) -> bool {
        (a - b).abs() < eps
    }

    #[test]
    fn delta_e_2000_matches_sharma_data() {
        // Pairs 1, 2, 7 and 17 from Sharma, Wu and Dalal (2005), table 1.
        let pairs = [
            ([50.0, 2.6772, -79.7751], [50.0, 0.0, -82.7485], 2.0425),
            ([50.0, 3.1571, -77.2803], [50.0, 0.0, -82.7485], 2.8615),
            ([50.0, 0.0, 0.0], [50.0, -1.0, 2.0], 2.3669),
            ([50.0, 2.5, 0.0], [73.0, 25.0, -18.0], 27.1492),
        ];
        for (a, b, want) in pairs {
            let got = delta_e_2000(a, b);
            assert!(close(got, want, 1e-4), "{a:?} {b:?}: {got} != {want}");
            assert!(close(delta_e_2000(b, a), want, 1e-4));
        }
    }

    #[test]
    fn white_is_l100() {
        let lab = xyz_to_lab(D65_WHITE, D65_WHITE);
        assert!(close(lab[0], 100.0, 1e-9) && lab[1].abs() < 1e-9 && lab[2].abs() < 1e-9);
    }

    #[test]
    fn matrices_match_their_primaries() {
        const D65: [f64; 2] = [0.3127, 0.3290];
        let cases = [
            (
                SRGB_TO_XYZ,
                [[0.64, 0.33], [0.30, 0.60], [0.15, 0.06]],
                5e-4,
            ),
            (
                DISPLAY_P3_TO_XYZ,
                [[0.680, 0.320], [0.265, 0.690], [0.150, 0.060]],
                1e-6,
            ),
            (
                REC2020_TO_XYZ,
                [[0.708, 0.292], [0.170, 0.797], [0.131, 0.046]],
                1e-6,
            ),
            (
                WORKING_TO_XYZ,
                [[0.713, 0.293], [0.165, 0.830], [0.128, 0.044]],
                1e-15,
            ),
        ];
        // sRGB's published matrix uses a slightly different D65, hence its tolerance.
        for (m, primaries, eps) in cases {
            let derived = rgb_to_xyz(primaries, D65);
            for (a, b) in m.iter().flatten().zip(derived.iter().flatten()) {
                assert!(close(*a, *b, eps), "{a} != {b}");
            }
        }
    }

    #[test]
    fn working_space_contains_display_p3_and_rec2020() {
        let to_working = invert(&WORKING_TO_XYZ).unwrap();
        for space in [DISPLAY_P3_TO_XYZ, REC2020_TO_XYZ] {
            let m = mat_mul(&to_working, &space);
            for primary in [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]] {
                for c in mul(&m, primary) {
                    assert!(c > -1e-6, "{c}");
                }
            }
        }
    }

    #[test]
    fn rec2020_does_not_contain_display_p3() {
        // Why the working space is not Rec. 2020.
        let m = mat_mul(&invert(&REC2020_TO_XYZ).unwrap(), &DISPLAY_P3_TO_XYZ);
        assert!(mul(&m, [1.0, 0.0, 0.0])[2] < -1e-3);
    }

    #[test]
    fn transfer_round_trip() {
        for i in 0..=100 {
            let x = i as f64 / 100.0;
            assert!(close(srgb_decode(srgb_encode(x)), x, 1e-12), "{x}");
        }
        assert!(close(srgb_encode(0.18), 0.461_356, 1e-5));
    }

    #[test]
    fn white_maps_to_d65() {
        for m in [
            SRGB_TO_XYZ,
            DISPLAY_P3_TO_XYZ,
            REC2020_TO_XYZ,
            WORKING_TO_XYZ,
        ] {
            let [x, y, z] = mul(&m, [1.0, 1.0, 1.0]);
            assert!(close(x, 0.950_47, 1e-4) && close(y, 1.0, 1e-4) && close(z, 1.088_83, 1e-3));
        }
    }

    #[test]
    fn display_p3_contains_srgb() {
        // Every sRGB primary is inside Display P3: non-negative P3 coordinates.
        let to_p3 = mat_mul(&invert(&DISPLAY_P3_TO_XYZ).unwrap(), &SRGB_TO_XYZ);
        for primary in [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]] {
            for c in mul(&to_p3, primary) {
                assert!(c > -1e-6, "{c}");
            }
        }
    }

    #[test]
    fn inverse_is_inverse() {
        let inv = invert(&SRGB_TO_XYZ).unwrap();
        let id = mat_mul(&SRGB_TO_XYZ, &inv);
        for (i, row) in id.iter().enumerate() {
            for (j, v) in row.iter().enumerate() {
                assert!(close(*v, if i == j { 1.0 } else { 0.0 }, 1e-9));
            }
        }
    }
}
