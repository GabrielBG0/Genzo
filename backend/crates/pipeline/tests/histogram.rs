//! Stage 9's histogram and clipping overlays on the GPU (DEV-03) against
//! the CPU reference (`pipeline::histogram`), on frames that clip.
//!
//! - Criterion 1: the counts equal a CPU count of the frame's read-back
//!   bytes, bin for bin, at Fit and in the half-size drag preview, and each
//!   frame's counts start from zero.
//! - Criterion 3: counting follows gamut compression: a Display P3 source
//!   in an sRGB frame clips less with it than without, as its codes say.
//! - Criterion 5: with J on, each pixel is `Overlay::apply` of the same
//!   frame without it, byte for byte; the histogram does not change.
//! - Criterion 6: the two Alt previews, the same way.
//! - Goldens of a clipping image with both overlays and with each preview,
//!   one test each.

use std::sync::{Mutex, MutexGuard};

use genzo_color::encoding::InputEncoding;
use genzo_color::output::Target;
use genzo_pipeline::camera_color::{camera_transform, encoded_transform};
use genzo_pipeline::gpu::raw_stages::{Pyramid, RawStages, TileConfig};
use genzo_pipeline::gpu::synthetic::{blocks, scene_raw};
use genzo_pipeline::gpu::view::{ViewStages, compresses, dithers};
use genzo_pipeline::gpu::{AdapterChoice, Gpu, GpuError};
use genzo_pipeline::histogram::{Histogram, Overlay, Preview, clipped, count};
use genzo_pipeline::look::{Look, Profile};
use genzo_pipeline::raw::RawSource;

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

/// The handoff's `label.red` and `label.blue`, as the frontend sends them.
const RED: [u8; 4] = [0xff, 0x6b, 0x6b, 255];
const BLUE: [u8; 4] = [0x5b, 0x9d, 0xff, 255];

/// The synthetic RAW scene at Fit, 1.5 EV over in Genzo Linear: its
/// highlights clip.
fn bright_scene(gpu: &Gpu, stages: &mut RawStages, view: &mut ViewStages, out: (u32, u32)) {
    let src = scene_raw(1536, 1024);
    let p = pyramid(gpu, stages, &src);
    view.sample(gpu, p, 1, out);
    let look = Look {
        exposure: 1.5,
        ..Look::with_profile(Profile::GenzoLinear)
    };
    view.tone(gpu, &look, None, None);
    view.color(gpu, &look);
}

/// An sRGB image of squares of `2 × scale` px, one per combination of the
/// codes 0, 96, 200 and 255 in R, G and B: highlights, shadows and both
/// clip. Sampled to `out`, so the frame is resampled; or, with None, to
/// level 1's own size, which the view copies (each square `scale` px).
/// Returns the size sampled to.
fn clipping_squares(
    gpu: &Gpu,
    stages: &mut RawStages,
    view: &mut ViewStages,
    scale: u32,
    out: Option<(u32, u32)>,
) -> (u32, u32) {
    let levels = [0u16, 96, 200, 255];
    let mut colors = vec![];
    for r in levels {
        for g in levels {
            for b in levels {
                colors.push([r, g, b].map(|v| v * 257));
            }
        }
    }
    let (size, rgba) = blocks(&colors, 8);
    // Each 2 x 2 block `scale` times as large.
    let big = (size.0 * scale, size.1 * scale);
    let mut up = vec![0u16; (big.0 * big.1 * 4) as usize];
    for y in 0..big.1 {
        for x in 0..big.0 {
            let from = (((y / scale) * size.0 + x / scale) * 4) as usize;
            let to = ((y * big.0 + x) * 4) as usize;
            up[to..to + 4].copy_from_slice(&rgba[from..from + 4]);
        }
    }
    let src = RawSource::from_encoded(big, &up, &InputEncoding::srgb()).unwrap();
    let p = pyramid(gpu, stages, &src);
    assert!(!compresses(p, Target::Srgb));
    let out = out.unwrap_or(p.size);
    view.sample(gpu, p, 1, out);
    out
}

/// Stage 9 into an sRGB frame with counting on, dithered, and `overlay`:
/// the frame and its histogram.
fn output(
    gpu: &Gpu,
    view: &mut ViewStages,
    compress: bool,
    overlay: Overlay,
    half: bool,
) -> (Vec<[u8; 4]>, Histogram) {
    view.set_counting(true);
    view.set_overlay(overlay);
    if half {
        view.output_half(gpu, Target::Srgb, compress, true);
    } else {
        view.output(gpu, Target::Srgb, compress, true);
    }
    (
        view.read_frame(gpu).unwrap(),
        view.read_histogram(gpu).unwrap(),
    )
}

#[test]
fn the_counts_are_the_frames() {
    let Some((_lock, gpu)) = gpu() else { return };
    let mut stages = RawStages::new(&gpu, TileConfig::default());
    let mut view = ViewStages::new(&gpu);
    bright_scene(&gpu, &mut stages, &mut view, (603, 402));
    for half in [false, true] {
        let (frame, h) = output(&gpu, &mut view, true, Overlay::default(), half);
        let (w, ht) = view.frame_size();
        assert_eq!(frame.len(), (w * ht) as usize);
        assert!(h.highlights.pixels > 0, "it clips");
        assert_eq!(h, count(&frame), "half size: {half}");
        // Again: the counts start from zero, not from the last frame's.
        let (_, again) = output(&gpu, &mut view, true, Overlay::default(), half);
        assert_eq!(again, h, "half size: {half}");
    }
    clipping_squares(&gpu, &mut stages, &mut view, 4, Some((480, 400)));
    for half in [false, true] {
        let (frame, h) = output(&gpu, &mut view, false, Overlay::default(), half);
        assert_eq!(h, count(&frame), "half size: {half}");
        assert!(h.highlights.pixels > 0 && h.shadows.pixels > 0);
        assert_eq!((h.highlights.channels, h.shadows.channels), (0b111, 0b111));
    }

    // Exact clip patches: an unedited sRGB image at its level 1 size is
    // its own codes, with no dither and no compression.
    let colors: Vec<[u16; 3]> = [
        [255, 128, 128],
        [255, 255, 255],
        [0, 128, 128],
        [0, 0, 255],
        [128, 128, 128],
        [12, 200, 77],
    ]
    .iter()
    .map(|c| c.map(|v: u16| v * 257))
    .collect();
    let (size, rgba) = blocks(&colors, 3);
    let src = RawSource::from_encoded(size, &rgba, &InputEncoding::srgb()).unwrap();
    let p = pyramid(&gpu, &mut stages, &src);
    assert!(!compresses(p, Target::Srgb) && !dithers(p, false));
    view.sample(&gpu, p, 1, p.size);
    view.set_counting(true);
    view.set_overlay(Overlay::default());
    view.output(&gpu, Target::Srgb, false, false);
    let frame = view.read_frame(&gpu).unwrap();
    let h = view.read_histogram(&gpu).unwrap();
    assert_eq!(h, count(&frame));
    // A level 1 texel is one 2 x 2 block: one pixel per color.
    assert_eq!((h.highlights.pixels, h.shadows.pixels), (3, 2));
    assert_eq!((h.highlights.channels, h.shadows.channels), (0b111, 0b011));
    assert_eq!(h.pixels, 6);
}

#[test]
fn clipping_is_counted_after_gamut_compression() {
    let Some((_lock, gpu)) = gpu() else { return };
    let mut stages = RawStages::new(&gpu, TileConfig::default());
    let mut view = ViewStages::new(&gpu);
    // Saturated Display P3 colors, most of them past sRGB.
    let mut colors = vec![];
    for r in [0u16, 64, 128, 200] {
        for g in [0u16, 96, 192, 255] {
            for b in [0u16, 80, 160, 240] {
                colors.push([r, g, b].map(|v| v * 257));
            }
        }
    }
    let (size, rgba) = blocks(&colors, 16);
    let src = RawSource::from_encoded(size, &rgba, &InputEncoding::display_p3()).unwrap();
    let p = pyramid(&gpu, &mut stages, &src);
    assert!(compresses(p, Target::Srgb), "P3 into sRGB compresses");
    view.sample(&gpu, p, 1, p.size);
    view.set_counting(true);
    let mut clipped = vec![];
    for compress in [true, false] {
        view.output(&gpu, Target::Srgb, compress, false);
        let frame = view.read_frame(&gpu).unwrap();
        let h = view.read_histogram(&gpu).unwrap();
        assert_eq!(h, count(&frame), "compress: {compress}");
        clipped.push(h.highlights.pixels + h.shadows.pixels);
    }
    println!(
        "clipped pixels with compression {}, without {}",
        clipped[0], clipped[1]
    );
    assert!(clipped[0] < clipped[1], "{clipped:?}");
}

#[test]
fn the_overlay_and_the_previews_draw_over_the_counted_frame() {
    let Some((_lock, gpu)) = gpu() else { return };
    let mut stages = RawStages::new(&gpu, TileConfig::default());
    let mut view = ViewStages::new(&gpu);
    clipping_squares(&gpu, &mut stages, &mut view, 4, Some((480, 400)));
    let (plain, h) = output(&gpu, &mut view, false, Overlay::default(), false);
    let both_sides = plain
        .iter()
        .filter(|&&px| {
            let (high, low) = clipped(px);
            high != 0 && low != 0
        })
        .count();
    println!(
        "{} highlight, {} shadow, {both_sides} both, of {}",
        h.highlights.pixels, h.shadows.pixels, h.pixels
    );
    assert!(both_sides > 0, "some pixels clip on both sides");
    let j = Overlay {
        highlights: Some(RED),
        shadows: Some(BLUE),
        preview: Preview::None,
    };
    for overlay in [
        j,
        Overlay {
            highlights: None,
            ..j
        },
        Overlay { shadows: None, ..j },
        Overlay {
            preview: Preview::Highlights,
            ..j
        },
        Overlay {
            preview: Preview::Shadows,
            ..Overlay::default()
        },
    ] {
        for half in [false, true] {
            let (base, _) = output(&gpu, &mut view, false, Overlay::default(), half);
            let (drawn, counted) = output(&gpu, &mut view, false, overlay, half);
            assert_eq!(counted, count(&base), "{overlay:?}: the photo is counted");
            let wrong = base
                .iter()
                .zip(&drawn)
                .filter(|(b, d)| overlay.apply(**b) != **d)
                .count();
            assert_eq!(wrong, 0, "{overlay:?}, half size {half}");
        }
    }
}

/// A stored golden: the clipping squares at level 1's size with
/// `overlay`, not dithered. The overlay is a threshold, so a frame that
/// differed by a code between adapters would flip pixels between the
/// photo and a color: here every code is exact (0 and 255 copied, and the
/// rest far from either). One test per case, so a missing golden does not
/// hide the next one's render.
fn golden(name: &str, overlay: Overlay) {
    let Some((_lock, gpu)) = gpu() else { return };
    let mut stages = RawStages::new(&gpu, TileConfig::default());
    let mut view = ViewStages::new(&gpu);
    let out = clipping_squares(&gpu, &mut stages, &mut view, 16, None);
    view.set_overlay(overlay);
    view.output(&gpu, Target::Srgb, false, false);
    let frame = view.read_frame(&gpu).unwrap();
    let pixels = frame
        .iter()
        .map(|f| [0, 1, 2].map(|c| f32::from(f[c]) / 255.0))
        .collect();
    let case = genzo_golden::Case::new(
        env!("CARGO_MANIFEST_DIR"),
        name,
        genzo_golden::Space::DisplaySrgb,
        genzo_golden::Producer::Gpu {
            adapter: gpu.info.name.clone(),
        },
    );
    genzo_golden::check(&case, &genzo_golden::Image::new(out.0, out.1, pixels));
}

#[test]
fn golden_clipping_overlay() {
    golden(
        "view/clipping-overlay",
        Overlay {
            highlights: Some(RED),
            shadows: Some(BLUE),
            preview: Preview::None,
        },
    );
}

#[test]
fn golden_clipping_preview_highlights() {
    golden(
        "view/clipping-preview-highlights",
        Overlay {
            preview: Preview::Highlights,
            ..Overlay::default()
        },
    );
}

#[test]
fn golden_clipping_preview_shadows() {
    golden(
        "view/clipping-preview-shadows",
        Overlay {
            preview: Preview::Shadows,
            ..Overlay::default()
        },
    );
}
