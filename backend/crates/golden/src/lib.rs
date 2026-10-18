//! Golden-image tests (DEV-01, acceptance criterion 5).
//!
//! A test renders a small image and calls [`check`], which compares it with the
//! stored golden in the calling crate's `tests/goldens/`. How the comparison
//! works depends on what the image holds ([`Space`]):
//!
//! - **Linear** data (camera RGB, scene-linear AP1): PSNR and the largest
//!   absolute error over the full float range, negatives and values above 1
//!   included, with peak 1.0 (the normalized white level). Plus ΔE2000
//!   through a fixed [`View`], which only has to be fixed, not accurate.
//! - **Display** data (encoded sRGB, 0 to 1): ΔE2000 and PSNR directly.
//!
//! Goldens are 16-bit RGB PNGs holding each channel's f16 bit pattern. That is
//! lossless, and since f16 bits rise with the value for positive numbers, a
//! golden reads as a log-encoded image in a viewer. Negative values carry the
//! sign bit and show as the brightest pixels.
//!
//! A missing or failing golden fails the test and writes the actual image, a
//! ΔE heatmap and a metadata file to `backend/target/golden-actual/<crate>/`
//! (`GENZO_GOLDEN_OUT` overrides the folder). CI uploads that folder, and
//! `scripts/accept-goldens.py` copies accepted renders into place. GPU goldens
//! are accepted only from lavapipe, the reference device. `GENZO_GOLDEN_UPDATE=1`
//! writes goldens in place instead, for CPU goldens or on lavapipe.

use std::fmt;
use std::fs;
use std::io::BufWriter;
use std::path::{Path, PathBuf};

use half::f16;

/// An RGB image of 32-bit floats, row by row.
#[derive(Debug, Clone, PartialEq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgb: Vec<[f32; 3]>,
}

impl Image {
    pub fn new(width: u32, height: u32, rgb: Vec<[f32; 3]>) -> Self {
        assert_eq!(rgb.len(), (width * height) as usize, "pixel count");
        Self { width, height, rgb }
    }

    /// An 8-bit RGBA frame as encoded values from 0 to 1. Alpha is dropped.
    pub fn from_rgba8(width: u32, height: u32, rgba: &[u8]) -> Self {
        let rgb = rgba
            .as_chunks::<4>()
            .0
            .iter()
            .map(|p| [p[0], p[1], p[2]].map(|c| c as f32 / 255.0))
            .collect();
        Self::new(width, height, rgb)
    }

    /// Averages `factor` × `factor` blocks, dropping a partial last row or
    /// column of blocks. Keeps goldens small.
    pub fn box_downscale(&self, factor: u32) -> Self {
        let (w, h) = (self.width / factor, self.height / factor);
        let n = (factor * factor) as f32;
        let mut rgb = Vec::with_capacity((w * h) as usize);
        for by in 0..h {
            for bx in 0..w {
                let mut sum = [0.0f32; 3];
                for y in by * factor..(by + 1) * factor {
                    for x in bx * factor..(bx + 1) * factor {
                        let p = self.rgb[(y * self.width + x) as usize];
                        for c in 0..3 {
                            sum[c] += p[c];
                        }
                    }
                }
                rgb.push(sum.map(|s| s / n));
            }
        }
        Self::new(w, h, rgb)
    }

    /// Each value rounded to f16, as a golden stores it.
    fn quantized(&self) -> Self {
        let rgb = self
            .rgb
            .iter()
            .map(|p| p.map(|c| f16::from_f32(c).to_f32()))
            .collect();
        Self::new(self.width, self.height, rgb)
    }
}

/// How ΔE2000 sees linear data: multiply by `matrix` into linear sRGB, apply
/// `exposure_ev`, clip to 0 to 1, encode as sRGB.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct View {
    pub matrix: [[f64; 3]; 3],
    pub exposure_ev: f64,
}

impl View {
    /// Reads the data as linear sRGB as it is. For camera RGB this is not
    /// accurate, and does not need to be: the view only has to stay fixed.
    pub const IDENTITY: View = View {
        matrix: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        exposure_ev: 0.0,
    };

    pub fn with_exposure(self, exposure_ev: f64) -> Self {
        Self {
            exposure_ev,
            ..self
        }
    }

    fn encode(&self, p: [f32; 3]) -> [f64; 3] {
        let gain = 2f64.powf(self.exposure_ev);
        let m = &self.matrix;
        let v = [0, 1, 2].map(|r| {
            (m[r][0] * p[0] as f64 + m[r][1] * p[1] as f64 + m[r][2] * p[2] as f64) * gain
        });
        v.map(|c| genzo_color::srgb_encode(c.clamp(0.0, 1.0)))
    }
}

/// What the image holds, which decides how it is compared.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Space {
    /// Camera RGB or scene-linear working space, normalized so white is 1.0.
    Linear(View),
    /// Encoded sRGB, 0 to 1.
    DisplaySrgb,
}

/// Who made the image. Only CPU goldens and lavapipe renders may become
/// goldens; a hardware GPU is compared against them.
#[derive(Debug, Clone, PartialEq)]
pub enum Producer {
    Cpu,
    Gpu { adapter: String },
}

impl Producer {
    fn may_write_golden(&self) -> bool {
        match self {
            Producer::Cpu => true,
            Producer::Gpu { adapter } => is_lavapipe(adapter),
        }
    }
}

/// lavapipe reports itself as an llvmpipe device.
pub fn is_lavapipe(adapter: &str) -> bool {
    adapter.to_ascii_lowercase().contains("llvmpipe")
}

/// The limits a comparison must stay within. The defaults are DEV-01's.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tolerance {
    pub mean_de: f64,
    pub max_de: f64,
    pub min_psnr_db: f64,
    /// Largest absolute difference per channel, if the case sets one.
    pub max_abs: Option<f64>,
}

impl Default for Tolerance {
    fn default() -> Self {
        Self {
            mean_de: 0.5,
            max_de: 2.0,
            min_psnr_db: 45.0,
            max_abs: None,
        }
    }
}

/// The result of comparing two images of the same size.
#[derive(Clone, PartialEq)]
pub struct Report {
    pub psnr_db: f64,
    pub max_abs: f64,
    pub mean_de: f64,
    pub p99_de: f64,
    pub max_de: f64,
    /// Share of pixels with ΔE2000 above 1, from 0 to 1.
    pub share_de_above_1: f64,
    de: Vec<f32>,
}

impl fmt::Debug for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "PSNR {:.2} dB, max abs {:.5}, ΔE2000 mean {:.3} / p99 {:.3} / max {:.3}, {:.3} % above 1",
            self.psnr_db,
            self.max_abs,
            self.mean_de,
            self.p99_de,
            self.max_de,
            self.share_de_above_1 * 100.0
        )
    }
}

impl Report {
    /// The limits this report breaks, empty if it passes.
    pub fn failures(&self, tol: &Tolerance) -> Vec<String> {
        let mut out = Vec::new();
        if self.mean_de > tol.mean_de {
            out.push(format!("mean ΔE2000 {:.3} > {}", self.mean_de, tol.mean_de));
        }
        if self.max_de > tol.max_de {
            out.push(format!("max ΔE2000 {:.3} > {}", self.max_de, tol.max_de));
        }
        if self.psnr_db < tol.min_psnr_db {
            out.push(format!("PSNR {:.2} dB < {}", self.psnr_db, tol.min_psnr_db));
        }
        if let Some(max) = tol.max_abs
            && self.max_abs > max
        {
            out.push(format!("max abs {:.5} > {max}", self.max_abs));
        }
        out
    }
}

/// Compares `actual` with `golden`, which must have the same size.
pub fn compare(actual: &Image, golden: &Image, space: Space) -> Report {
    assert_eq!(
        (actual.width, actual.height),
        (golden.width, golden.height),
        "image sizes differ"
    );
    let encode = |p: [f32; 3]| match space {
        Space::Linear(view) => view.encode(p),
        Space::DisplaySrgb => p.map(|c| (c as f64).clamp(0.0, 1.0)),
    };
    let (mut se, mut max_abs) = (0.0f64, 0.0f64);
    let mut de = Vec::with_capacity(actual.rgb.len());
    for (a, g) in actual.rgb.iter().zip(&golden.rgb) {
        for c in 0..3 {
            let d = (a[c] as f64 - g[c] as f64).abs();
            se += d * d;
            max_abs = max_abs.max(d);
        }
        de.push(genzo_color::delta_e_2000_srgb(encode(*a), encode(*g)) as f32);
    }
    let mse = se / (actual.rgb.len() * 3) as f64;
    let mut sorted = de.clone();
    sorted.sort_by(f32::total_cmp);
    let n = sorted.len().max(1);
    Report {
        psnr_db: if mse == 0.0 {
            f64::INFINITY
        } else {
            10.0 * (1.0 / mse).log10()
        },
        max_abs,
        mean_de: sorted.iter().map(|&d| d as f64).sum::<f64>() / n as f64,
        p99_de: sorted.get(n * 99 / 100).copied().unwrap_or(0.0) as f64,
        max_de: sorted.last().copied().unwrap_or(0.0) as f64,
        share_de_above_1: sorted.iter().filter(|&&d| d > 1.0).count() as f64 / n as f64,
        de,
    }
}

/// One golden-image test.
#[derive(Debug, Clone)]
pub struct Case {
    /// The calling crate's directory: pass `env!("CARGO_MANIFEST_DIR")`.
    pub crate_dir: PathBuf,
    /// The golden's name under `tests/goldens/`, without extension. May hold
    /// `/` for subfolders, such as `decode/iphone-12-pro`.
    pub name: String,
    pub space: Space,
    pub tolerance: Tolerance,
    pub producer: Producer,
}

impl Case {
    pub fn new(crate_dir: &str, name: &str, space: Space, producer: Producer) -> Self {
        Self {
            crate_dir: PathBuf::from(crate_dir),
            name: name.to_owned(),
            space,
            tolerance: Tolerance::default(),
            producer,
        }
    }

    pub fn with_tolerance(self, tolerance: Tolerance) -> Self {
        Self { tolerance, ..self }
    }

    fn golden_path(&self) -> PathBuf {
        self.crate_dir
            .join("tests/goldens")
            .join(format!("{}.f16.png", self.name))
    }

    fn crate_name(&self) -> String {
        self.crate_dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    /// Where a failing run writes its actual image, heatmap and metadata.
    fn out_stem(&self) -> PathBuf {
        let root = std::env::var_os("GENZO_GOLDEN_OUT")
            .map(PathBuf::from)
            .unwrap_or_else(|| self.crate_dir.join("../../target/golden-actual"));
        root.join(self.crate_name()).join(&self.name)
    }
}

/// Compares `actual` with the case's golden and panics, with the report and
/// the paths of the written outputs, when it is missing or out of tolerance.
/// Returns the report when it passes.
pub fn check(case: &Case, actual: &Image) -> Report {
    let actual = actual.quantized();
    let golden_path = case.golden_path();

    if std::env::var("GENZO_GOLDEN_UPDATE").as_deref() == Ok("1") {
        assert!(
            case.producer.may_write_golden(),
            "GENZO_GOLDEN_UPDATE: {:?} may not write goldens; GPU goldens come from lavapipe",
            case.producer
        );
        write_f16_png(&golden_path, &actual).expect("write golden");
        eprintln!("golden written: {}", golden_path.display());
        return compare(&actual, &actual, case.space);
    }

    let golden = match read_f16_png(&golden_path) {
        Ok(g) => g,
        Err(e) => {
            let out = write_outputs(case, &actual, None, "no golden");
            panic!(
                "golden {}: {e}. The render was written to {}; accept it with scripts/accept-goldens.py",
                golden_path.display(),
                out.display()
            );
        }
    };
    if (golden.width, golden.height) != (actual.width, actual.height) {
        let out = write_outputs(case, &actual, None, "size changed");
        panic!(
            "golden {} is {}x{}, the render is {}x{}; render written to {}",
            golden_path.display(),
            golden.width,
            golden.height,
            actual.width,
            actual.height,
            out.display()
        );
    }
    let report = compare(&actual, &golden, case.space);
    let failures = report.failures(&case.tolerance);
    if !failures.is_empty() {
        let out = write_outputs(case, &actual, Some(&report), &failures.join("; "));
        panic!(
            "golden {} failed: {}\n{report:?}\nrender and ΔE heatmap written to {}",
            golden_path.display(),
            failures.join("; "),
            out.display()
        );
    }
    eprintln!("golden {} ok: {report:?}", case.name);
    report
}

/// Writes the actual image, the ΔE heatmap and the metadata of a failed check,
/// and returns the actual image's path.
fn write_outputs(case: &Case, actual: &Image, report: Option<&Report>, why: &str) -> PathBuf {
    let stem = case.out_stem();
    let image = stem.with_extension("f16.png");
    write_f16_png(&image, actual).expect("write actual render");
    if let Some(report) = report {
        write_heatmap(&stem.with_extension("heatmap.png"), actual, report).expect("write heatmap");
    }
    let (producer, adapter) = match &case.producer {
        Producer::Cpu => ("cpu", String::new()),
        Producer::Gpu { adapter } => ("gpu", adapter.clone()),
    };
    let meta = format!(
        "{{\n  \"case\": {:?},\n  \"crate\": {:?},\n  \"producer\": {producer:?},\n  \"adapter\": {adapter:?},\n  \"reason\": {why:?},\n  \"report\": {:?}\n}}\n",
        case.name,
        case.crate_name(),
        report.map(|r| format!("{r:?}")).unwrap_or_default()
    );
    fs::write(stem.with_extension("meta.json"), meta).expect("write metadata");
    image
}

/// ΔE2000 per pixel as 8-bit gray: black is 0, white is 4 or more.
fn write_heatmap(path: &Path, img: &Image, report: &Report) -> std::io::Result<()> {
    let gray: Vec<u8> = report
        .de
        .iter()
        .map(|&d| ((d / 4.0).clamp(0.0, 1.0) * 255.0).round() as u8)
        .collect();
    encode_png(
        path,
        img.width,
        img.height,
        png::ColorType::Grayscale,
        png::BitDepth::Eight,
        &gray,
    )
}

/// Writes an image as a 16-bit RGB PNG of f16 bit patterns.
pub fn write_f16_png(path: &Path, img: &Image) -> std::io::Result<()> {
    let mut bytes = Vec::with_capacity(img.rgb.len() * 6);
    for p in &img.rgb {
        for c in p {
            bytes.extend_from_slice(&f16::from_f32(*c).to_bits().to_be_bytes());
        }
    }
    encode_png(
        path,
        img.width,
        img.height,
        png::ColorType::Rgb,
        png::BitDepth::Sixteen,
        &bytes,
    )
}

fn encode_png(
    path: &Path,
    width: u32,
    height: u32,
    color: png::ColorType,
    depth: png::BitDepth,
    data: &[u8],
) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut encoder = png::Encoder::new(BufWriter::new(fs::File::create(path)?), width, height);
    encoder.set_color(color);
    encoder.set_depth(depth);
    encoder.set_compression(png::Compression::Best);
    let mut writer = encoder.write_header().map_err(std::io::Error::other)?;
    writer.write_image_data(data).map_err(std::io::Error::other)
}

/// Reads a golden written by [`write_f16_png`].
pub fn read_f16_png(path: &Path) -> std::io::Result<Image> {
    let decoder = png::Decoder::new(std::io::BufReader::new(fs::File::open(path)?));
    let mut reader = decoder.read_info().map_err(std::io::Error::other)?;
    let info = reader.info();
    if info.color_type != png::ColorType::Rgb || info.bit_depth != png::BitDepth::Sixteen {
        return Err(std::io::Error::other("not a 16-bit RGB golden"));
    }
    let (width, height) = (info.width, info.height);
    let mut buf = vec![0; reader.output_buffer_size()];
    reader.next_frame(&mut buf).map_err(std::io::Error::other)?;
    let rgb = buf
        .as_chunks::<6>()
        .0
        .iter()
        .map(|p| {
            [0, 1, 2].map(|c| f16::from_bits(u16::from_be_bytes([p[2 * c], p[2 * c + 1]])).to_f32())
        })
        .collect();
    Ok(Image::new(width, height, rgb))
}

/// A corpus input by its `corpus/manifest.json` key, if it is on disk.
///
/// When it is missing the test should skip, and this says so, unless
/// `GENZO_REQUIRE_CORPUS=1` (set in the one CI job that fetches the corpus),
/// where it panics with the command that fetches it.
pub fn corpus(key: &str) -> Option<PathBuf> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../corpus")
        .join(key);
    if path.exists() {
        return Some(path);
    }
    let fetch = format!("python scripts/fetch-raw-corpus.py --only \"{key}\"");
    if std::env::var("GENZO_REQUIRE_CORPUS").as_deref() == Ok("1") {
        panic!("corpus file {key} is missing; fetch it with: {fetch}");
    }
    eprintln!("skipped: corpus file {key} is missing ({fetch})");
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ramp(w: u32, h: u32, scale: f32) -> Image {
        let rgb = (0..w * h)
            .map(|i| {
                let t = i as f32 / (w * h) as f32;
                [t * scale, 0.5 * t * scale, -0.01 + t]
            })
            .collect();
        Image::new(w, h, rgb)
    }

    #[test]
    fn f16_png_round_trips_negatives_and_highlights() {
        let img = ramp(17, 9, 4.0).quantized();
        let dir = std::env::temp_dir().join("genzo-golden-test");
        let path = dir.join("ramp.f16.png");
        write_f16_png(&path, &img).unwrap();
        assert_eq!(read_f16_png(&path).unwrap(), img);
    }

    #[test]
    fn identical_images_compare_perfectly() {
        let img = ramp(8, 8, 2.0);
        let r = compare(&img, &img, Space::Linear(View::IDENTITY));
        assert_eq!(r.psnr_db, f64::INFINITY);
        assert_eq!((r.max_abs, r.max_de), (0.0, 0.0));
        assert!(r.failures(&Tolerance::default()).is_empty());
    }

    #[test]
    fn psnr_and_delta_e_see_a_shift() {
        let a = Image::new(1, 1, vec![[0.5, 0.5, 0.5]]);
        let b = Image::new(1, 1, vec![[0.51, 0.5, 0.5]]);
        let r = compare(&a, &b, Space::DisplaySrgb);
        // MSE = 0.01² / 3, so PSNR = 10 log10(30000) ≈ 44.77 dB.
        assert!((r.psnr_db - 44.77).abs() < 0.01, "{r:?}");
        assert!(r.max_de > 0.5 && r.max_de < 2.0, "{r:?}");
        assert_eq!(r.failures(&Tolerance::default()).len(), 2, "{r:?}");
    }

    #[test]
    fn view_clips_and_exposes() {
        let v = View::IDENTITY.with_exposure(1.0);
        let got = v.encode([0.5, 2.0, -1.0]);
        for (g, want) in got.iter().zip([1.0, 1.0, 0.0]) {
            assert!((g - want).abs() < 1e-12, "{got:?}");
        }
    }

    #[test]
    fn only_cpu_and_lavapipe_write_goldens() {
        assert!(Producer::Cpu.may_write_golden());
        let lavapipe = "llvmpipe (LLVM 15.0.7, 256 bits)";
        assert!(
            Producer::Gpu {
                adapter: lavapipe.into()
            }
            .may_write_golden()
        );
        let rtx = "NVIDIA GeForce RTX 3060";
        assert!(
            !Producer::Gpu {
                adapter: rtx.into()
            }
            .may_write_golden()
        );
    }

    #[test]
    fn downscale_averages_blocks() {
        let img = Image::new(
            2,
            2,
            vec![
                [0.0, 0.0, 0.0],
                [1.0, 1.0, 1.0],
                [1.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
            ],
        );
        assert_eq!(img.box_downscale(2).rgb, vec![[0.5, 0.5, 0.25]]);
    }
}
