//! A bounds-checked reader of TIFF structure (header, IFDs, entry values),
//! shared by the DNG reader, EXIF orientation and the TIFF routing. The
//! input is untrusted: every offset and count is checked, counts are capped
//! before anything is allocated, and nothing here panics on bad data.

/// Most entries read from one IFD, and most values from one entry.
const MAX_IFD_ENTRIES: usize = 1024;
const MAX_VALUES: usize = 1 << 20;

/// DNGVersion, in IFD0 of every DNG.
pub(crate) const DNG_VERSION: u16 = 50706;

/// Whether IFD0 of a TIFF has `tag`. False for anything unreadable.
pub(crate) fn ifd0_has_tag(data: &[u8], tag: u16) -> bool {
    (|| {
        let t = Tiff::new(data)?;
        let ifd0 = t.ifd(t.u32(4)? as usize).ok()?;
        Some(find(&ifd0, tag).is_some())
    })()
    .unwrap_or(false)
}

/// What was wrong with the structure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TiffError(pub String);

fn corrupt(message: impl Into<String>) -> TiffError {
    TiffError(message.into())
}

/// One IFD entry: its type, count and the 4 bytes that hold the value or
/// its offset.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Entry {
    pub(crate) tag: u16,
    pub(crate) ty: u16,
    pub(crate) count: u32,
    pub(crate) value: [u8; 4],
}

pub(crate) fn find(ifd: &[Entry], tag: u16) -> Option<Entry> {
    ifd.iter().find(|e| e.tag == tag).copied()
}

/// A bounds-checked view of a TIFF file.
pub(crate) struct Tiff<'a> {
    pub(crate) data: &'a [u8],
    pub(crate) big_endian: bool,
}

impl<'a> Tiff<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Option<Self> {
        let big_endian = match data.get(..4)? {
            b"II*\0" => false,
            b"MM\0*" => true,
            _ => return None,
        };
        Some(Self { data, big_endian })
    }

    pub(crate) fn bytes<const N: usize>(&self, at: usize) -> Option<[u8; N]> {
        self.data.get(at..at.checked_add(N)?)?.try_into().ok()
    }

    pub(crate) fn u16(&self, at: usize) -> Option<u16> {
        let b = self.bytes::<2>(at)?;
        Some(if self.big_endian {
            u16::from_be_bytes(b)
        } else {
            u16::from_le_bytes(b)
        })
    }

    pub(crate) fn u32(&self, at: usize) -> Option<u32> {
        let b = self.bytes::<4>(at)?;
        Some(if self.big_endian {
            u32::from_be_bytes(b)
        } else {
            u32::from_le_bytes(b)
        })
    }

    pub(crate) fn ifd(&self, at: usize) -> Result<Vec<Entry>, TiffError> {
        let bad = || corrupt(format!("IFD at {at} lies outside the file"));
        let n = usize::from(self.u16(at).ok_or_else(bad)?);
        if n > MAX_IFD_ENTRIES {
            return Err(corrupt(format!("IFD at {at} has {n} entries")));
        }
        (0..n)
            .map(|i| {
                let e = at + 2 + i * 12;
                Ok(Entry {
                    tag: self.u16(e).ok_or_else(bad)?,
                    ty: self.u16(e + 2).ok_or_else(bad)?,
                    count: self.u32(e + 4).ok_or_else(bad)?,
                    value: self.bytes::<4>(e + 8).ok_or_else(bad)?,
                })
            })
            .collect()
    }

    /// An entry's values as raw bytes, for BYTE, ASCII and UNDEFINED tags
    /// (opcode lists, the gain table map, EXIF strings). Bounded by the file's size, not by `MAX_VALUES`.
    pub(crate) fn blob(&self, e: Entry) -> Result<Vec<u8>, TiffError> {
        if !matches!(e.ty, 1 | 2 | 7) {
            return Err(corrupt(format!("tag {} is not bytes", e.tag)));
        }
        let count = e.count as usize;
        if count <= 4 {
            return Ok(e.value[..count].to_vec());
        }
        let offset = if self.big_endian {
            u32::from_be_bytes(e.value)
        } else {
            u32::from_le_bytes(e.value)
        } as usize;
        offset
            .checked_add(count)
            .and_then(|end| self.data.get(offset..end))
            .map(<[u8]>::to_vec)
            .ok_or_else(|| corrupt(format!("tag {} lies outside the file", e.tag)))
    }

    /// Where an entry's values start, and their total size in bytes.
    pub(crate) fn location(&self, e: Entry) -> Result<(usize, usize), TiffError> {
        let size = match e.ty {
            1 | 2 | 6 | 7 => 1,
            3 | 8 => 2,
            4 | 9 | 11 => 4,
            5 | 10 | 12 => 8,
            other => return Err(corrupt(format!("tag {} has type {other}", e.tag))),
        };
        let count = e.count as usize;
        if count > MAX_VALUES {
            return Err(corrupt(format!("tag {} has {count} values", e.tag)));
        }
        let total = size * count;
        if total <= 4 {
            return Ok((usize::MAX, total));
        }
        let offset = if self.big_endian {
            u32::from_be_bytes(e.value)
        } else {
            u32::from_le_bytes(e.value)
        } as usize;
        match offset.checked_add(total) {
            Some(end) if end <= self.data.len() => Ok((offset, total)),
            _ => Err(corrupt(format!("tag {} lies outside the file", e.tag))),
        }
    }

    /// Reads value `i` of an entry as raw bytes of `N`, inline or not.
    pub(crate) fn raw<const N: usize>(&self, e: Entry, base: usize, i: usize) -> Option<[u8; N]> {
        if base == usize::MAX {
            e.value.get(i * N..i * N + N)?.try_into().ok()
        } else {
            self.bytes::<N>(base + i * N)
        }
    }

    pub(crate) fn uint<const N: usize>(&self, b: [u8; N]) -> u64 {
        let mut v = 0u64;
        for i in 0..N {
            let byte = if self.big_endian { b[i] } else { b[N - 1 - i] };
            v = (v << 8) | u64::from(byte);
        }
        v
    }

    /// All values of an unsigned integer entry (BYTE, SHORT or LONG).
    pub(crate) fn ints(&self, e: Entry) -> Result<Vec<u32>, TiffError> {
        let (base, _) = self.location(e)?;
        let bad = || corrupt(format!("tag {}", e.tag));
        (0..e.count as usize)
            .map(|i| {
                Ok(match e.ty {
                    1 | 7 => self.uint(self.raw::<1>(e, base, i).ok_or_else(bad)?) as u32,
                    3 => self.uint(self.raw::<2>(e, base, i).ok_or_else(bad)?) as u32,
                    4 => self.uint(self.raw::<4>(e, base, i).ok_or_else(bad)?) as u32,
                    _ => return Err(corrupt(format!("tag {} is not an integer", e.tag))),
                })
            })
            .collect()
    }

    /// All values of a numeric entry, rationals included, as f64.
    // Only the DNG reader reads these.
    #[cfg_attr(not(feature = "dng"), allow(dead_code))]
    pub(crate) fn numbers(&self, e: Entry) -> Result<Vec<f64>, TiffError> {
        let (base, _) = self.location(e)?;
        let bad = || corrupt(format!("tag {}", e.tag));
        (0..e.count as usize)
            .map(|i| {
                Ok(match e.ty {
                    1 | 7 => self.uint(self.raw::<1>(e, base, i).ok_or_else(bad)?) as f64,
                    3 => self.uint(self.raw::<2>(e, base, i).ok_or_else(bad)?) as f64,
                    4 => self.uint(self.raw::<4>(e, base, i).ok_or_else(bad)?) as f64,
                    8 => f64::from(
                        self.uint(self.raw::<2>(e, base, i).ok_or_else(bad)?) as u16 as i16
                    ),
                    9 => f64::from(
                        self.uint(self.raw::<4>(e, base, i).ok_or_else(bad)?) as u32 as i32
                    ),
                    11 => f64::from(f32::from_bits(
                        self.uint(self.raw::<4>(e, base, i).ok_or_else(bad)?) as u32,
                    )),
                    12 => f64::from_bits(self.uint(self.raw::<8>(e, base, i).ok_or_else(bad)?)),
                    5 | 10 => {
                        let b = self.raw::<8>(e, base, i).ok_or_else(bad)?;
                        let num = self.uint::<4>([b[0], b[1], b[2], b[3]]) as u32;
                        let den = self.uint::<4>([b[4], b[5], b[6], b[7]]) as u32;
                        let (num, den) = if e.ty == 10 {
                            (f64::from(num as i32), f64::from(den as i32))
                        } else {
                            (f64::from(num), f64::from(den))
                        };
                        if den == 0.0 { 0.0 } else { num / den }
                    }
                    _ => return Err(corrupt(format!("tag {} is not a number", e.tag))),
                })
            })
            .collect()
    }

    pub(crate) fn first_int(&self, ifd: &[Entry], tag: u16) -> Option<u32> {
        self.ints(find(ifd, tag)?).ok()?.first().copied()
    }

    // Only the DNG reader reads these.
    #[cfg_attr(not(feature = "dng"), allow(dead_code))]
    pub(crate) fn required_int(&self, ifd: &[Entry], tag: u16) -> Result<u32, TiffError> {
        self.first_int(ifd, tag)
            .ok_or_else(|| corrupt(format!("missing or unreadable tag {tag}")))
    }
}
