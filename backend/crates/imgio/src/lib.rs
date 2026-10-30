//! Decoding and encoding of image files.
//!
//! Every decoder returns the same normalized [`DecodedImage`], so the
//! pipeline never sees a file format (DEV-01, stage 0). HEIC and HEIF decode
//! through the bundled libheif (the `heic` feature). Linear DNGs (ProRAW,
//! including JPEG XL tiles) have their own reader in [`dng`] (the `dng`
//! feature). rawler for CFA DNG and camera RAW, and JPEG, TIFF and PNG, are
//! still to come.

mod bands;
#[cfg(feature = "raw")]
mod camera;
pub mod capture;
#[cfg(feature = "dng")]
pub mod dng;
mod exif;
mod heic;
pub mod jpeg;
#[cfg(feature = "raw")]
mod ljpeg;
pub mod opcodes;
mod png_image;
mod preview;
mod raster;
pub mod raw;
mod tiff_ifd;
mod tiff_image;

pub use raw::RawMetadata;

#[doc(hidden)]
pub use bands::check as check_preview_bands;
#[doc(hidden)]
pub use preview::check_bands_in;

/// A file format Genzo recognizes from its first bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FileFormat {
    Jpeg,
    Png,
    /// TIFF container: plain TIFF, DNG and most TIFF-based RAW formats
    /// (NEF, ARW, CR2, PEF, ...). Told apart later by their tags.
    Tiff,
    /// ISO base media container with an image brand: HEIC, HEIF, AVIF.
    Heif,
    /// Canon CR3 (ISO base media with the `crx ` brand).
    Cr3,
    /// Fujifilm RAF.
    Raf,
    /// Panasonic RW2 (a TIFF variant with its own magic).
    Rw2,
    /// Olympus and OM System ORF (TIFF variants with their own magic).
    Orf,
}

/// Why a file did not decode. The engine shows the photo's embedded preview
/// for all of them (DEV-01, States): `Corrupt` is the "damaged" state, every
/// other variant "unsupported".
#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    #[error("unrecognized file format")]
    UnknownFormat,
    /// A format, or a variant of one, Genzo does not decode (yet).
    #[error("{0:?} file not supported: {1}")]
    Unsupported(FileFormat, String),
    #[error("{0:?} support is not included in this build (the `{1}` feature is off)")]
    NotBuiltIn(FileFormat, &'static str),
    /// Truncated, or failing a decoder's bounds checks.
    #[error("corrupt or unreadable {format:?} file: {message}")]
    Corrupt { format: FileFormat, message: String },
}

/// Identifies a file from its first bytes. Never panics, whatever the input.
pub fn sniff(head: &[u8]) -> Option<FileFormat> {
    if head.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some(FileFormat::Jpeg);
    }
    if head.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some(FileFormat::Png);
    }
    if head.starts_with(b"FUJIFILMCCD-RAW") {
        return Some(FileFormat::Raf);
    }
    if head.starts_with(b"IIU\0") {
        return Some(FileFormat::Rw2);
    }
    if head.starts_with(b"IIRO") || head.starts_with(b"IIRS") || head.starts_with(b"MMOR") {
        return Some(FileFormat::Orf);
    }
    if head.starts_with(b"II*\0") || head.starts_with(b"MM\0*") {
        return Some(FileFormat::Tiff);
    }
    if head.len() >= 12 && &head[4..8] == b"ftyp" {
        return match &head[8..12] {
            b"crx " => Some(FileFormat::Cr3),
            b"heic" | b"heix" | b"heim" | b"heis" | b"hevc" | b"hevx" | b"mif1" | b"msf1"
            | b"avif" => Some(FileFormat::Heif),
            _ => None,
        };
    }
    None
}

/// How the pixel values of a decoded image are encoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ColorEncoding {
    /// sRGB primaries and transfer function.
    Srgb,
    /// Display P3: DCI-P3 primaries, D65 white, sRGB transfer function.
    /// What iPhones tag their HEIC photos with.
    DisplayP3,
    /// ITU-T H.273 code points with no shortcut above.
    Nclx {
        primaries: u16,
        transfer: u16,
        matrix: u16,
        full_range: bool,
    },
    /// An embedded ICC profile, applied by the color crate.
    Icc(Vec<u8>),
    /// Adobe RGB (1998), declared the DCF way by cameras that embed no
    /// profile: EXIF ColorSpace 0xFFFF and Interoperability index "R03".
    /// An embedded profile takes precedence.
    AdobeRgb,
}

/// Pixel data of a decoded image, with what is needed to interpret it.
#[derive(Debug, Clone)]
pub enum Pixels {
    /// Interleaved RGBA, 16 bits per channel over the full 0..=65535 range,
    /// encoded per `color` (not linear). Non-RAW formats decode to this.
    Rgba16 {
        data: Vec<u16>,
        color: ColorEncoding,
    },
    /// Linear camera samples of a RAW file that needs no demosaic (ProRAW
    /// and other LinearRaw DNGs), row by row: 3 interleaved samples per
    /// pixel, or 1 for a monochrome image. They are after the
    /// LinearizationTable and before the levels, white balance and
    /// BaselineExposure, which the pipeline applies from `raw`.
    Linear {
        samples_per_pixel: u8,
        data: Vec<u16>,
        raw: Box<RawMetadata>,
    },
    /// The sensor mosaic of a camera RAW or CFA DNG, one sample per site,
    /// row by row over the whole sensor (masked borders included; the active
    /// area is in `raw`). Before the levels and the demosaic, which the
    /// pipeline applies.
    Cfa {
        pattern: raw::CfaPattern,
        data: Vec<u16>,
        raw: Box<RawMetadata>,
    },
}

/// The normalized decode output every decoder produces.
#[derive(Debug, Clone)]
pub struct DecodedImage {
    pub format: FileFormat,
    pub width: u32,
    pub height: u32,
    /// Bits per channel in the file, before widening to 16.
    pub source_bits: u8,
    pub has_alpha: bool,
    pub pixels: Pixels,
    /// The EXIF or DNG orientation, 1 to 8, *not* applied to the pixels: the
    /// pipeline's geometry step applies it (DEV-01, Geometry). HEIC reports
    /// 1, since libheif applies the container's rotation and mirror itself.
    pub orientation: u8,
    /// Raw EXIF (TIFF structure), for metadata. Use `orientation`, never the
    /// orientation tag in here.
    pub exif: Option<Vec<u8>>,
}

/// A small embedded preview, shown while the full image decodes.
#[derive(Debug, Clone)]
pub struct Preview {
    pub width: u32,
    pub height: u32,
    /// Interleaved RGBA, 8 bits per channel.
    pub rgba: Vec<u8>,
    /// The orientation to show it in, 1 to 8, not applied to the pixels, as
    /// for [`DecodedImage`]. Cameras store previews unrotated, so this is
    /// the file's orientation.
    pub orientation: u8,
    /// How its 8-bit values are encoded: sRGB unless the preview or its
    /// file says otherwise.
    pub color: ColorEncoding,
}

/// Decode settings.
#[derive(Debug, Clone, Copy)]
pub struct DecodeOptions {
    /// Threads a decoder may use inside one file (HEIC tiles). Use 1 for
    /// batch work on the rayon pool, more to open one photo quickly.
    pub threads: u8,
}

impl Default for DecodeOptions {
    fn default() -> Self {
        Self { threads: 4 }
    }
}

/// Decodes a file's full image.
pub fn decode(data: &[u8], options: DecodeOptions) -> Result<DecodedImage, DecodeError> {
    match sniff(data).ok_or(DecodeError::UnknownFormat)? {
        FileFormat::Heif => heic::decode(data, options),
        FileFormat::Jpeg => jpeg::decode(data),
        FileFormat::Png => png_image::decode(data),
        FileFormat::Tiff => decode_tiff(data),
        f @ (FileFormat::Cr3 | FileFormat::Raf | FileFormat::Rw2 | FileFormat::Orf) => {
            decode_camera(data, f)
        }
    }
}

/// Camera RAW through rawler (the `raw` feature).
#[cfg(feature = "raw")]
fn decode_camera(data: &[u8], format: FileFormat) -> Result<DecodedImage, DecodeError> {
    camera::decode(data, format)
}

#[cfg(not(feature = "raw"))]
fn decode_camera(_: &[u8], format: FileFormat) -> Result<DecodedImage, DecodeError> {
    Err(DecodeError::NotBuiltIn(format, "raw"))
}

/// The three kinds of file behind the TIFF magic: DNG (told apart by its
/// DNGVersion tag), TIFF-based camera RAW (NEF, ARW, 3FR, IIQ, ...), and
/// plain TIFF images.
///
/// A non-DNG TIFF goes to the camera decoder first. Only when that fails,
/// and IFD0 holds a full image the way a plain TIFF does, is it read as a
/// TIFF image: an exported TIFF whose Make names a camera still opens, and a
/// damaged camera RAW never comes back as its thumbnail.
fn decode_tiff(data: &[u8]) -> Result<DecodedImage, DecodeError> {
    if tiff_ifd::ifd0_has_tag(data, tiff_ifd::DNG_VERSION) {
        return decode_dng(data);
    }
    let camera = decode_camera(data, FileFormat::Tiff);
    match camera {
        Ok(img) => Ok(img),
        Err(e) if tiff_image::looks_like_plain_tiff(data) => {
            tiff_image::decode(data).map_err(|tiff_error| match e {
                // No camera format matched: the TIFF decoder's error is the
                // relevant one.
                DecodeError::NotBuiltIn(..) => tiff_error,
                DecodeError::Unsupported(_, ref m) if m == NOT_A_CAMERA_RAW => tiff_error,
                camera_error => camera_error,
            })
        }
        Err(e) => Err(e),
    }
}

/// What the camera decoder says of a TIFF that is no camera RAW it knows.
pub(crate) const NOT_A_CAMERA_RAW: &str =
    "neither a camera RAW format Genzo reads nor a plain TIFF image";

#[cfg(feature = "dng")]
fn decode_dng(data: &[u8]) -> Result<DecodedImage, DecodeError> {
    use dng::DngError;
    let format = FileFormat::Tiff;
    let corrupt = |message: String| DecodeError::Corrupt { format, message };
    match dng::photometric(data) {
        Ok(dng::PHOTOMETRIC_LINEAR_DNG) => {}
        Ok(dng::PHOTOMETRIC_CFA_DNG) => return decode_cfa_dng(data),
        Ok(p) => {
            return Err(DecodeError::Unsupported(
                format,
                format!("a DNG with photometric interpretation {p}"),
            ));
        }
        Err(e) => return Err(corrupt(e.to_string())),
    }
    let img = dng::decode_linear(data).map_err(|e| match e {
        DngError::Unsupported(m) => DecodeError::Unsupported(format, m),
        DngError::NotBuiltIn(_, feature) => DecodeError::NotBuiltIn(format, feature),
        DngError::Corrupt(m) => corrupt(m),
        e @ (DngError::NotTiff | DngError::NoRawImage | DngError::TooLarge(_)) => {
            corrupt(e.to_string())
        }
    })?;
    if !matches!(img.samples_per_pixel, 1 | 3) {
        return Err(DecodeError::Unsupported(
            format,
            format!(
                "a linear DNG with {} samples per pixel",
                img.samples_per_pixel
            ),
        ));
    }
    Ok(DecodedImage {
        format,
        width: img.width,
        height: img.height,
        source_bits: img.bits_per_sample as u8,
        has_alpha: false,
        pixels: Pixels::Linear {
            samples_per_pixel: img.samples_per_pixel,
            data: img.pixels,
            raw: Box::new(with_opcode_plan(img.meta)?),
        },
        orientation: img.orientation,
        exif: None,
    })
}

/// Fills a DNG's opcode plan; an opcode the pipeline cannot honor makes the
/// file unsupported (DEV-01, decided 2026-10-18).
#[cfg(feature = "dng")]
pub(crate) fn with_opcode_plan(mut meta: RawMetadata) -> Result<RawMetadata, DecodeError> {
    meta.opcode_plan = opcodes::plan(&meta.opcodes)
        .map_err(|why| DecodeError::Unsupported(FileFormat::Tiff, why))?;
    Ok(meta)
}

#[cfg(not(feature = "dng"))]
fn decode_dng(_: &[u8]) -> Result<DecodedImage, DecodeError> {
    Err(DecodeError::NotBuiltIn(FileFormat::Tiff, "dng"))
}

#[cfg(all(feature = "dng", feature = "raw"))]
fn decode_cfa_dng(data: &[u8]) -> Result<DecodedImage, DecodeError> {
    camera::decode_dng(data)
}

#[cfg(all(feature = "dng", not(feature = "raw")))]
fn decode_cfa_dng(_: &[u8]) -> Result<DecodedImage, DecodeError> {
    Err(DecodeError::NotBuiltIn(FileFormat::Tiff, "raw"))
}

/// Decodes a file's embedded preview, if it has one. Fast enough for the
/// "first pixels within one frame" budget.
///
/// Everything but HEIC is searched for embedded JPEG streams (see
/// `preview.rs`), so a file whose image cannot be decoded (Nikon High
/// Efficiency, a damaged file) still shows its preview. `Ok(None)` when the
/// file holds none: PNG, a plain TIFF, Phase One IIQ, Kodak DCR.
pub fn decode_preview(data: &[u8]) -> Result<Option<Preview>, DecodeError> {
    match sniff(data).ok_or(DecodeError::UnknownFormat)? {
        FileFormat::Heif => heic::decode_preview(data),
        // Only the EXIF thumbnail is a JPEG file's preview: not its own
        // image, nor the MPF images or gain maps (Ultra HDR) after it.
        FileFormat::Jpeg => Ok(jpeg_app1_exif(data)
            .and_then(|app1| preview::decode(app1, &preview_hints(data, FileFormat::Jpeg)))),
        format => Ok(preview::decode(data, &preview_hints(data, format))),
    }
}

/// What a file declares about its preview: the orientation and the DCF
/// Adobe RGB marker, from IFD0 of TIFF-based files, from the CMT1 box of a
/// CR3, and from the APP1 EXIF of a JPEG (whose preview is its thumbnail).
/// RAF and the rest leave it to the preview's own EXIF.
fn preview_hints(data: &[u8], format: FileFormat) -> preview::FileHints {
    let tiff: Option<&[u8]> = match format {
        FileFormat::Tiff | FileFormat::Orf | FileFormat::Rw2 => Some(data),
        FileFormat::Cr3 => cr3_box(data, b"CMT1"),
        FileFormat::Jpeg => jpeg_app1_exif(data).map(exif::tiff_payload),
        _ => None,
    };
    let Some(tiff) = tiff else {
        return preview::FileHints::default();
    };
    preview::FileHints {
        orientation: exif::orientation_tag(tiff),
        color: exif::is_adobe_rgb(tiff).then_some(ColorEncoding::AdobeRgb),
    }
}

/// The TIFF structure in one of a CR3's CMT boxes (CMT1 is IFD0 of the
/// camera's EXIF, CMT2 its EXIF IFD), found by its box type in the file's
/// head.
fn cr3_box<'a>(data: &'a [u8], kind: &[u8; 4]) -> Option<&'a [u8]> {
    let head = &data[..data.len().min(1 << 20)];
    let at = memchr::memmem::find(head, kind)?;
    let box_start = at.checked_sub(4)?;
    let size = u32::from_be_bytes(data.get(box_start..at)?.try_into().ok()?) as usize;
    data.get(at + 4..box_start.checked_add(size)?)
}

/// The body of a JPEG's APP1 EXIF segment, walking the markers from SOI.
fn jpeg_app1_exif(data: &[u8]) -> Option<&[u8]> {
    let mut at = 2;
    for _ in 0..64 {
        if *data.get(at)? != 0xFF {
            return None;
        }
        let marker = *data.get(at + 1)?;
        let len = usize::from(u16::from_be_bytes([*data.get(at + 2)?, *data.get(at + 3)?]));
        let body = data.get(at + 4..at + 2 + len)?;
        if marker == 0xE1 && body.starts_with(b"Exif\0\0") {
            return Some(body);
        }
        if marker == 0xDA || len < 2 {
            return None;
        }
        at += 2 + len;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniffs_common_formats() {
        assert_eq!(sniff(&[0xFF, 0xD8, 0xFF, 0xE1]), Some(FileFormat::Jpeg));
        assert_eq!(sniff(b"\x89PNG\r\n\x1a\n...."), Some(FileFormat::Png));
        assert_eq!(sniff(b"II*\0\x08\0\0\0"), Some(FileFormat::Tiff));
        assert_eq!(sniff(b"MM\0*\0\0\0\x08"), Some(FileFormat::Tiff));
        assert_eq!(sniff(b"\0\0\0\x18ftypheic"), Some(FileFormat::Heif));
        assert_eq!(sniff(b"\0\0\0\x18ftypcrx "), Some(FileFormat::Cr3));
        assert_eq!(sniff(b"FUJIFILMCCD-RAW 0201"), Some(FileFormat::Raf));
        assert_eq!(sniff(b"IIU\0"), Some(FileFormat::Rw2));
        assert_eq!(sniff(b"IIRO"), Some(FileFormat::Orf));
    }

    #[test]
    fn unsupported_formats_say_so() {
        assert!(matches!(
            decode(b"GIF89a", DecodeOptions::default()),
            Err(DecodeError::UnknownFormat)
        ));
        // A JPEG cut short is damaged, not unsupported.
        assert!(matches!(
            decode(&[0xFF, 0xD8, 0xFF, 0xE1], DecodeOptions::default()),
            Err(DecodeError::Corrupt {
                format: FileFormat::Jpeg,
                ..
            })
        ));
        // A TIFF with an empty IFD0: no camera RAW and no image.
        let plain_tiff = b"II*\0\x08\0\0\0\0\0\0\0\0\0";
        let err = decode(plain_tiff, DecodeOptions::default()).unwrap_err();
        if cfg!(feature = "raw") {
            assert!(
                matches!(&err, DecodeError::Unsupported(FileFormat::Tiff, m) if m == NOT_A_CAMERA_RAW),
                "{err:?}"
            );
        } else {
            // Not a DNG, and no camera decoder in this build.
            assert!(matches!(
                err,
                DecodeError::NotBuiltIn(FileFormat::Tiff, "raw")
            ));
        }
    }

    #[test]
    fn unknown_and_short_input() {
        assert_eq!(sniff(b""), None);
        assert_eq!(sniff(b"\0\0\0\x18ftyp"), None);
        assert_eq!(sniff(b"\0\0\0\x18ftypisom"), None);
        assert_eq!(sniff(b"GIF89a"), None);
    }
}
