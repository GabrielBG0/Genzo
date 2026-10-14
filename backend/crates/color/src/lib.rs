//! Color math: transfer functions, primaries and matrices.
//!
//! This is the CPU reference for the color math only. The GPU shaders are
//! checked against it; no other stage has a CPU version.
//! ICC handling (lcms2 or moxcms) waits for the M0 license review.

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

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64, eps: f64) -> bool {
        (a - b).abs() < eps
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
        for m in [SRGB_TO_XYZ, DISPLAY_P3_TO_XYZ] {
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
