//! A photo's EXIF, read from any format Genzo opens and written again for
//! an exported JPEG (OUT-01, *Metadata*).
//!
//! - **Read** ([`read`]): IFD0's descriptive tags, the Exif IFD and the GPS
//!   IFD, wherever the format keeps them: a JPEG's APP1, a PNG's eXIf
//!   chunk, a HEIC's EXIF item, IFD0 of a TIFF-based RAW, DNG or TIFF, a
//!   CR3's CMT1, CMT2 and CMT4 boxes, a RAF's embedded JPEG. A TIFF/EP file
//!   without an Exif IFD (an old Kodak DCR) keeps the Exif tags in IFD0.
//! - **Not copied byte for byte.** A RAW's IFD0 also holds its image
//!   structure (strips, SubIFDs, DNG tags), so only IFD0's descriptive
//!   tags are kept, by a list. IFD pointers are dropped and written anew;
//!   IFD1 (the thumbnail) and the interoperability IFD are not read.
//! - **Written** ([`write`]) as a little-endian TIFF structure in an APP1
//!   payload (`Exif\0\0` first): orientation 1, the export's pixel
//!   dimensions, Genzo as the software and the source's software as
//!   ProcessingSoftware. ColorSpace is sRGB, as the export is. The tags a
//!   JPEG's EXIF requires are added when the source lacks them (a RAW
//!   rarely has YCbCrPositioning or ComponentsConfiguration): the
//!   positioning and the components. The Exif and GPS versions are always
//!   2.32 and 2.3, the structure written.
//!   A payload over APP1's 65,533 bytes drops the
//!   MakerNote first, then the largest tags, and says which.
//! - Every value is kept as the source wrote it, converted to little
//!   endian element by element. A MakerNote is opaque bytes: its offsets
//!   are its camera's business, as in other tools.

use crate::tiff_ifd::{Entry, Tiff, find};
use crate::{FileFormat, exif, sniff};

const EXIF_IFD: u16 = 34665;
const GPS_IFD: u16 = 34853;
const INTEROP_IFD: u16 = 40965;
const ORIENTATION: u16 = 274;
const SOFTWARE: u16 = 305;
const PROCESSING_SOFTWARE: u16 = 11;
const MAKER_NOTE: u16 = 37500;
const PIXEL_X_DIMENSION: u16 = 40962;
const PIXEL_Y_DIMENSION: u16 = 40963;
const COLOR_SPACE: u16 = 40961;
const EXIF_VERSION: u16 = 0x9000;
const COMPONENTS_CONFIGURATION: u16 = 0x9101;
const Y_CB_CR_POSITIONING: u16 = 0x0213;
const GPS_VERSION_ID: u16 = 0;

/// The most an APP1 segment's payload holds: its length field counts
/// itself and is 16 bits.
pub const APP1_MAX: usize = 65_533;

/// IFD0's tags an export keeps, with the type each must have: what
/// describes the photo, never how a file stores its pixels. The type is
/// checked too: Panasonic's RW2 reuses some of these numbers in its IFD0
/// for tags of its own.
const IFD0_KEPT: [(u16, u16); 11] = [
    (270, 2),   // ImageDescription
    (271, 2),   // Make
    (272, 2),   // Model
    (282, 5),   // XResolution
    (283, 5),   // YResolution
    (296, 3),   // ResolutionUnit
    (305, 2),   // Software, written again as ProcessingSoftware
    (306, 2),   // DateTime
    (315, 2),   // Artist
    (316, 2),   // HostComputer
    (33432, 2), // Copyright
];

/// Whether IFD0's `t` is one an export keeps.
fn kept_in_ifd0(t: &Tag) -> bool {
    IFD0_KEPT.contains(&(t.tag, t.ty))
}

/// One tag as it is written: its type, its count and its values, little
/// endian.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tag {
    pub tag: u16,
    pub ty: u16,
    pub count: u32,
    pub value: Vec<u8>,
}

/// The EXIF of a photo, as an export writes it again.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Metadata {
    pub ifd0: Vec<Tag>,
    pub exif: Vec<Tag>,
    pub gps: Vec<Tag>,
}

impl Metadata {
    pub fn is_empty(&self) -> bool {
        self.ifd0.is_empty() && self.exif.is_empty() && self.gps.is_empty()
    }

    /// The tag `tag` of the Exif IFD.
    pub fn exif_tag(&self, tag: u16) -> Option<&Tag> {
        self.exif.iter().find(|t| t.tag == tag)
    }
}

/// An exported JPEG's APP1 payload, and what did not fit in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Written {
    /// `Exif\0\0` and the TIFF structure.
    pub app1: Vec<u8>,
    /// Tags dropped to fit APP1, MakerNote first.
    pub dropped: Vec<u16>,
}

/// The EXIF of the file in `data`, by its format; `None` when it has none
/// or it is unreadable. Never panics, whatever the input.
pub fn read(data: &[u8]) -> Option<Metadata> {
    let m = match sniff(data)? {
        FileFormat::Jpeg => from_exif(crate::jpeg_app1_exif(data)?)?,
        FileFormat::Png => from_exif(crate::png_image::exif_chunk(data)?)?,
        FileFormat::Heif => from_exif(&crate::heic::exif(data)?)?,
        FileFormat::Tiff | FileFormat::Orf | FileFormat::Rw2 => from_tiff(data)?,
        FileFormat::Raf => {
            // The RAF header points to the embedded JPEG (offset at 84,
            // length at 88, big-endian), whose EXIF is the camera's.
            let at = u32::from_be_bytes(data.get(84..88)?.try_into().ok()?) as usize;
            let len = u32::from_be_bytes(data.get(88..92)?.try_into().ok()?) as usize;
            let jpeg = data.get(at..at.checked_add(len)?.min(data.len()))?;
            from_exif(crate::jpeg_app1_exif(jpeg)?)?
        }
        FileFormat::Cr3 => {
            // CMT1 is IFD0, CMT2 the Exif IFD and CMT4 the GPS IFD, each a
            // TIFF structure of its own.
            let first = |kind: &[u8; 4]| -> Vec<Tag> {
                crate::cr3_box(data, kind)
                    .and_then(|b| {
                        let t = Tiff::new(b)?;
                        let ifd = t.ifd(t.u32(4)? as usize).ok()?;
                        Some(tags(&t, &ifd))
                    })
                    .unwrap_or_default()
            };
            Metadata {
                ifd0: first(b"CMT1"),
                exif: first(b"CMT2"),
                gps: first(b"CMT4"),
            }
        }
    };
    let m = Metadata {
        ifd0: m.ifd0.into_iter().filter(kept_in_ifd0).collect(),
        exif: m
            .exif
            .into_iter()
            .filter(|t| !matches!(t.tag, INTEROP_IFD | PIXEL_X_DIMENSION | PIXEL_Y_DIMENSION))
            .collect(),
        gps: m.gps,
    };
    (!m.is_empty()).then_some(m)
}

/// The EXIF in an EXIF block (a JPEG's APP1 payload, with or without
/// `Exif\0\0`, or a PNG's eXIf): for the fuzz target and the round trip.
pub fn from_exif(block: &[u8]) -> Option<Metadata> {
    from_tiff(exif::tiff_payload(block))
}

/// IFD0 of the TIFF structure `data`, its Exif IFD and its GPS IFD. All of
/// IFD0's tags are returned; [`read`] keeps the descriptive ones.
fn from_tiff(data: &[u8]) -> Option<Metadata> {
    let t = Tiff::new(data)?;
    let ifd0 = t.ifd(t.u32(4)? as usize).ok()?;
    let sub = |tag: u16| -> Option<Vec<Entry>> {
        let at = *t.ints(find(&ifd0, tag)?).ok()?.first()? as usize;
        t.ifd(at).ok()
    };
    let exif = match sub(EXIF_IFD) {
        Some(ifd) => tags(&t, &ifd),
        // TIFF/EP: the Exif tags sit in IFD0 itself.
        None if find(&ifd0, EXIF_IFD).is_none() => tags(&t, &ifd0)
            .into_iter()
            .filter(|tag| is_exif_tag(tag.tag))
            .collect(),
        None => Vec::new(),
    };
    Some(Metadata {
        ifd0: tags(&t, &ifd0),
        exif,
        gps: sub(GPS_IFD).map(|ifd| tags(&t, &ifd)).unwrap_or_default(),
    })
}

/// Tags of the Exif IFD's range that TIFF/EP files keep in IFD0.
fn is_exif_tag(tag: u16) -> bool {
    matches!(tag, 0x829A..=0x829D | 0x8822..=0x8832 | 0x9000..=0x9FFF | 0xA000..=0xA4FF)
        && !matches!(tag, EXIF_IFD | GPS_IFD | INTEROP_IFD)
}

/// The entries of `ifd` as tags, little endian; IFD pointers and entries
/// of an unknown type, or whose values lie outside the file, are left out.
fn tags(t: &Tiff, ifd: &[Entry]) -> Vec<Tag> {
    let mut out: Vec<Tag> = ifd
        .iter()
        .filter(|e| !matches!(e.tag, EXIF_IFD | GPS_IFD | INTEROP_IFD | 330))
        .filter_map(|&e| tag(t, e))
        .collect();
    out.sort_by_key(|t| t.tag);
    out.dedup_by_key(|t| t.tag);
    out
}

fn tag(t: &Tiff, e: Entry) -> Option<Tag> {
    // The size of one element, and of the parts swapped for byte order.
    let (size, part) = match e.ty {
        1 | 2 | 6 | 7 => (1, 1),
        3 | 8 => (2, 2),
        4 | 9 | 11 => (4, 4),
        5 | 10 => (8, 4),
        12 => (8, 8),
        _ => return None,
    };
    let (base, total) = t.location(e).ok()?;
    let bytes: &[u8] = if base == usize::MAX {
        e.value.get(..total)?
    } else {
        t.data.get(base..base.checked_add(total)?)?
    };
    debug_assert_eq!(total, size * e.count as usize);
    let value = if t.big_endian && part > 1 {
        bytes
            .chunks_exact(part)
            .flat_map(|c| c.iter().rev().copied())
            .collect()
    } else {
        bytes.to_vec()
    };
    Some(Tag {
        tag: e.tag,
        ty: e.ty,
        count: e.count,
        value,
    })
}

/// A LONG tag of one value.
fn int(tag: u16, v: u32) -> Tag {
    Tag {
        tag,
        ty: 4,
        count: 1,
        value: v.to_le_bytes().to_vec(),
    }
}

/// A SHORT tag of one value.
fn short(tag: u16, v: u16) -> Tag {
    Tag {
        tag,
        ty: 3,
        count: 1,
        value: v.to_le_bytes().to_vec(),
    }
}

/// An UNDEFINED tag.
fn undefined(tag: u16, bytes: &[u8]) -> Tag {
    Tag {
        tag,
        ty: 7,
        count: bytes.len() as u32,
        value: bytes.to_vec(),
    }
}

/// An ASCII tag, NUL-terminated.
fn ascii(tag: u16, text: &[u8]) -> Tag {
    let mut value: Vec<u8> = text.iter().copied().take_while(|&b| b != 0).collect();
    value.push(0);
    Tag {
        tag,
        ty: 2,
        count: value.len() as u32,
        value,
    }
}

/// `meta` as an exported JPEG's APP1 payload, for pixels of `size` written
/// by `software`. A payload over [`APP1_MAX`] drops the MakerNote, then the
/// largest tag stored outside its entry, of any IFD, until it fits: with
/// only values of 4 bytes or less left, three IFDs of at most 1024
/// entries take under 37 KB.
pub fn write(meta: &Metadata, size: (u32, u32), software: &str) -> Written {
    let mut m = meta.clone();
    m.exif
        .retain(|t| !matches!(t.tag, PIXEL_X_DIMENSION | PIXEL_Y_DIMENSION | INTEROP_IFD));
    m.exif.push(int(PIXEL_X_DIMENSION, size.0));
    m.exif.push(int(PIXEL_Y_DIMENSION, size.1));
    // The export is sRGB, whatever the source said (an Adobe RGB camera
    // JPEG says 0xFFFF).
    m.exif.retain(|t| t.tag != COLOR_SPACE);
    m.exif.push(short(COLOR_SPACE, 1));
    let missing = |ifd: &[Tag], tag: u16| ifd.iter().all(|t| t.tag != tag);
    if missing(&m.ifd0, Y_CB_CR_POSITIONING) {
        m.ifd0.push(short(Y_CB_CR_POSITIONING, 1));
    }
    // The structure written is EXIF 2.32's, whatever version the source
    // claimed: one claiming an older version can hold newer tags
    // (OffsetTimeOriginal is 2.31's).
    m.exif.retain(|t| t.tag != EXIF_VERSION);
    m.exif.push(undefined(EXIF_VERSION, b"0232"));
    if missing(&m.exif, COMPONENTS_CONFIGURATION) {
        m.exif
            .push(undefined(COMPONENTS_CONFIGURATION, &[1, 2, 3, 0]));
    }
    if !m.gps.is_empty() {
        m.gps.retain(|t| t.tag != GPS_VERSION_ID);
        m.gps.push(Tag {
            tag: GPS_VERSION_ID,
            ty: 1,
            count: 4,
            value: vec![2, 3, 0, 0],
        });
    }
    let mut dropped = Vec::new();
    loop {
        let app1 = serialize(&m, software);
        if app1.len() <= APP1_MAX {
            return Written { app1, dropped };
        }
        if let Some(i) = m.exif.iter().position(|t| t.tag == MAKER_NOTE) {
            dropped.push(m.exif.remove(i).tag);
            continue;
        }
        let largest = [&m.ifd0, &m.exif, &m.gps]
            .iter()
            .enumerate()
            .flat_map(|(k, ifd)| {
                ifd.iter()
                    .enumerate()
                    .map(move |(i, t)| (k, i, t.value.len()))
            })
            .filter(|&(_, _, len)| len > 4)
            .max_by_key(|&(_, _, len)| len);
        let Some((k, i, _)) = largest else {
            // Unreachable by the bound above; never loop forever.
            return Written { app1, dropped };
        };
        let ifd = match k {
            0 => &mut m.ifd0,
            1 => &mut m.exif,
            _ => &mut m.gps,
        };
        dropped.push(ifd.remove(i).tag);
    }
}

fn serialize(meta: &Metadata, software: &str) -> Vec<u8> {
    let exif = &meta.exif;
    let mut ifd0: Vec<Tag> = meta
        .ifd0
        .iter()
        .filter(|t| !matches!(t.tag, ORIENTATION | SOFTWARE | PROCESSING_SOFTWARE))
        .filter(|t| !matches!((t.tag, t.ty), (EXIF_IFD | GPS_IFD, _)))
        .cloned()
        .collect();
    if let Some(source) = meta.ifd0.iter().find(|t| t.tag == SOFTWARE && t.ty == 2) {
        ifd0.push(ascii(PROCESSING_SOFTWARE, &source.value));
    }
    ifd0.push(short(ORIENTATION, 1));
    ifd0.push(ascii(SOFTWARE, software.as_bytes()));
    let has_exif = !exif.is_empty();
    let has_gps = !meta.gps.is_empty();
    // Pointers, filled in once the layout is known.
    if has_exif {
        ifd0.push(int(EXIF_IFD, 0));
    }
    if has_gps {
        ifd0.push(int(GPS_IFD, 0));
    }
    ifd0.sort_by_key(|t| t.tag);
    let mut exif = exif.to_vec();
    exif.sort_by_key(|t| t.tag);
    let mut gps = meta.gps.clone();
    gps.sort_by_key(|t| t.tag);

    let ifd0_at = 8;
    let exif_at = ifd0_at + ifd_len(&ifd0);
    let gps_at = exif_at + if has_exif { ifd_len(&exif) } else { 0 };
    for t in &mut ifd0 {
        match t.tag {
            EXIF_IFD => t.value = (exif_at as u32).to_le_bytes().to_vec(),
            GPS_IFD => t.value = (gps_at as u32).to_le_bytes().to_vec(),
            _ => {}
        }
    }
    let mut tiff = b"II*\0\x08\0\0\0".to_vec();
    write_ifd(&mut tiff, &ifd0);
    if has_exif {
        write_ifd(&mut tiff, &exif);
    }
    if has_gps {
        write_ifd(&mut tiff, &gps);
    }
    let mut out = b"Exif\0\0".to_vec();
    out.extend_from_slice(&tiff);
    out
}

/// The bytes an IFD and its values take, values padded to even offsets.
fn ifd_len(tags: &[Tag]) -> usize {
    2 + 12 * tags.len()
        + 4
        + tags
            .iter()
            .map(|t| outside(t).div_ceil(2) * 2)
            .sum::<usize>()
}

/// The bytes of `t`'s values stored after the IFD: none when they fit in
/// the entry's 4 bytes.
fn outside(t: &Tag) -> usize {
    if t.value.len() > 4 { t.value.len() } else { 0 }
}

/// Appends an IFD whose entries are followed by their values; its next
/// IFD is none.
fn write_ifd(out: &mut Vec<u8>, tags: &[Tag]) {
    let start = out.len();
    let mut data_at = start + 2 + 12 * tags.len() + 4;
    let mut data = Vec::new();
    out.extend_from_slice(&(tags.len() as u16).to_le_bytes());
    for t in tags {
        out.extend_from_slice(&t.tag.to_le_bytes());
        out.extend_from_slice(&t.ty.to_le_bytes());
        out.extend_from_slice(&t.count.to_le_bytes());
        if t.value.len() > 4 {
            out.extend_from_slice(&(data_at as u32).to_le_bytes());
            data.extend_from_slice(&t.value);
            if t.value.len() % 2 == 1 {
                data.push(0);
            }
            data_at += t.value.len().div_ceil(2) * 2;
        } else {
            let mut inline = [0u8; 4];
            inline[..t.value.len()].copy_from_slice(&t.value);
            out.extend_from_slice(&inline);
        }
    }
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&data);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An entry as a test writes it: tag, type, count, values.
    type Raw = (u16, u16, u32, Vec<u8>);

    /// A TIFF structure of IFDs (IFD0 first), in either byte order. Each
    /// entry is (tag, type, count, values as written in that order).
    /// Pointer values are given as IFD indices and patched.
    struct Builder {
        big: bool,
    }

    impl Builder {
        fn u16(&self, v: u16) -> [u8; 2] {
            if self.big {
                v.to_be_bytes()
            } else {
                v.to_le_bytes()
            }
        }
        fn u32(&self, v: u32) -> [u8; 4] {
            if self.big {
                v.to_be_bytes()
            } else {
                v.to_le_bytes()
            }
        }

        fn build(&self, ifds: &[Vec<Raw>], pointers: &[(usize, u16, usize)]) -> Vec<u8> {
            let mut out = if self.big {
                b"MM\0*".to_vec()
            } else {
                b"II*\0".to_vec()
            };
            out.extend(self.u32(8));
            // Layout: each IFD, then its values.
            let mut at = Vec::new();
            let mut pos = 8;
            for ifd in ifds {
                at.push(pos);
                pos += 2 + 12 * ifd.len() + 4;
                pos += ifd
                    .iter()
                    .map(|e| {
                        if e.3.len() > 4 {
                            e.3.len().div_ceil(2) * 2
                        } else {
                            0
                        }
                    })
                    .sum::<usize>();
            }
            for (i, ifd) in ifds.iter().enumerate() {
                let mut data_at = at[i] + 2 + 12 * ifd.len() + 4;
                let mut data = Vec::new();
                out.extend(self.u16(ifd.len() as u16));
                for (tag, ty, count, value) in ifd {
                    out.extend(self.u16(*tag));
                    out.extend(self.u16(*ty));
                    out.extend(self.u32(*count));
                    let pointer = pointers.iter().find(|p| p.0 == i && p.1 == *tag);
                    if let Some(p) = pointer {
                        out.extend(self.u32(at[p.2] as u32));
                    } else if value.len() > 4 {
                        out.extend(self.u32(data_at as u32));
                        data.extend(value);
                        if value.len() % 2 == 1 {
                            data.push(0);
                        }
                        data_at += value.len().div_ceil(2) * 2;
                    } else {
                        let mut v = value.clone();
                        v.resize(4, 0);
                        out.extend(v);
                    }
                }
                out.extend(self.u32(0));
                out.extend(data);
            }
            out
        }

        fn short(&self, v: u16) -> Vec<u8> {
            self.u16(v).to_vec()
        }
        fn long(&self, v: u32) -> Vec<u8> {
            self.u32(v).to_vec()
        }
        fn rationals(&self, vs: &[(u32, u32)]) -> Vec<u8> {
            vs.iter()
                .flat_map(|&(n, d)| [self.u32(n), self.u32(d)])
                .flatten()
                .collect()
        }
    }

    /// A RAW-like IFD0 with structure, an Exif IFD and a GPS IFD.
    fn sample(big: bool, maker_note: usize) -> Vec<u8> {
        let b = Builder { big };
        let ifd0 = vec![
            (256, 4, 1, b.long(6000)),             // ImageWidth: structure
            (271, 2, 6, b"Nikon\0".to_vec()),      // Make
            (272, 2, 10, b"NIKON Z 8\0".to_vec()), // Model
            (273, 4, 1, b.long(123_456)),          // StripOffsets: structure
            (ORIENTATION, 3, 1, b.short(6)),
            (SOFTWARE, 2, 9, b"Ver.1.00\0".to_vec()),
            (EXIF_IFD, 4, 1, b.long(0)),
            (GPS_IFD, 4, 1, b.long(0)),
            (33432, 2, 6, b"Gabri\0".to_vec()), // Copyright
            (50706, 1, 4, vec![1, 4, 0, 0]),    // DNGVersion: structure
        ];
        let mut exif = vec![
            (33434, 5, 1, b.rationals(&[(1, 250)])), // ExposureTime
            (34855, 3, 1, b.short(400)),             // ISO
            (0x9003, 2, 20, b"2026:10:12 17:02:04\0".to_vec()),
            (0x9291, 2, 3, b"25\0".to_vec()),
            (40962, 4, 1, b.long(6000)),
            (40963, 4, 1, b.long(4000)),
            (INTEROP_IFD, 4, 1, b.long(0)),
        ];
        if maker_note > 0 {
            exif.push((MAKER_NOTE, 7, maker_note as u32, vec![0xAB; maker_note]));
        }
        exif.sort_by_key(|e| e.0);
        let gps = vec![
            (1, 2, 2, b"S\0".to_vec()),                               // LatitudeRef
            (2, 5, 3, b.rationals(&[(22, 1), (54, 1), (1234, 100)])), // Latitude
            (3, 2, 2, b"W\0".to_vec()),
            (4, 5, 3, b.rationals(&[(47, 1), (3, 1), (3000, 100)])),
        ];
        b.build(&[ifd0, exif, gps], &[(0, EXIF_IFD, 1), (0, GPS_IFD, 2)])
    }

    fn written_tiff(w: &Written) -> Metadata {
        assert!(w.app1.starts_with(b"Exif\0\0"));
        // All of IFD0, as written.
        from_exif(&w.app1).expect("the written block reads back")
    }

    #[test]
    fn keeps_the_description_drops_the_structure_in_both_byte_orders() {
        for big in [false, true] {
            let m = from_tiff(&sample(big, 40)).unwrap();
            // IFD pointers are never read as tags: the writer sets its own.
            let pointer = |t: &Tag| matches!(t.tag, EXIF_IFD | GPS_IFD | INTEROP_IFD);
            assert!(!m.ifd0.iter().chain(&m.exif).any(pointer));
            let m = Metadata {
                ifd0: m.ifd0.into_iter().filter(kept_in_ifd0).collect(),
                exif: m
                    .exif
                    .into_iter()
                    .filter(|t| {
                        !matches!(t.tag, INTEROP_IFD | PIXEL_X_DIMENSION | PIXEL_Y_DIMENSION)
                    })
                    .collect(),
                gps: m.gps,
            };
            let ifd0: Vec<u16> = m.ifd0.iter().map(|t| t.tag).collect();
            assert_eq!(ifd0, [271, 272, SOFTWARE, 33432], "big endian {big}");
            // Rationals come out little endian, numerator then denominator.
            let exposure = m.exif_tag(33434).unwrap();
            assert_eq!(exposure.value, [1, 0, 0, 0, 250, 0, 0, 0]);
            assert_eq!(m.exif_tag(34855).unwrap().value, [144, 1]);
            assert_eq!(m.gps.len(), 4);

            let w = write(&m, (2048, 1365), "Genzo 0.1.0");
            assert!(w.dropped.is_empty());
            let back = written_tiff(&w);
            let tag = |ifd: &[Tag], n: u16| ifd.iter().find(|t| t.tag == n).cloned();
            assert_eq!(tag(&back.ifd0, ORIENTATION).unwrap().value, [1, 0]);
            assert_eq!(tag(&back.ifd0, SOFTWARE).unwrap().value, b"Genzo 0.1.0\0");
            assert_eq!(
                tag(&back.ifd0, PROCESSING_SOFTWARE).unwrap().value,
                b"Ver.1.00\0"
            );
            assert_eq!(tag(&back.ifd0, 271).unwrap().value, b"Nikon\0");
            assert!(tag(&back.ifd0, 273).is_none() && tag(&back.ifd0, 50706).is_none());
            assert_eq!(
                tag(&back.exif, PIXEL_X_DIMENSION).unwrap().value,
                2048u32.to_le_bytes()
            );
            assert_eq!(
                tag(&back.exif, PIXEL_Y_DIMENSION).unwrap().value,
                1365u32.to_le_bytes()
            );
            assert!(tag(&back.exif, INTEROP_IFD).is_none());
            assert_eq!(tag(&back.exif, MAKER_NOTE).unwrap().value, vec![0xAB; 40]);
            assert_eq!(tag(&back.exif, 0x9003), tag(&m.exif, 0x9003));
            assert_eq!(
                &back.gps[1..],
                &m.gps[..],
                "the source's GPS, after the version added"
            );
            assert_eq!(back.gps[0].value, [2, 3, 0, 0]);
            assert_eq!(tag(&back.exif, COLOR_SPACE).unwrap().value, [1, 0]);
            assert_eq!(tag(&back.exif, EXIF_VERSION).unwrap().value, b"0232");
            assert_eq!(
                tag(&back.exif, COMPONENTS_CONFIGURATION).unwrap().value,
                [1, 2, 3, 0]
            );
            assert_eq!(tag(&back.ifd0, Y_CB_CR_POSITIONING).unwrap().value, [1, 0]);
            // The capture reader sees the same capture.
            let jpeg_like = crate::capture::from_exif_block(&w.app1);
            assert_eq!(jpeg_like.shooting.iso, Some(400));
            assert!(jpeg_like.time.is_some());
        }
    }

    #[test]
    fn a_maker_note_too_large_for_app1_is_dropped_and_said() {
        let m = from_tiff(&sample(false, 70_000)).unwrap();
        let w = write(&m, (100, 100), "Genzo");
        assert!(w.app1.len() <= APP1_MAX);
        assert_eq!(w.dropped, [MAKER_NOTE]);
        let back = written_tiff(&w);
        assert!(back.exif.iter().all(|t| t.tag != MAKER_NOTE));
        assert_eq!(back.gps.len(), 5);
        // The MakerNote goes first, even beside a larger tag: a UserComment
        // of 50 KB stays once the 20 KB MakerNote is gone.
        let b = Builder { big: false };
        let ifd0 = vec![(EXIF_IFD, 4, 1, b.long(0))];
        let exif = vec![
            (37500, 7, 20_000, vec![1; 20_000]),
            (37510, 7, 50_000, vec![2; 50_000]),
        ];
        let m = from_tiff(&b.build(&[ifd0, exif], &[(0, EXIF_IFD, 1)])).unwrap();
        let w = write(&m, (1, 1), "Genzo");
        assert_eq!(w.dropped, [MAKER_NOTE]);
        assert!(from_exif(&w.app1).unwrap().exif_tag(37510).is_some());
        // One that fits stays.
        let m = from_tiff(&sample(false, 60_000)).unwrap();
        let w = write(&m, (100, 100), "Genzo");
        assert!(w.dropped.is_empty() && w.app1.len() <= APP1_MAX);
    }

    #[test]
    fn without_an_exif_ifd_the_exif_tags_of_ifd0_are_read() {
        let b = Builder { big: false };
        let ifd0 = vec![
            (271, 2, 6, b"Kodak\0".to_vec()),
            (273, 4, 1, b.long(8)),
            (33434, 5, 1, b.rationals(&[(1, 60)])),
            (0x9003, 2, 20, b"2002:01:01 10:00:00\0".to_vec()),
        ];
        let m = from_tiff(&b.build(&[ifd0], &[])).unwrap();
        let exif: Vec<u16> = m.exif.iter().map(|t| t.tag).collect();
        assert_eq!(exif, [33434, 0x9003]);
    }

    /// Panasonic's RW2 has tags of its own under YResolution's number in
    /// IFD0; a tag of the wrong type is not the standard one. And a source
    /// claiming an old Exif version gets the version of what is written.
    #[test]
    fn ifd0_tags_of_another_type_are_not_kept_and_versions_are_written_anew() {
        let b = Builder { big: false };
        let ifd0 = vec![
            (271, 2, 10, b"Panasonic\0".to_vec()),
            (272, 3, 1, b.short(7)),
            (283, 7, 8, vec![9; 8]),
            (EXIF_IFD, 4, 1, b.long(0)),
            (GPS_IFD, 4, 1, b.long(0)),
        ];
        let exif = vec![(EXIF_VERSION, 7, 4, b"0210".to_vec())];
        let gps = vec![
            (GPS_VERSION_ID, 1, 4, vec![0, 0, 0, 0]),
            (1, 2, 2, b"N\0".to_vec()),
        ];
        let data = b.build(&[ifd0, exif, gps], &[(0, EXIF_IFD, 1), (0, GPS_IFD, 2)]);
        let m = from_tiff(&data).unwrap();
        let kept: Vec<u16> = m
            .ifd0
            .iter()
            .filter(|t| kept_in_ifd0(t))
            .map(|t| t.tag)
            .collect();
        assert_eq!(kept, [271]);
        let back = from_exif(&write(&m, (1, 1), "Genzo").app1).unwrap();
        let tag = |ifd: &[Tag], n: u16| ifd.iter().find(|t| t.tag == n).map(|t| t.value.clone());
        assert_eq!(tag(&back.exif, EXIF_VERSION).unwrap(), b"0232");
        assert_eq!(tag(&back.gps, GPS_VERSION_ID).unwrap(), [2, 3, 0, 0]);
    }

    #[test]
    fn nothing_in_random_bytes_and_no_panic() {
        let mut seed = 0x1234_5678u32;
        for len in [0usize, 8, 64, 512] {
            for _ in 0..200 {
                let mut data = b"II*\0\x08\0\0\0".to_vec();
                for _ in 0..len {
                    seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    data.push((seed >> 24) as u8);
                }
                if let Some(m) = from_exif(&data) {
                    let w = write(&m, (1, 1), "Genzo");
                    assert!(w.app1.len() <= APP1_MAX);
                    assert!(from_exif(&w.app1).is_some());
                }
            }
        }
    }
}
