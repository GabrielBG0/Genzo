//! An export's JPEG file (OUT-01, *Encoding and writing*).
//!
//! - **Streamed:** the encoder takes the image's rows as the render thread
//!   finishes them, one row of regions at a time ([`Strip`]), through
//!   jpeg-encoder's interleaved mode, which asks for rows in order. Only the
//!   strip it is reading is held, not the image.
//! - **Baseline JPEG** at the quality asked, chroma 4:2:0 below quality 90
//!   and 4:4:4 from 90; the EXIF segment (APP1) and the sRGB ICC profile
//!   (APP2) before the image.
//! - **Written to a temporary file** in the destination's folder, then
//!   renamed over the destination, so a cancel, a failure or a crash
//!   leaves no half-written file under its name. A failure or a cancel
//!   removes the temporary file; one a crash left behind has a name the
//!   next export of the same path removes first.

use std::cell::{Cell, RefCell};
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use jpeg_encoder::{Encoder, ImageBuffer, JpegColorType, SamplingFactor, rgb_to_ycbcr};

/// Quality from which chroma is kept whole (4:4:4).
pub const FULL_CHROMA_FROM: u8 = 90;

/// One run of the image's rows: RGBA8, row by row, `height` rows from `y`,
/// the image's full width.
#[derive(Debug)]
pub struct Strip {
    pub y: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// What goes into the file besides the pixels.
#[derive(Debug, Clone)]
pub struct Settings {
    /// 1 to 100.
    pub quality: u8,
    /// The EXIF segment's payload (`Exif\0\0` first), if any.
    pub exif: Option<Vec<u8>>,
    /// The ICC profile to embed.
    pub icc: Vec<u8>,
}

#[derive(Debug, thiserror::Error)]
pub enum JpegError {
    /// The rows stopped before the last: cancelled, or the render failed
    /// (its reason comes from the caller).
    #[error("the export stopped before its last row")]
    Stopped,
    #[error("too large for JPEG: {0} x {1} pixels, 65,535 at most on a side")]
    TooLarge(u32, u32),
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("JPEG encoding failed: {0}")]
    Encoding(String),
}

/// The temporary file an export of `destination` writes first: in the same
/// folder, so the rename stays on one volume, and hidden by its dot.
pub fn temporary(destination: &Path) -> PathBuf {
    let name = destination
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    destination.with_file_name(format!(".{name}.genzo-export"))
}

/// Writes the `size` image from `next` (which hands its strips in order,
/// `None` to stop early) to `destination` as a JPEG, through a temporary
/// file renamed at the end. `progress` gets the rows done after each strip.
pub fn write_file(
    destination: &Path,
    size: (u32, u32),
    settings: &Settings,
    next: impl FnMut() -> Option<Strip>,
    progress: impl FnMut(u32),
) -> Result<(), JpegError> {
    let tmp = temporary(destination);
    // One a crash left behind.
    let _ = std::fs::remove_file(&tmp);
    let result = (|| {
        let file = File::create(&tmp)?;
        let mut out = BufWriter::new(file);
        encode(&mut out, size, settings, next, progress)?;
        let file = out.into_inner().map_err(|e| e.into_error())?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, destination)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// Encodes the `size` image from `next`'s strips into `out`.
pub fn encode<W: Write>(
    out: W,
    size: (u32, u32),
    settings: &Settings,
    next: impl FnMut() -> Option<Strip>,
    progress: impl FnMut(u32),
) -> Result<(), JpegError> {
    let (Ok(w), Ok(h)) = (u16::try_from(size.0), u16::try_from(size.1)) else {
        return Err(JpegError::TooLarge(size.0, size.1));
    };
    let mut encoder = Encoder::new(out, settings.quality.clamp(1, 100));
    encoder.set_sampling_factor(if settings.quality >= FULL_CHROMA_FROM {
        SamplingFactor::F_1_1
    } else {
        SamplingFactor::F_2_2
    });
    if let Some(exif) = &settings.exif {
        encoder
            .add_app_segment(1, exif.clone())
            .map_err(|e| JpegError::Encoding(e.to_string()))?;
    }
    encoder
        .add_icc_profile(&settings.icc)
        .map_err(|e| JpegError::Encoding(e.to_string()))?;
    let image = Rows {
        width: w,
        height: h,
        next: RefCell::new(next),
        progress: RefCell::new(progress),
        current: RefCell::new(None),
        stopped: Rc::default(),
        reported: Cell::new(0),
    };
    let stopped = Rc::clone(&image.stopped);
    let encoded = encoder.encode_image(image);
    if stopped.get() {
        return Err(JpegError::Stopped);
    }
    encoded.map_err(|e| JpegError::Encoding(e.to_string()))
}

/// The image as jpeg-encoder reads it: row by row, from the strips.
struct Rows<N, P> {
    width: u16,
    height: u16,
    next: RefCell<N>,
    progress: RefCell<P>,
    current: RefCell<Option<Strip>>,
    /// Shared with the caller, which the encoder takes the image from.
    stopped: Rc<Cell<bool>>,
    reported: Cell<u32>,
}

impl<N: FnMut() -> Option<Strip>, P: FnMut(u32)> Rows<N, P> {
    /// Reports `rows` done, once each: the encoder asks for the last row
    /// again to pad its last block row.
    fn report(&self, rows: u32) {
        if rows > self.reported.get() {
            self.reported.set(rows);
            (self.progress.borrow_mut())(rows);
        }
    }

    /// Makes the strip holding row `y` current; false when the strips ran
    /// out first.
    fn reach(&self, y: u32) -> bool {
        let mut current = self.current.borrow_mut();
        loop {
            match current.as_ref() {
                Some(s) if y >= s.y && y < s.y + s.height => return true,
                Some(s) if y < s.y => return false,
                _ => {}
            }
            if let Some(done) = current.as_ref() {
                self.report(done.y + done.height);
            }
            match (self.next.borrow_mut())() {
                Some(s) if s.rgba.len() == s.height as usize * self.width as usize * 4 => {
                    *current = Some(s);
                }
                _ => {
                    *current = None;
                    return false;
                }
            }
        }
    }
}

impl<N: FnMut() -> Option<Strip>, P: FnMut(u32)> ImageBuffer for Rows<N, P> {
    fn get_jpeg_color_type(&self) -> JpegColorType {
        JpegColorType::Ycbcr
    }

    fn width(&self) -> u16 {
        self.width
    }

    fn height(&self) -> u16 {
        self.height
    }

    fn fill_buffers(&self, y: u16, buffers: &mut [Vec<u8>; 4]) {
        let y = u32::from(y);
        if self.stopped.get() || !self.reach(y) {
            // The encoder cannot be stopped from here: it finishes on gray
            // rows, and the file is thrown away.
            self.stopped.set(true);
            for _ in 0..self.width {
                buffers[0].push(128);
                buffers[1].push(128);
                buffers[2].push(128);
            }
            return;
        }
        let current = self.current.borrow();
        let s = current.as_ref().expect("reached");
        let row = (y - s.y) as usize * self.width as usize * 4;
        for px in s.rgba[row..row + self.width as usize * 4].as_chunks::<4>().0 {
            let (luma, cb, cr) = rgb_to_ycbcr(px[0], px[1], px[2]);
            buffers[0].push(luma);
            buffers[1].push(cb);
            buffers[2].push(cr);
        }
        if y + 1 == u32::from(self.height) {
            self.report(y + 1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gradient(size: (u32, u32)) -> Vec<u8> {
        (0..size.0 * size.1)
            .flat_map(|i| {
                let (x, y) = (i % size.0, i / size.0);
                [(x * 255 / size.0) as u8, (y * 255 / size.1) as u8, 90, 255]
            })
            .collect()
    }

    fn strips(rgba: &[u8], size: (u32, u32), rows: u32) -> Vec<Strip> {
        (0..size.1)
            .step_by(rows as usize)
            .map(|y| {
                let height = rows.min(size.1 - y);
                let at = (y * size.0 * 4) as usize;
                Strip {
                    y,
                    height,
                    rgba: rgba[at..at + (height * size.0 * 4) as usize].to_vec(),
                }
            })
            .collect()
    }

    fn settings(quality: u8) -> Settings {
        Settings {
            quality,
            exif: Some(b"Exif\0\0II*\0\x08\0\0\0\0\0\0\0\0\0".to_vec()),
            icc: genzo_color::output::srgb_icc().to_vec(),
        }
    }

    /// Streamed in strips of any height, the file is the bytes of the
    /// image encoded whole, at both chroma settings.
    #[test]
    fn strips_encode_as_the_whole_image() {
        let size = (300, 211);
        let rgba = gradient(size);
        for quality in [85, 95] {
            let mut whole = Vec::new();
            let mut one = strips(&rgba, size, size.1).into_iter();
            encode(&mut whole, size, &settings(quality), || one.next(), |_| {}).unwrap();
            for rows in [1, 7, 16, 100] {
                let mut parts = strips(&rgba, size, rows).into_iter();
                let mut done = Vec::new();
                let mut out = Vec::new();
                encode(
                    &mut out,
                    size,
                    &settings(quality),
                    || parts.next(),
                    |r| done.push(r),
                )
                .unwrap();
                assert_eq!(out, whole, "quality {quality}, {rows} rows a strip");
                assert_eq!(done.last(), Some(&size.1));
                assert!(done.windows(2).all(|w| w[0] < w[1]), "{done:?}");
            }
            // It decodes to the image, and carries the segments.
            let img = genzo_imgio::decode(&whole, Default::default()).unwrap();
            assert_eq!((img.width, img.height), size);
            assert!(whole.windows(6).any(|w| w == b"Exif\0\0"));
            assert!(whole.windows(12).any(|w| w == b"ICC_PROFILE\0"));
        }
    }

    #[test]
    fn rows_that_stop_early_fail_and_leave_no_file() {
        let dir = std::env::temp_dir().join(format!("genzo-jpeg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("out.jpg");
        let size = (64, 64);
        let rgba = gradient(size);
        let mut parts = strips(&rgba, size, 16).into_iter().take(2);
        let err = write_file(&path, size, &settings(85), || parts.next(), |_| {}).unwrap_err();
        assert!(matches!(err, JpegError::Stopped), "{err}");
        assert!(!path.exists() && !temporary(&path).exists());
        // An existing destination stays as it was.
        std::fs::write(&path, b"before").unwrap();
        let mut none = std::iter::empty();
        assert!(write_file(&path, size, &settings(85), || none.next(), |_| {}).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"before");
        // A whole run replaces it.
        let mut all = strips(&rgba, size, 16).into_iter();
        write_file(&path, size, &settings(85), || all.next(), |_| {}).unwrap();
        assert!(std::fs::read(&path).unwrap().starts_with(&[0xFF, 0xD8]));
        assert!(!temporary(&path).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn too_large_for_jpeg_is_refused() {
        let mut none = std::iter::empty();
        let err = encode(
            Vec::new(),
            (70_000, 10),
            &settings(85),
            || none.next(),
            |_| {},
        );
        assert!(matches!(err, Err(JpegError::TooLarge(70_000, 10))));
    }
}
