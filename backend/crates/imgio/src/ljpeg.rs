//! Lossless JPEG (ITU T.81, process 14) streams of DNG tiles and strips,
//! through rawler's decompressor.
//!
//! rawler 0.8 ignores restart intervals (DRI): past the first restart it
//! returns wrong samples, with no error. Samsung writes its DNG strips with a
//! restart every 16 rows. A restart resets prediction as the start of a scan
//! does (T.81, H.1.2.1), so a stream with restarts is split at its RST markers
//! and each interval is decoded as a stream of its own, in parallel. Only
//! the layout Samsung uses is accepted (every component sampled 1 × 1, an
//! interval of whole rows); any other is reported unsupported, never decoded
//! wrongly.

use rayon::prelude::*;

/// A decoded stream: `width` samples per row (pixels × components).
pub(crate) struct Samples {
    pub width: usize,
    pub height: usize,
    pub data: Vec<u16>,
}

/// Why a stream did not decode: unsupported, or damaged.
#[derive(Debug)]
pub(crate) enum Error {
    Unsupported(String),
    Corrupt(String),
}

/// The parts of a stream's header the split needs, as byte ranges.
struct Header {
    /// SOF3 segment, and its height, width and components.
    sof: (usize, usize),
    height: usize,
    width: usize,
    components: usize,
    one_by_one: bool,
    /// DHT segments.
    tables: Vec<(usize, usize)>,
    /// Restart interval in MCUs; 0 for none.
    restart: usize,
    /// SOS segment; the entropy data start at its end.
    sos: (usize, usize),
}

fn corrupt(m: impl Into<String>) -> Error {
    Error::Corrupt(m.into())
}

fn header(b: &[u8]) -> Result<Header, Error> {
    if !b.starts_with(&[0xFF, 0xD8]) {
        return Err(corrupt("no SOI marker"));
    }
    let u16_at = |at: usize| -> Result<usize, Error> {
        b.get(at..at + 2)
            .map(|v| usize::from(u16::from_be_bytes([v[0], v[1]])))
            .ok_or_else(|| corrupt("truncated header"))
    };
    let (mut sof, mut tables, mut restart) = (None, Vec::new(), 0);
    let mut at = 2;
    loop {
        // Markers may be preceded by any number of 0xFF fill bytes.
        while b.get(at) == Some(&0xFF) && b.get(at + 1) == Some(&0xFF) {
            at += 1;
        }
        if b.get(at) != Some(&0xFF) {
            return Err(corrupt("expected a marker"));
        }
        let marker = *b.get(at + 1).ok_or_else(|| corrupt("truncated header"))?;
        let len = u16_at(at + 2)?;
        let end = at + 2 + len;
        if len < 2 || end > b.len() {
            return Err(corrupt("a header segment lies outside the stream"));
        }
        let segment = (at, end);
        match marker {
            0xC3 => {
                let n = *b.get(at + 9).ok_or_else(|| corrupt("short SOF3"))? as usize;
                if len < 8 + 3 * n {
                    return Err(corrupt("short SOF3"));
                }
                let one_by_one = (0..n).all(|i| b[at + 11 + 3 * i] == 0x11);
                sof = Some((segment, u16_at(at + 5)?, u16_at(at + 7)?, n, one_by_one));
            }
            0xC4 => tables.push(segment),
            0xDD => restart = u16_at(at + 4)?,
            0xDA => {
                let (sof, height, width, components, one_by_one) =
                    sof.ok_or_else(|| corrupt("SOS before SOF3"))?;
                return Ok(Header {
                    sof,
                    height,
                    width,
                    components,
                    one_by_one,
                    tables,
                    restart,
                    sos: segment,
                });
            }
            0xC0..=0xC2 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF => {
                return Err(Error::Unsupported(format!(
                    "JPEG process SOF{} instead of lossless (SOF3)",
                    marker - 0xC0
                )));
            }
            _ => {}
        }
        at = end;
    }
}

/// Positions of the RST markers in the entropy data, which start at `from`,
/// and where the data end (at EOI, or the end of the stream).
fn restart_markers(b: &[u8], from: usize) -> Result<(Vec<usize>, usize), Error> {
    let mut markers = Vec::new();
    let mut i = from;
    while i + 1 < b.len() {
        if b[i] != 0xFF {
            i += 1;
            continue;
        }
        match b[i + 1] {
            // A stuffed 0xFF data byte, or fill before a marker.
            0x00 | 0xFF => i += 1,
            0xD0..=0xD7 => {
                if b[i + 1] != 0xD0 + (markers.len() % 8) as u8 {
                    return Err(corrupt("restart markers out of order"));
                }
                markers.push(i);
                i += 2;
            }
            0xD9 => return Ok((markers, i)),
            other => return Err(corrupt(format!("marker 0x{other:02X} in the entropy data"))),
        }
    }
    Ok((markers, b.len()))
}

/// Decodes one stream with no restarts. rawler is not hardened against bad
/// input the way this crate is: a panic in it is turned into an error.
fn decode_one(b: &[u8], max_samples: usize) -> Result<Samples, Error> {
    use rawler::decompressors::ljpeg::LjpegDecompressor;
    std::panic::catch_unwind(|| -> Result<Samples, String> {
        let ljpeg = LjpegDecompressor::new(b)?;
        let (w, h) = (ljpeg.width(), ljpeg.height());
        match w.checked_mul(h) {
            Some(n) if n <= max_samples => {}
            _ => return Err(format!("a {w}x{h} stream is larger than its tile")),
        }
        let mut data = Vec::new();
        data.try_reserve_exact(w * h)
            .map_err(|_| format!("no memory for a {w}x{h} stream"))?;
        data.resize(w * h, 0u16);
        ljpeg.decode(&mut data, 0, w, w, h, false)?;
        Ok(Samples {
            width: w,
            height: h,
            data,
        })
    })
    .map_err(|_| corrupt("the decoder panicked"))?
    .map_err(Error::Corrupt)
}

/// Whether a stream declares a restart interval: what rawler's own
/// decoders get wrong. A stream whose header does not parse counts as having
/// none; the decoder that reads it reports the damage.
pub(crate) fn has_restarts(b: &[u8]) -> bool {
    header(b).is_ok_and(|h| h.restart != 0)
}

/// Decodes a lossless JPEG stream of at most `max_samples` samples.
pub(crate) fn decode(b: &[u8], max_samples: usize) -> Result<Samples, Error> {
    let h = header(b)?;
    if h.restart == 0 {
        return decode_one(b, max_samples);
    }
    if !h.one_by_one || h.width == 0 || h.restart % h.width != 0 {
        return Err(Error::Unsupported(format!(
            "lossless JPEG with a restart interval of {} for {} columns",
            h.restart, h.width
        )));
    }
    let rows = h.restart / h.width;
    let intervals = h.height.div_ceil(rows);
    let row_len = h.width * h.components;
    if h.height.saturating_mul(row_len) > max_samples {
        return Err(corrupt("the stream is larger than its tile"));
    }
    let (markers, end) = restart_markers(b, h.sos.1)?;
    if markers.len() + 1 != intervals {
        return Err(corrupt(format!(
            "{} restart markers for {intervals} intervals",
            markers.len()
        )));
    }
    let starts = std::iter::once(h.sos.1).chain(markers.iter().map(|m| m + 2));
    let ends = markers.iter().copied().chain(std::iter::once(end));
    let pieces: Vec<(usize, &[u8])> = starts
        .zip(ends)
        .enumerate()
        .map(|(i, (s, e))| (rows.min(h.height - i * rows), &b[s..e]))
        .collect();
    let decoded = pieces
        .par_iter()
        .map(|&(n, entropy)| {
            // SOI, SOF3 with this interval's height, the tables, SOS, the
            // interval's entropy data, EOI. No DRI.
            let mut s = Vec::with_capacity(entropy.len() + 512);
            s.extend([0xFF, 0xD8]);
            let sof = &b[h.sof.0..h.sof.1];
            s.extend(&sof[..5]);
            s.extend((n as u16).to_be_bytes());
            s.extend(&sof[7..]);
            for &(ts, te) in &h.tables {
                s.extend(&b[ts..te]);
            }
            s.extend(&b[h.sos.0..h.sos.1]);
            s.extend(entropy);
            s.extend([0xFF, 0xD9]);
            let part = decode_one(&s, n * row_len)?;
            if part.width != row_len || part.height != n {
                return Err(corrupt("a restart interval decoded to the wrong size"));
            }
            Ok(part.data)
        })
        .collect::<Result<Vec<_>, Error>>()?;
    Ok(Samples {
        width: row_len,
        height: h.height,
        data: decoded.concat(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Vec<u8> {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/dng");
        std::fs::read(format!("{dir}/{name}")).unwrap()
    }

    fn sample(x: usize, y: usize, c: usize) -> u16 {
        ((37 * x + 91 * y + 13 * c) % 1100) as u16
    }

    #[test]
    fn decodes_a_stream_with_restarts() {
        // 40x27, 3 components, a restart every 4 rows (make-restarts.py).
        let b = fixture("lj12_restarts.ljpg");
        let s = decode(&b, usize::MAX).unwrap();
        assert_eq!((s.width, s.height), (120, 27));
        for y in 0..27 {
            for x in 0..40 {
                for c in 0..3 {
                    assert_eq!(
                        s.data[y * 120 + x * 3 + c],
                        sample(x, y, c),
                        "({x}, {y}) {c}"
                    );
                }
            }
        }
        // rawler alone gets the rows after the first restart wrong: the
        // reason this module exists.
        let whole = decode_one(&b, usize::MAX).unwrap();
        assert_ne!(whole.data, s.data);
        assert_eq!(whole.data[..4 * 120], s.data[..4 * 120]);
    }

    #[test]
    fn rejects_restart_layouts_it_cannot_split() {
        let b = fixture("lj12_restarts.ljpg");
        let dri = b.windows(2).position(|w| w == [0xFF, 0xDD]).unwrap();
        // An interval of 2.5 rows.
        let mut odd = b.clone();
        odd[dri + 4..dri + 6].copy_from_slice(&100u16.to_be_bytes());
        assert!(matches!(
            decode(&odd, usize::MAX),
            Err(Error::Unsupported(_))
        ));
        // An interval of 2 rows: twice the markers the stream holds.
        let mut short = b.clone();
        short[dri + 4..dri + 6].copy_from_slice(&80u16.to_be_bytes());
        assert!(
            matches!(decode(&short, usize::MAX), Err(Error::Corrupt(m)) if m.contains("restart markers"))
        );
        // Markers out of order.
        let mut swapped = b.clone();
        let rst = swapped.windows(2).position(|w| w == [0xFF, 0xD1]).unwrap();
        swapped[rst + 1] = 0xD3;
        assert!(matches!(
            decode(&swapped, usize::MAX),
            Err(Error::Corrupt(_))
        ));
        // More samples than the tile holds.
        assert!(matches!(decode(&b, 100), Err(Error::Corrupt(_))));
    }

    #[test]
    fn damaged_streams_never_panic() {
        let b = fixture("lj12_restarts.ljpg");
        for len in 0..b.len() {
            let _ = decode(&b[..len], usize::MAX);
        }
        for i in 0..b.len() {
            let mut d = b.clone();
            d[i] ^= 0xFF;
            let _ = decode(&d, 1 << 20);
        }
    }
}
