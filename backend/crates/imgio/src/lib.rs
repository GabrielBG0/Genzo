//! Decoding and encoding of image files.
//!
//! Every decoder returns the same normalized [`DecodedImage`], so the
//! pipeline never sees a file format. HEIC and HEIF decode through the
//! bundled libheif (the `heic` feature). The RAW decoders (rawler, LibRaw)
//! and JPEG, TIFF and PNG are still to come: the RAW ones with the 0.C spike,
//! after their license review in docs/licenses.md.

mod heic;

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

#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    #[error("unrecognized file format")]
    UnknownFormat,
    #[error("decoding {0:?} is not implemented yet")]
    Unsupported(FileFormat),
    #[error("{0:?} support is not included in this build (the `{1}` feature is off)")]
    NotBuiltIn(FileFormat, &'static str),
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
}

/// Pixel data of a decoded image.
#[derive(Debug, Clone)]
pub enum Pixels {
    /// Interleaved RGBA, 16 bits per channel over the full 0..=65535 range,
    /// encoded per the image's [`ColorEncoding`] (not linear). Non-RAW
    /// formats decode to this.
    Rgba16(Vec<u16>),
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
    pub color: ColorEncoding,
    pub pixels: Pixels,
    /// Raw EXIF (TIFF structure), for metadata. Its orientation tag is
    /// already reflected in the pixels and must not be applied again.
    pub exif: Option<Vec<u8>>,
}

/// A small embedded preview, shown while the full image decodes.
#[derive(Debug, Clone)]
pub struct Preview {
    pub width: u32,
    pub height: u32,
    /// Interleaved RGBA, 8 bits per channel.
    pub rgba: Vec<u8>,
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
        other => Err(DecodeError::Unsupported(other)),
    }
}

/// Decodes a file's embedded preview, if it has one. Fast enough for the
/// "first pixels within one frame" budget.
pub fn decode_preview(data: &[u8]) -> Result<Option<Preview>, DecodeError> {
    match sniff(data).ok_or(DecodeError::UnknownFormat)? {
        FileFormat::Heif => heic::decode_preview(data),
        other => Err(DecodeError::Unsupported(other)),
    }
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
        assert!(matches!(
            decode(&[0xFF, 0xD8, 0xFF, 0xE1], DecodeOptions::default()),
            Err(DecodeError::Unsupported(FileFormat::Jpeg))
        ));
    }

    #[test]
    fn unknown_and_short_input() {
        assert_eq!(sniff(b""), None);
        assert_eq!(sniff(b"\0\0\0\x18ftyp"), None);
        assert_eq!(sniff(b"\0\0\0\x18ftypisom"), None);
        assert_eq!(sniff(b"GIF89a"), None);
    }
}
