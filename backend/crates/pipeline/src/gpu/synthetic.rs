//! A synthetic Bayer scene with a known ground truth. It stands in for a raw
//! file in the benchmarks, and lets the tests measure the demosaic against
//! the true image instead of against a stored render.

use super::spike::BayerImage;

/// Black and white levels of the synthetic sensor (14-bit).
pub const BLACK: u16 = 512;
pub const WHITE: u16 = 16383;

/// The scene in linear sRGB at pixel (x, y). Smooth gradients, flat color
/// patches with hard edges, and a band of fine diagonal lines.
pub fn scene(x: u32, y: u32, width: u32, height: u32) -> [f32; 3] {
    let u = x as f32 / width as f32;
    let v = y as f32 / height as f32;
    if v < 0.4 {
        // Smooth gradients.
        [0.05 + 0.7 * u, 0.05 + 0.6 * v / 0.4, 0.05 + 0.5 * (1.0 - u)]
    } else if v < 0.8 {
        // A 6 × 2 grid of flat patches.
        const PATCHES: [[f32; 3]; 12] = [
            [0.40, 0.10, 0.08],
            [0.08, 0.35, 0.10],
            [0.07, 0.10, 0.40],
            [0.45, 0.40, 0.06],
            [0.06, 0.35, 0.40],
            [0.40, 0.08, 0.35],
            [0.18, 0.18, 0.18],
            [0.60, 0.60, 0.60],
            [0.03, 0.03, 0.03],
            [0.30, 0.20, 0.12],
            [0.12, 0.22, 0.30],
            [0.25, 0.30, 0.10],
        ];
        let col = ((u * 6.0) as usize).min(5);
        let row = if v < 0.6 { 0 } else { 1 };
        PATCHES[row * 6 + col]
    } else {
        // Fine diagonal lines: a period of 8 pixels, dark on light.
        let on = (x + y) % 8 < 3;
        if on {
            [0.05, 0.05, 0.05]
        } else {
            [0.5, 0.45, 0.4]
        }
    }
}

/// Mosaics `rgb` (linear, in the camera space) onto an RGGB sensor.
pub fn mosaic(width: u32, height: u32, rgb: impl Fn(u32, u32) -> [f32; 3]) -> BayerImage {
    let cfa = [0, 1, 1, 2];
    let range = (WHITE - BLACK) as f32;
    let mut data = Vec::with_capacity(width as usize * height as usize);
    for y in 0..height {
        for x in 0..width {
            let c = cfa[((y & 1) * 2 + (x & 1)) as usize];
            let v = rgb(x, y)[c as usize].clamp(0.0, 1.0);
            data.push(BLACK + (v * range).round() as u16);
        }
    }
    BayerImage {
        width,
        height,
        data,
        cfa,
        black: BLACK as f32,
        white: WHITE as f32,
        // The synthetic camera records the working space directly.
        cam_to_working: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
    }
}

/// [`scene`] converted to the working space and mosaiced, so the pipeline's
/// sRGB output should reproduce `scene`.
pub fn scene_mosaic(width: u32, height: u32) -> BayerImage {
    let to_working = genzo_color::mat_mul(
        &genzo_color::invert(&genzo_color::WORKING_TO_XYZ).expect("invertible"),
        &genzo_color::SRGB_TO_XYZ,
    );
    mosaic(width, height, |x, y| {
        let s = scene(x, y, width, height).map(f64::from);
        genzo_color::mul(&to_working, s).map(|c| c as f32)
    })
}
