//! Decoding and encoding of image files.
//!
//! The decoders (rawler, LibRaw, libheif, zune-jpeg) are not adopted yet:
//! the M0 license review (docs/licenses.md) has to clear them first. Until
//! then this crate only identifies file formats, which is enough for folder
//! listing and gives the fuzzer a first target.

use genzo_model::frame::Rect;

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

/// The normalized decode output every decoder will produce. The pipeline
/// never sees a file format. Filled in by the 0.C decode spike.
#[derive(Debug)]
pub struct DecodedImage {
    pub width: u32,
    pub height: u32,
    /// Area of the sensor that holds image data.
    pub active_area: Rect,
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
    fn unknown_and_short_input() {
        assert_eq!(sniff(b""), None);
        assert_eq!(sniff(b"\0\0\0\x18ftyp"), None);
        assert_eq!(sniff(b"\0\0\0\x18ftypisom"), None);
        assert_eq!(sniff(b"GIF89a"), None);
    }
}
