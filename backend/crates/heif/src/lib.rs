//! HEIC and HEIF decoding through libheif with its libde265 HEVC decoder,
//! both built from pinned sources and linked statically (see build.rs).
//!
//! This crate is the only place in the backend that runs C code for images,
//! and the only one allowed `unsafe`. It is optional: `genzo-imgio` uses it
//! behind its `heic` feature, so HEIC support can be cut by turning that off.
//!
//! Decoding follows the spec for v1: the primary image only, its SDR base.
//! HDR gain maps, depth maps and Live Photo data are ignored.

use std::ffi::{CStr, c_char, c_int};
use std::ptr;
use std::sync::Once;

#[allow(unsafe_code)]
mod ffi;

/// The libheif version this crate is built from. A test checks that the
/// linked library reports the same, so a system copy can never slip in.
pub const LIBHEIF_VERSION: &str = "1.23.6";

/// Where a file's colors are defined.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ColorProfile {
    /// No profile in the file. HEIF says to assume sRGB.
    None,
    /// ITU-T H.273 code points, as used by iPhone files (Display P3 is
    /// primaries 12, transfer 13).
    Nclx {
        primaries: u16,
        transfer: u16,
        matrix: u16,
        full_range: bool,
    },
    /// An embedded ICC profile.
    Icc(Vec<u8>),
}

/// A decoded image: interleaved RGBA, 16 bits per channel, scaled to the full
/// 0..=65535 range whatever the source depth. Rotation, mirroring and crop
/// from the container are already applied, so the EXIF orientation tag must
/// not be applied again.
#[derive(Debug, Clone)]
pub struct Decoded {
    pub width: u32,
    pub height: u32,
    /// Bits per channel in the file: 8, 10 or 12.
    pub source_bits: u8,
    pub has_alpha: bool,
    pub color: ColorProfile,
    /// The TIFF-structured EXIF data, without HEIF's 4-byte offset prefix.
    pub exif: Option<Vec<u8>>,
    pub rgba: Vec<u16>,
}

/// An embedded thumbnail, 8 bits per channel RGBA.
#[derive(Debug, Clone)]
pub struct Thumbnail {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    /// libheif's error code, or 0 for errors raised on this side.
    pub code: i32,
    pub message: String,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "HEIF: {} (code {})", self.message, self.code)
    }
}

impl std::error::Error for Error {}

/// Decode settings.
#[derive(Debug, Clone, Copy)]
pub struct Options {
    /// Threads libheif may use to decode tiles in parallel. Fixed rather
    /// than "all cores", so batch work on the rayon pool is not
    /// oversubscribed. Use 1 for batch jobs, more to open one photo quickly.
    pub threads: u8,
}

impl Default for Options {
    fn default() -> Self {
        Self { threads: 4 }
    }
}

fn init() {
    static INIT: Once = Once::new();
    // SAFETY: heif_init is safe to call once, before any other use. It is
    // never paired with heif_deinit; the library lives as long as the process.
    #[allow(unsafe_code)]
    INIT.call_once(|| unsafe { ffi::gz_init() });
}

/// The version string of the linked libheif.
pub fn linked_version() -> String {
    init();
    // SAFETY: returns a pointer to a static, NUL-terminated string.
    #[allow(unsafe_code)]
    unsafe {
        CStr::from_ptr(ffi::gz_version())
            .to_string_lossy()
            .into_owned()
    }
}

fn error(code: c_int, message: *const c_char) -> Error {
    let message = if message.is_null() {
        "unknown error".to_owned()
    } else {
        // SAFETY: libheif error messages are NUL-terminated and live at least
        // as long as the context, which is still open here.
        #[allow(unsafe_code)]
        unsafe {
            CStr::from_ptr(message).to_string_lossy().into_owned()
        }
    };
    Error {
        code,
        message,
    }
}

fn local(message: &str) -> Error {
    Error {
        code: 0,
        message: message.to_owned(),
    }
}

/// An open file. Owns the libheif context and the primary image handle, and
/// borrows the bytes, which libheif reads without copying.
struct File<'a> {
    ctx: *mut ffi::Context,
    primary: *mut ffi::Handle,
    _data: &'a [u8],
}

impl Drop for File<'_> {
    fn drop(&mut self) {
        // SAFETY: both pointers came from libheif and are released once.
        #[allow(unsafe_code)]
        unsafe {
            if !self.primary.is_null() {
                ffi::gz_handle_free(self.primary);
            }
            ffi::gz_context_free(self.ctx);
        }
    }
}

#[allow(unsafe_code)]
impl<'a> File<'a> {
    fn open(data: &'a [u8], options: Options) -> Result<Self, Error> {
        init();
        // SAFETY: plain allocation; checked for null below.
        let ctx = unsafe { ffi::gz_context_new(c_int::from(options.threads.max(1))) };
        if ctx.is_null() {
            return Err(local("out of memory"));
        }
        let mut file = File {
            ctx,
            primary: ptr::null_mut(),
            _data: data,
        };
        let mut msg = ptr::null();
        // SAFETY: `data` outlives `file`, which owns the context reading it.
        let code = unsafe { ffi::gz_context_read(ctx, data.as_ptr(), data.len(), &mut msg) };
        if code != 0 {
            return Err(error(code, msg));
        }
        // SAFETY: the context is valid; the handle is owned by `file`.
        let code = unsafe { ffi::gz_primary_handle(ctx, &mut file.primary, &mut msg) };
        if code != 0 {
            return Err(error(code, msg));
        }
        Ok(file)
    }

    fn color(&self) -> Result<ColorProfile, Error> {
        let (mut p, mut t, mut m, mut f) = (0, 0, 0, 0);
        // SAFETY: valid handle; the out-params are plain ints.
        let kind = unsafe { ffi::gz_color(self.primary, &mut p, &mut t, &mut m, &mut f) };
        Ok(match kind {
            1 => ColorProfile::Nclx {
                primaries: p as u16,
                transfer: t as u16,
                matrix: m as u16,
                full_range: f != 0,
            },
            2 => {
                // SAFETY: the buffer has exactly the size libheif reports.
                let len = unsafe { ffi::gz_icc_size(self.primary) };
                let mut icc = vec![0u8; len];
                let mut msg = ptr::null();
                let code = unsafe { ffi::gz_icc_copy(self.primary, icc.as_mut_ptr(), &mut msg) };
                if code != 0 {
                    return Err(error(code, msg));
                }
                ColorProfile::Icc(icc)
            }
            _ => ColorProfile::None,
        })
    }

    fn exif(&self) -> Result<Option<Vec<u8>>, Error> {
        let mut id = 0u32;
        // SAFETY: valid handle; `id` is written when a block exists.
        let len = unsafe { ffi::gz_exif_size(self.primary, &mut id) };
        if len == 0 {
            return Ok(None);
        }
        let mut block = vec![0u8; len];
        let mut msg = ptr::null();
        // SAFETY: the buffer has exactly the size libheif reports.
        let code = unsafe { ffi::gz_exif_copy(self.primary, id, block.as_mut_ptr(), &mut msg) };
        if code != 0 {
            return Err(error(code, msg));
        }
        Ok(strip_exif_offset(block))
    }
}

/// HEIF prefixes the EXIF payload with a big-endian offset to the TIFF
/// header. Returns the TIFF data, or None if the block is malformed.
fn strip_exif_offset(block: Vec<u8>) -> Option<Vec<u8>> {
    let offset = u32::from_be_bytes(block.get(0..4)?.try_into().ok()?) as usize;
    let start = offset.checked_add(4)?;
    (start <= block.len()).then(|| block[start..].to_vec())
}

/// A decoded libheif image, released on drop.
struct Pixels {
    img: *mut ffi::Image,
}

impl Drop for Pixels {
    fn drop(&mut self) {
        // SAFETY: the pointer came from libheif and is released once.
        #[allow(unsafe_code)]
        unsafe {
            ffi::gz_image_free(self.img)
        }
    }
}

/// Decodes `handle` to RGBA and hands each row to `row` as bytes.
#[allow(unsafe_code)]
fn decode_handle(
    handle: *const ffi::Handle,
    wide: bool,
    mut row: impl FnMut(&[u8]),
) -> Result<(u32, u32, u8), Error> {
    let mut img = ptr::null_mut();
    let mut msg = ptr::null();
    // SAFETY: valid handle; the image is owned by `Pixels` below.
    let code = unsafe { ffi::gz_decode(handle, c_int::from(wide), &mut img, &mut msg) };
    if code != 0 {
        return Err(error(code, msg));
    }
    let pixels = Pixels { img };
    let (mut w, mut h, mut stride, mut bits) = (0, 0, 0usize, 0);
    // SAFETY: valid image; the plane stays valid while `pixels` lives.
    let base = unsafe { ffi::gz_image_pixels(pixels.img, &mut w, &mut h, &mut stride, &mut bits) };
    if base.is_null() || w <= 0 || h <= 0 {
        return Err(local("decoded image has no pixels"));
    }
    let row_len = w as usize * 4 * if wide { 2 } else { 1 };
    if stride < row_len {
        return Err(local("decoded image has a short stride"));
    }
    for y in 0..h as usize {
        // SAFETY: libheif guarantees `h` rows of `stride` bytes, and each row
        // holds at least `row_len` bytes (checked above).
        let line = unsafe { std::slice::from_raw_parts(base.add(y * stride), row_len) };
        row(line);
    }
    Ok((w as u32, h as u32, bits.clamp(1, 16) as u8))
}

/// Widens a source value of `bits` bits to the full 16-bit range, by bit
/// replication so that the maximum maps to 65535.
fn widen(v: u16, bits: u8) -> u16 {
    match bits {
        16 => v,
        b if b >= 8 => (v << (16 - b)) | (v >> (2 * b - 16)),
        b => {
            // Below 8 bits: repeat the pattern until 16 bits are filled.
            let mut out = 0u32;
            let mut filled = 0;
            while filled < 16 {
                out = (out << b) | u32::from(v);
                filled += b;
            }
            (out >> (filled - 16)) as u16
        }
    }
}

/// Decodes the primary image of a HEIC or HEIF file.
pub fn decode(data: &[u8], options: Options) -> Result<Decoded, Error> {
    let file = File::open(data, options)?;
    let (mut w, mut h, mut alpha, mut luma) = (0, 0, 0, 0);
    // SAFETY: valid handle; the out-params are plain ints.
    #[allow(unsafe_code)]
    unsafe {
        ffi::gz_handle_info(file.primary, &mut w, &mut h, &mut alpha, &mut luma)
    };
    if w <= 0 || h <= 0 {
        return Err(local("image has no size"));
    }
    let wide = luma > 8;
    let mut rgba = Vec::with_capacity(w as usize * h as usize * 4);
    let mut bits = 8;
    let (width, height, decoded_bits) = decode_handle(file.primary, wide, |line| {
        if wide {
            rgba.extend(
                line.chunks_exact(2)
                    .map(|b| u16::from_le_bytes([b[0], b[1]])),
            );
        } else {
            rgba.extend(line.iter().map(|&b| u16::from(b)));
        }
    })?;
    if wide {
        bits = decoded_bits;
    }
    for v in &mut rgba {
        *v = widen(*v, bits);
    }
    Ok(Decoded {
        width,
        height,
        source_bits: luma.clamp(1, 16) as u8,
        has_alpha: alpha != 0,
        color: file.color()?,
        exif: file.exif()?,
        rgba,
    })
}

/// Decodes the file's embedded thumbnail, if it has one. Much faster than a
/// full decode; used to show first pixels while the full image decodes.
pub fn decode_thumbnail(data: &[u8]) -> Result<Option<Thumbnail>, Error> {
    let file = File::open(data, Options { threads: 1 })?;
    // SAFETY: valid primary handle; the thumbnail handle is released below.
    #[allow(unsafe_code)]
    let thumb = unsafe { ffi::gz_thumbnail(file.primary) };
    if thumb.is_null() {
        return Ok(None);
    }
    let mut rgba = Vec::new();
    let result = decode_handle(thumb, false, |line| rgba.extend_from_slice(line));
    // SAFETY: released exactly once, after its last use.
    #[allow(unsafe_code)]
    unsafe {
        ffi::gz_handle_free(thumb)
    };
    let (width, height, _) = result?;
    Ok(Some(Thumbnail {
        width,
        height,
        rgba,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn sample(name: &str) -> Vec<u8> {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("vendor/libheif/tests/data")
            .join(name);
        std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    }

    #[test]
    fn links_the_bundled_libheif() {
        assert_eq!(linked_version(), LIBHEIF_VERSION);
    }

    #[test]
    fn decodes_a_heic() {
        let img = decode(&sample("rainbow-451x461.heic"), Options::default()).unwrap();
        assert_eq!((img.width, img.height), (451, 461));
        assert_eq!(img.rgba.len(), 451 * 461 * 4);
        assert!(!img.has_alpha);
        // Opaque: alpha is fully on everywhere.
        assert!(img.rgba.chunks_exact(4).all(|px| px[3] == u16::MAX));
        // A rainbow has strong reds, greens and blues somewhere.
        for c in 0..3 {
            let max = img.rgba.chunks_exact(4).map(|px| px[c]).max().unwrap();
            assert!(max > 50_000, "channel {c} peaks at {max}");
        }
    }

    #[test]
    fn decodes_alpha() {
        let img = decode(&sample("with-alpha-512x512.heic"), Options::default()).unwrap();
        assert!(img.has_alpha);
        assert!(img.rgba.chunks_exact(4).any(|px| px[3] < u16::MAX));
    }

    #[test]
    fn rejects_garbage() {
        assert!(decode(b"definitely not a heic file", Options::default()).is_err());
        assert!(decode(&[], Options::default()).is_err());
        let mut truncated = sample("rainbow-451x461.heic");
        truncated.truncate(truncated.len() / 3);
        assert!(decode(&truncated, Options::default()).is_err());
    }

    #[test]
    fn widening_hits_both_ends() {
        assert_eq!(widen(0, 8), 0);
        assert_eq!(widen(255, 8), 65535);
        assert_eq!(widen(0x80, 8), 0x8080);
        assert_eq!(widen(1023, 10), 65535);
        assert_eq!(widen(4095, 12), 65535);
        assert_eq!(widen(0, 10), 0);
    }

    #[test]
    fn exif_offset_is_stripped() {
        assert_eq!(
            strip_exif_offset(vec![0, 0, 0, 0, b'I', b'I']),
            Some(vec![b'I', b'I'])
        );
        assert_eq!(
            strip_exif_offset(vec![0, 0, 0, 2, 9, 9, b'M', b'M']),
            Some(vec![b'M', b'M'])
        );
        assert_eq!(strip_exif_offset(vec![0, 0, 0, 9, 1]), None);
        assert_eq!(strip_exif_offset(vec![1, 2]), None);
    }
}
