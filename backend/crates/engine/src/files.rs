//! Opening image files: format detection, decode and embedded previews.
//! Rust API only for now; the contract gains file commands in M1.

use std::path::Path;
use std::time::Instant;

use genzo_imgio::{ColorEncoding, DecodeOptions, Pixels};

use crate::EngineError;

/// What `genzo-cli inspect` prints about a file.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileReport {
    pub format: String,
    pub width: u32,
    pub height: u32,
    pub source_bits: u8,
    pub has_alpha: bool,
    /// What the pixels are: encoded RGB and its color encoding, or linear
    /// camera samples.
    pub color: String,
    /// EXIF or DNG orientation, 1 to 8, not yet applied to the pixels.
    pub orientation: u8,
    pub exif_bytes: Option<usize>,
    /// Size of the embedded preview, if the file has one.
    pub preview: Option<(u32, u32)>,
    /// Why the embedded preview could not be read, if it could not.
    pub preview_error: Option<String>,
    pub preview_ms: f64,
    pub decode_ms: f64,
}

fn read(path: &Path) -> Result<Vec<u8>, EngineError> {
    std::fs::read(path).map_err(|e| EngineError::Io(format!("{}: {e}", path.display())))
}

fn describe(pixels: &Pixels) -> String {
    match pixels {
        Pixels::Rgba16 { color, .. } => describe_encoding(color),
        Pixels::Linear {
            samples_per_pixel, ..
        } => format!("linear camera samples, {samples_per_pixel} per pixel"),
        Pixels::Cfa { pattern, .. } => {
            format!("CFA mosaic, {}x{} pattern", pattern.width, pattern.height)
        }
    }
}

fn describe_encoding(color: &ColorEncoding) -> String {
    match color {
        ColorEncoding::Srgb => "sRGB".into(),
        ColorEncoding::DisplayP3 => "Display P3".into(),
        ColorEncoding::Nclx {
            primaries,
            transfer,
            matrix,
            full_range,
        } => format!(
            "nclx primaries {primaries}, transfer {transfer}, matrix {matrix}, {}",
            if *full_range {
                "full range"
            } else {
                "limited range"
            }
        ),
        ColorEncoding::Icc(icc) => format!("ICC profile, {} bytes", icc.len()),
    }
}

/// Decodes a file's preview and full image, and reports what it found.
pub fn inspect(path: &Path) -> Result<FileReport, EngineError> {
    let data = read(path)?;
    let started = Instant::now();
    let preview = genzo_imgio::decode_preview(&data);
    let preview_ms = ms_since(started);
    let started = Instant::now();
    let img = genzo_imgio::decode(&data, DecodeOptions::default()).map_err(EngineError::decode)?;
    let decode_ms = ms_since(started);
    Ok(FileReport {
        format: format!("{:?}", img.format),
        width: img.width,
        height: img.height,
        source_bits: img.source_bits,
        has_alpha: img.has_alpha,
        color: describe(&img.pixels),
        orientation: img.orientation,
        exif_bytes: img.exif.as_ref().map(Vec::len),
        preview: preview
            .as_ref()
            .ok()
            .and_then(|p| p.as_ref().map(|p| (p.width, p.height))),
        preview_error: preview.err().map(|e| e.to_string()),
        preview_ms,
        decode_ms,
    })
}

/// Decodes a file `iterations` times with `threads` decode threads and
/// returns each run's time in milliseconds. The file is read once.
pub fn time_decode(path: &Path, threads: u8, iterations: u32) -> Result<Vec<f64>, EngineError> {
    let data = read(path)?;
    let options = DecodeOptions { threads };
    (0..iterations.max(1))
        .map(|_| {
            let started = Instant::now();
            let img = genzo_imgio::decode(&data, options).map_err(EngineError::decode)?;
            std::hint::black_box(img);
            Ok(ms_since(started))
        })
        .collect()
}

fn ms_since(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}
