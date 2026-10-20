//! The Genzo Standard base tone curve (DEV-02): the Adobe DNG SDK 1.4's
//! default rendering after the camera matrix. In linear ProPhoto RGB, the
//! exposure ramp sets the black and white points per channel, then a tone
//! curve goes through RGBTone, which keeps hue. The curve is the file's own
//! ProfileToneCurve when it has a valid one, else the ACR3 default.
//!
//! This is the CPU reference; the GPU's stage 6 is checked against it. It
//! follows the SDK step by step (`dng_render.cpp`, `dng_reference.cpp`,
//! `dng_spline.cpp`, `dng_tone_curve.cpp`, `dng_1d_table.h/.cpp`,
//! `dng_color_space.cpp`), with the SDK's f32 and f64 operations in the same
//! order, and is tested against that SDK's own results
//! (scripts/tone-reference.py). Genzo applies exposure as a gain before this
//! step, so the ramp's white is 1 here (DEV-02).
//!
//! Ported from the DNG SDK 1.4 as distributed in gopro/gpr, under this
//! notice. Changes: Rust; the ramp is evaluated directly, not through a
//! table; the split into functions is Genzo's.
//!
//! Copyright (c) 2006 - 2009, Adobe Systems Incorporated
//! All rights reserved.
//!
//! Redistribution and use in source and binary forms, with or without
//! modification, are permitted provided that the following conditions are
//! met:
//!
//! * Redistributions of source code must retain the above copyright notice,
//!   this list of conditions and the following disclaimer.
//! * Redistributions in binary form must reproduce the above copyright
//!   notice, this list of conditions and the following disclaimer in the
//!   documentation and/or other materials provided with the distribution.
//! * Neither the name of Adobe Systems Incorporated, nor the names of its
//!   contributors may be used to endorse or promote products derived from
//!   this software without specific prior written permission.
//!
//! THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS
//! IS" AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO,
//! THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR
//! PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT OWNER OR
//! CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL,
//! EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO,
//! PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR
//! PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF
//! LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING
//! NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS
//! SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

mod acr3;

use crate::camera::{D50_XY, D65_XY, map_white_matrix, xy_to_xyz};
use crate::{Mat3, WORKING_TO_XYZ, invert, mat_mul, mul};

/// `dng_1d_table::kTableSize`: a curve is sampled at `i / 4096`.
pub const TABLE_SIZE: usize = 4096;

/// The SDK's default Shadows setting, which sets the default black.
pub const DEFAULT_SHADOWS: f64 = 5.0;

/// `dng_tone_curve_acr3_default::Evaluate`, in the SDK's f32 arithmetic.
pub fn acr3(x: f64) -> f64 {
    let n = acr3::ACR3.len();
    let y = (x as f32) * ((n - 1) as f32);
    let index = (y as i32).clamp(0, n as i32 - 2) as usize;
    let fract = y - index as f32;
    f64::from(acr3::ACR3[index] * (1.0 - fract) + acr3::ACR3[index + 1] * fract)
}

/// `dng_function_exposure_ramp`: 0 below the black, a line from the black
/// to the white, with a quadratic toe around the black.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ExposureRamp {
    slope: f64,
    black: f64,
    radius: f64,
    q_scale: f64,
}

impl ExposureRamp {
    /// The ramp from `black` to `white`, with the SDK's toe for `min_black`
    /// (dng_render passes the black itself).
    pub fn new(white: f64, black: f64, min_black: f64) -> Self {
        const MAX_CURVE_X: f64 = 0.5; // fraction of min_black
        const MAX_CURVE_Y: f64 = 1.0 / 16.0; // fraction of white
        let slope = 1.0 / (white - black);
        let radius = (MAX_CURVE_X * min_black).min(MAX_CURVE_Y / slope);
        let q_scale = if radius > 0.0 {
            slope / (4.0 * radius)
        } else {
            0.0
        };
        Self {
            slope,
            black,
            radius,
            q_scale,
        }
    }

    pub fn black(&self) -> f64 {
        self.black
    }

    pub fn radius(&self) -> f64 {
        self.radius
    }

    pub fn evaluate(&self, x: f64) -> f64 {
        if x <= self.black - self.radius {
            return 0.0;
        }
        if x >= self.black + self.radius {
            return ((x - self.black) * self.slope).min(1.0);
        }
        let y = x - (self.black - self.radius);
        self.q_scale * y * y
    }
}

/// `dng_spline_solver`: the C1 and C2 continuous cubic through a
/// ProfileToneCurve's points, constant past either end.
#[derive(Debug, Clone, PartialEq)]
pub struct Spline {
    x: Vec<f64>,
    y: Vec<f64>,
    s: Vec<f64>,
}

impl Spline {
    /// Solves the spline through `points` (x, y). The caller checks the
    /// points first ([`profile_curve`]); at least two are needed.
    pub fn solve(points: &[[f64; 2]]) -> Self {
        let x: Vec<f64> = points.iter().map(|p| p[0]).collect();
        let y: Vec<f64> = points.iter().map(|p| p[1]).collect();
        let count = x.len();
        assert!(count >= 2, "a spline needs two points");
        let (start, end) = (0, count);
        let mut a = x[start + 1] - x[start];
        let mut b = (y[start + 1] - y[start]) / a;
        let mut s = vec![0.0; count];
        s[start] = b;
        // Slopes here are a weighted average of the slopes to each of the
        // adjacent control points.
        for j in start + 2..end {
            let c = x[j] - x[j - 1];
            let d = (y[j] - y[j - 1]) / c;
            s[j - 1] = (b * c + d * a) / (a + c);
            a = c;
            b = d;
        }
        s[end - 1] = 2.0 * b - s[end - 2];
        s[start] = 2.0 * s[start] - s[start + 1];
        if end - start > 2 {
            let mut e = vec![0.0; count];
            let mut f = vec![0.0; count];
            let mut g = vec![0.0; count];
            f[start] = 0.5;
            e[end - 1] = 0.5;
            g[start] = 0.75 * (s[start] + s[start + 1]);
            g[end - 1] = 0.75 * (s[end - 2] + s[end - 1]);
            for j in start + 1..end - 1 {
                let a = (x[j + 1] - x[j - 1]) * 2.0;
                e[j] = (x[j + 1] - x[j]) / a;
                f[j] = (x[j] - x[j - 1]) / a;
                g[j] = 1.5 * s[j];
            }
            for j in start + 1..end {
                let a = 1.0 - f[j - 1] * e[j];
                if j != end - 1 {
                    f[j] /= a;
                }
                g[j] = (g[j] - g[j - 1] * e[j]) / a;
            }
            for j in (start..=end - 2).rev() {
                g[j] -= f[j] * g[j + 1];
            }
            s = g;
        }
        Self { x, y, s }
    }

    pub fn evaluate(&self, x: f64) -> f64 {
        let count = self.x.len();
        if x <= self.x[0] {
            return self.y[0];
        }
        if x >= self.x[count - 1] {
            return self.y[count - 1];
        }
        // Binary search for the index.
        let (mut lower, mut upper) = (1, count - 1);
        while upper > lower {
            let mid = (lower + upper) >> 1;
            if x == self.x[mid] {
                return self.y[mid];
            }
            if x > self.x[mid] {
                lower = mid + 1;
            } else {
                upper = mid;
            }
        }
        let j = lower;
        segment(
            x,
            self.x[j - 1],
            self.y[j - 1],
            self.s[j - 1],
            self.x[j],
            self.y[j],
            self.s[j],
        )
    }
}

/// `EvaluateSplineSegment`: the 1D Bezier with control values y0,
/// y0 + s0 × A, y1 − s1 × A, y1.
fn segment(x: f64, x0: f64, y0: f64, s0: f64, x1: f64, y1: f64, s1: f64) -> f64 {
    let a = x1 - x0;
    let b = (x - x0) / a;
    let c = (x1 - x) / a;
    ((y0 * (2.0 - c + b) + (s0 * a * b)) * (c * c))
        + ((y1 * (2.0 - b + c) - (s1 * a * c)) * (b * b))
}

/// Why a file's ProfileToneCurve is not used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CurveError {
    /// The tag was not in a form the SDK reads (type, count, size cap).
    Unreadable,
    /// Fewer than 2 points, a coordinate outside 0 to 1, or x not strictly
    /// increasing (`dng_tone_curve::IsValid`).
    Invalid,
}

/// A file's ProfileToneCurve (x, y pairs as stored) as a spline, if the SDK
/// would use it. An empty slice is the decoder's mark for an unreadable tag.
pub fn profile_curve(floats: &[f32]) -> Result<Spline, CurveError> {
    if floats.is_empty() || !floats.len().is_multiple_of(2) {
        return Err(CurveError::Unreadable);
    }
    let points: Vec<[f64; 2]> = floats
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&[x, y]| [f64::from(x), f64::from(y)])
        .collect();
    let in_range = |v: f64| (0.0..=1.0).contains(&v);
    let valid = points.len() >= 2
        && points.iter().all(|p| in_range(p[0]) && in_range(p[1]))
        && points.windows(2).all(|w| w[1][0] > w[0][0]);
    if !valid {
        return Err(CurveError::Invalid);
    }
    Ok(Spline::solve(&points))
}

/// The tone curve the base curve applies.
#[derive(Debug, Clone, PartialEq)]
pub enum ToneCurve {
    Acr3,
    Profile(Spline),
    /// Output-referred files with no valid profile curve (`dng_1d_identity`).
    Identity,
}

impl ToneCurve {
    pub fn evaluate(&self, x: f64) -> f64 {
        match self {
            Self::Acr3 => acr3(x),
            Self::Profile(s) => s.evaluate(x),
            Self::Identity => x,
        }
    }
}

/// A curve sampled as dng_render samples it (`dng_1d_table`, not
/// subsampled): 4097 values at `i / 4096`, plus the last one repeated, in
/// f32, interpolated linearly in f32.
#[derive(Debug, Clone, PartialEq)]
pub struct CurveTable {
    table: Vec<f32>,
}

impl CurveTable {
    pub fn new(curve: &ToneCurve) -> Self {
        let mut table: Vec<f32> = (0..=TABLE_SIZE)
            .map(|j| curve.evaluate(j as f64 * (1.0 / TABLE_SIZE as f64)) as f32)
            .collect();
        table.push(table[TABLE_SIZE]);
        Self { table }
    }

    /// The 4098 values, as the GPU reads them.
    pub fn values(&self) -> &[f32] {
        &self.table
    }

    /// `dng_1d_table::Interpolate`, for `x` in 0 to 1 (the SDK clips before
    /// it; this clamps the same way, `RefBaseline1DTable`).
    pub fn interpolate(&self, x: f32) -> f32 {
        let x = x.clamp(0.0, 1.0);
        let y = x * TABLE_SIZE as f32;
        let index = y as i32 as usize;
        let fract = y - index as f32;
        self.table[index] * (1.0 - fract) + self.table[index + 1] * fract
    }
}

/// `RefBaselineRGBTone` for one pixel: the largest and smallest channels go
/// through the curve, and the middle one keeps its fraction between them.
pub fn rgb_tone([r, g, b]: [f32; 3], table: &CurveTable) -> [f32; 3] {
    // (rr, gg, bb) from the largest, middle and smallest values.
    let tone = |hi: f32, mid: f32, lo: f32| {
        let hh = table.interpolate(hi);
        let ll = table.interpolate(lo);
        (hh, ll + ((hh - ll) * (mid - lo) / (hi - lo)), ll)
    };
    if r >= g {
        if g > b {
            // Case 1: r >= g > b
            let (rr, gg, bb) = tone(r, g, b);
            [rr, gg, bb]
        } else if b > r {
            // Case 2: b > r >= g
            let (bb, rr, gg) = tone(b, r, g);
            [rr, gg, bb]
        } else if b > g {
            // Case 3: r >= b > g
            let (rr, bb, gg) = tone(r, b, g);
            [rr, gg, bb]
        } else {
            // Case 4: r >= g == b
            let rr = table.interpolate(r);
            let gg = table.interpolate(g);
            [rr, gg, gg]
        }
    } else if r >= b {
        // Case 5: g > r >= b
        let (gg, rr, bb) = tone(g, r, b);
        [rr, gg, bb]
    } else if b > g {
        // Case 6: b > g > r
        let (bb, gg, rr) = tone(b, g, r);
        [rr, gg, bb]
    } else {
        // Case 7: g >= b > r
        let (gg, bb, rr) = tone(g, b, r);
        [rr, gg, bb]
    }
}

/// `dng_space_ProPhoto`'s matrix to the PCS (XYZ D50), with the SDK's
/// scaling so ProPhoto white lands on the PCS white exactly
/// (`dng_color_space::SetMatrixToPCS`).
pub fn prophoto_to_pcs() -> Mat3 {
    const M: Mat3 = [
        [0.7977, 0.1352, 0.0313],
        [0.2880, 0.7119, 0.0001],
        [0.0000, 0.0000, 0.8249],
    ];
    let w1 = mul(&M, [1.0, 1.0, 1.0]);
    let w2 = xy_to_xyz(D50_XY);
    let mut out = M;
    for (i, row) in out.iter_mut().enumerate() {
        let s = w2[i] / w1[i];
        for v in row.iter_mut() {
            *v *= s;
        }
    }
    out
}

/// The working space (AP1, D65) to linear ProPhoto RGB: the exact inverse
/// of stage 4's PCS to working tail (`ColorSpec::camera_to_working`), then
/// the SDK's PCS to ProPhoto (DEV-02, step 1).
pub fn working_to_prophoto() -> Mat3 {
    let xyz_to_working = invert(&WORKING_TO_XYZ).expect("the working space is invertible");
    let pcs_to_working = mat_mul(&xyz_to_working, &map_white_matrix(D50_XY, D65_XY));
    let working_to_pcs = invert(&pcs_to_working).expect("PCS to working is invertible");
    let pcs_to_prophoto = invert(&prophoto_to_pcs()).expect("ProPhoto is invertible");
    mat_mul(&pcs_to_prophoto, &working_to_pcs)
}

/// The SDK's BaselineExposure plus the profile's offset
/// (`TotalBaselineExposure`). `SetBaselineExposure` keeps the stored value
/// as a rational over 100, so it is rounded to 1/100 stop, half away from
/// zero; the offset is used as stored.
pub fn total_baseline_exposure(baseline_exposure: f64, offset: f64) -> f64 {
    (baseline_exposure * 100.0).round() / 100.0 + offset
}

/// What the base curve needs from the file (DEV-02).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CurveInputs {
    /// DNG ShadowScale as stored; the SDK uses it only when in (0, 1].
    pub shadow_scale: Option<f64>,
    /// DefaultBlackRender is 1 (None).
    pub no_default_black: bool,
    /// ColorimetricReference is 0.
    pub scene_referred: bool,
}

impl CurveInputs {
    /// The ShadowScale the SDK uses (`SetShadowScale`): 1 unless the stored
    /// value is in (0, 1].
    pub fn shadow_scale(&self) -> f64 {
        self.shadow_scale
            .filter(|s| *s > 0.0 && *s <= 1.0)
            .unwrap_or(1.0)
    }

    /// The ramp's black point in Genzo's units, after BaselineExposure and
    /// exposure were applied as gains (DEV-02, step 2): the SDK's black,
    /// `Shadows × ShadowScale × 0.001`, times `2^(total exposure)`, limited
    /// to 0.99 × the white of 1. `exposure` is BaselineExposure (the total)
    /// plus the user's exposure, in stops. 0 for an output-referred file or
    /// one whose profile asks for no default black.
    pub fn black(&self, exposure: f64) -> f64 {
        if !self.scene_referred || self.no_default_black {
            return 0.0;
        }
        let black = DEFAULT_SHADOWS * self.shadow_scale() * 0.001 * 2f64.powf(exposure);
        black.min(0.99)
    }
}

/// The curve the SDK picks: the profile's valid ProfileToneCurve, else ACR3
/// for scene-referred files and the identity for others. The error says why
/// a curve the file has is not used.
pub fn choose_curve(
    floats: Option<&[f32]>,
    scene_referred: bool,
) -> (ToneCurve, Option<CurveError>) {
    let fallback = if scene_referred {
        ToneCurve::Acr3
    } else {
        ToneCurve::Identity
    };
    match floats.map(profile_curve) {
        None => (fallback, None),
        Some(Ok(spline)) => (ToneCurve::Profile(spline), None),
        Some(Err(e)) => (fallback, Some(e)),
    }
}

/// The whole base curve for one working-space pixel (DEV-02, steps 1 to 4),
/// as the CPU reference: into linear ProPhoto, the ramp from `black` to 1
/// per channel, RGBTone through `table`, back to the working space.
/// `to_prophoto` is [`working_to_prophoto`] and `from_prophoto` its inverse.
pub fn base_curve(
    working: [f64; 3],
    to_prophoto: &Mat3,
    from_prophoto: &Mat3,
    ramp: &ExposureRamp,
    table: &CurveTable,
) -> [f64; 3] {
    let p = mul(to_prophoto, working).map(|c| ramp.evaluate(c.clamp(0.0, 1.0)) as f32);
    let toned = rgb_tone(p, table).map(f64::from);
    mul(from_prophoto, toned)
}

/// Middle gray, the point the tone controls work around (DEV-01, stage 6).
pub const MIDDLE_GRAY: f64 = 0.18;

/// The Whites end: the value the base curve maps to display white, in
/// `t = log2(Y / 0.18)`.
pub fn white_end() -> f64 {
    (1.0 / MIDDLE_GRAY).log2()
}

/// The Blacks end: the base curve's default black at 0 EV, in `t`.
pub fn black_end() -> f64 {
    (DEFAULT_SHADOWS * 0.001 / MIDDLE_GRAY).log2()
}

/// The furthest Whites and Blacks move their end, in stops.
pub const ENDS_RANGE: f64 = 1.5;

/// `r(u)`: `u²` up to 1, then `2u − 1`, so the change starts flat at middle
/// gray and continues smoothly past the source point.
fn ramp_up(u: f64) -> f64 {
    if u <= 1.0 { u * u } else { 2.0 * u - 1.0 }
}

/// Whites and Blacks (DEV-01, stage 6, step 3) on `t`, with `whites` and
/// `blacks` the sliders over 100 (−1 to 1). Whites acts above middle gray,
/// Blacks below; each moves its source point onto its end.
pub fn whites_blacks(t: f64, whites: f64, blacks: f64) -> f64 {
    if t > 0.0 && whites != 0.0 {
        let source = white_end() - ENDS_RANGE * whites;
        t + ENDS_RANGE * whites * ramp_up(t / source)
    } else if t < 0.0 && blacks != 0.0 {
        let source = black_end() - ENDS_RANGE * blacks;
        t + ENDS_RANGE * blacks * ramp_up(t / source)
    } else {
        t
    }
}

/// Contrast's gain at middle gray per unit of slider (DEV-01).
pub const CONTRAST_GAIN: f64 = 0.4;

/// Contrast (DEV-01, stage 6, step 4) on `t`, with `contrast` the slider over
/// 100: `t + 0.4 c × t × exp(−t² / 2σ²)`, σ 1 stop above middle gray and 1.6
/// below.
pub fn contrast(t: f64, contrast: f64) -> f64 {
    let sigma: f64 = if t > 0.0 { 1.0 } else { 1.6 };
    t + CONTRAST_GAIN * contrast * t * (-t * t / (2.0 * sigma * sigma)).exp()
}
