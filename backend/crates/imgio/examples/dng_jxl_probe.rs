//! 0.C prototype: read a JPEG XL-compressed LinearRaw DNG (iPhone 16 Pro ProRAW,
//! DNG 1.7, compression 52546) with a small TIFF reader and jxl-oxide.
//!
//! cargo run --release -p genzo-imgio --features raw --example dng_jxl_probe -- FILE [full|mip]
//!
//! `full` assembles the whole image as u16 (what a naive decoder does). `mip`
//! decodes tile by tile, box-filters each tile down 8x into a small level and
//! drops it, which is how the pipeline will feed the mip pyramid.

use std::time::Instant;

use jxl_oxide::JxlImage;

const COMPRESSION_JXL: u32 = 52546;

struct Tiff<'a> {
    d: &'a [u8],
    be: bool,
}

#[derive(Debug, Clone, Copy)]
struct Entry {
    ty: u16,
    count: u32,
    /// The 4 value bytes: the value itself if it fits, else an offset.
    raw: [u8; 4],
}

impl<'a> Tiff<'a> {
    fn u16(&self, o: usize) -> u16 {
        let b = [self.d[o], self.d[o + 1]];
        if self.be {
            u16::from_be_bytes(b)
        } else {
            u16::from_le_bytes(b)
        }
    }
    fn u32(&self, o: usize) -> u32 {
        let b = [self.d[o], self.d[o + 1], self.d[o + 2], self.d[o + 3]];
        if self.be {
            u32::from_be_bytes(b)
        } else {
            u32::from_le_bytes(b)
        }
    }
    fn ifd(&self, off: usize) -> Vec<(u16, Entry)> {
        let n = self.u16(off) as usize;
        (0..n)
            .map(|i| {
                let e = off + 2 + i * 12;
                let raw = [self.d[e + 8], self.d[e + 9], self.d[e + 10], self.d[e + 11]];
                (
                    self.u16(e),
                    Entry {
                        ty: self.u16(e + 2),
                        count: self.u32(e + 4),
                        raw,
                    },
                )
            })
            .collect()
    }
    /// All values of a SHORT or LONG entry.
    fn values(&self, e: Entry) -> Vec<u32> {
        let size = if e.ty == 3 { 2 } else { 4 };
        let total = size * e.count as usize;
        let base = if total <= 4 {
            None
        } else {
            Some(self.u32_of(e.raw) as usize)
        };
        (0..e.count as usize)
            .map(|i| match (base, e.ty) {
                (Some(b), 3) => u32::from(self.u16(b + i * 2)),
                (Some(b), _) => self.u32(b + i * 4),
                (None, 3) => {
                    let o = i * 2;
                    let b = [e.raw[o], e.raw[o + 1]];
                    u32::from(if self.be {
                        u16::from_be_bytes(b)
                    } else {
                        u16::from_le_bytes(b)
                    })
                }
                (None, _) => self.u32_of(e.raw),
            })
            .collect()
    }
    fn u32_of(&self, b: [u8; 4]) -> u32 {
        if self.be {
            u32::from_be_bytes(b)
        } else {
            u32::from_le_bytes(b)
        }
    }
}

fn get(ifd: &[(u16, Entry)], tag: u16) -> Option<Entry> {
    ifd.iter().find(|(t, _)| *t == tag).map(|(_, e)| *e)
}

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("usage: dng_jxl_probe FILE [full|mip]");
    let mode = args.next().unwrap_or_else(|| "full".into());
    let data = std::fs::read(&path).expect("read file");
    let tiff = Tiff {
        d: &data,
        be: &data[..2] == b"MM",
    };
    let ifd0 = tiff.ifd(tiff.u32(4) as usize);

    // The raw image is in IFD0 or one of its SubIFDs: the one with NewSubFileType 0.
    let mut candidates = vec![ifd0.clone()];
    if let Some(e) = get(&ifd0, 330) {
        for off in tiff.values(e) {
            candidates.push(tiff.ifd(off as usize));
        }
    }
    let raw = candidates
        .into_iter()
        .find(|ifd| {
            get(ifd, 254).is_none_or(|e| tiff.values(e)[0] == 0)
                && get(ifd, 259).is_some_and(|e| tiff.values(e)[0] == COMPRESSION_JXL)
        })
        .expect("no JPEG XL raw IFD");
    let one = |tag| {
        tiff.values(get(&raw, tag).unwrap_or_else(|| panic!("missing tag {tag}")))[0] as usize
    };
    let (w, h, spp, tw, th) = (one(256), one(257), one(277), one(322), one(323));
    let offsets = tiff.values(get(&raw, 324).expect("TileOffsets"));
    let counts = tiff.values(get(&raw, 325).expect("TileByteCounts"));
    let (tx, ty) = (w.div_ceil(tw), h.div_ceil(th));
    println!(
        "{w}x{h}, {spp} samples, tiles {tw}x{th} ({tx}x{ty} = {}), mode {mode}",
        offsets.len()
    );
    assert_eq!(offsets.len(), tx * ty);

    // DNG LinearizationTable: stored value -> linear 16-bit. Values past the end of the
    // table use its last entry (this file stores up to 1029 with a 1024-entry table).
    let table: Vec<u16> = get(&raw, 50712)
        .map(|e| tiff.values(e).into_iter().map(|v| v as u16).collect())
        .unwrap_or_default();
    let (black, white) = (
        get(&raw, 50714).map(|e| tiff.values(e)),
        get(&raw, 50717).map(|e| tiff.values(e)),
    );
    println!(
        "linearization table: {} entries, black {black:?}, white {white:?}",
        table.len()
    );
    println!(
        "ifd0: ColorMatrix1 {} ColorMatrix2 {} AsShotNeutral {} BaselineExposure {} ForwardMatrix1 {} | OpcodeList1/2/3: {}/{}/{} (ifd0 or raw ifd)",
        get(&ifd0, 50721).is_some(),
        get(&ifd0, 50722).is_some(),
        get(&ifd0, 50728).is_some(),
        get(&ifd0, 50730).is_some(),
        get(&ifd0, 50964).is_some(),
        [51008]
            .iter()
            .any(|&t| get(&ifd0, t).is_some() || get(&raw, t).is_some()),
        [51009]
            .iter()
            .any(|&t| get(&ifd0, t).is_some() || get(&raw, t).is_some()),
        [51022]
            .iter()
            .any(|&t| get(&ifd0, t).is_some() || get(&raw, t).is_some()),
    );
    let mut clamped = 0u64;

    const SHRINK: usize = 8;
    let (mw, mh) = (w.div_ceil(SHRINK), h.div_ceil(SHRINK));
    let mut full = if mode == "full" {
        vec![0u16; w * h * spp]
    } else {
        Vec::new()
    };
    let mut mip = vec![0u32; mw * mh * spp];
    let mut mip_n = vec![0u32; mw * mh];

    let started = Instant::now();
    for (i, (&off, &len)) in offsets.iter().zip(&counts).enumerate() {
        let t0 = Instant::now();
        let bytes = &data[off as usize..(off + len) as usize];
        let image = JxlImage::builder().read(bytes).expect("jxl header");
        let render = image.render_frame(0).expect("jxl render");
        let mut stream = render.stream_no_alpha();
        if i == 0 {
            println!(
                "jxl bit depth {}",
                image.image_header().metadata.bit_depth.bits_per_sample()
            );
        }
        let (sw, sh, ch) = (
            stream.width() as usize,
            stream.height() as usize,
            stream.channels() as usize,
        );
        assert_eq!(ch, spp);
        let mut buf = vec![0u16; sw * sh * ch];
        assert_eq!(stream.write_to_buffer(&mut buf), buf.len());
        let (x0, y0) = ((i % tx) * tw, (i / tx) * th);
        for y in 0..sh {
            for x in 0..sw {
                let mut px = [0u16; 3];
                px[..ch].copy_from_slice(&buf[(y * sw + x) * ch..][..ch]);
                if !table.is_empty() {
                    for v in &mut px[..ch] {
                        clamped += u64::from(usize::from(*v) >= table.len());
                        *v = table[usize::from(*v).min(table.len() - 1)];
                    }
                }
                let px = &px[..ch];
                if mode == "full" {
                    full[((y0 + y) * w + x0 + x) * spp..][..spp].copy_from_slice(px);
                }
                let m = ((y0 + y) / SHRINK) * mw + (x0 + x) / SHRINK;
                for c in 0..ch {
                    mip[m * spp + c] += u32::from(px[c]);
                }
                mip_n[m] += 1;
            }
        }
        {
            let (mn, mx) = buf
                .iter()
                .fold((u16::MAX, 0), |(a, b), &v| (a.min(v), b.max(v)));
            println!(
                "tile 0: {sw}x{sh}, first tile decode {:?}, sample range {mn}..{mx}",
                t0.elapsed()
            );
        }
    }
    println!("samples past the end of the table (clamped): {clamped}");
    println!("decoded {} tiles in {:?}", offsets.len(), started.elapsed());

    let mean = |c: usize| {
        mip.iter()
            .skip(c)
            .step_by(spp)
            .map(|&v| u64::from(v))
            .sum::<u64>() as f64
            / mip_n.iter().map(|&n| u64::from(n)).sum::<u64>() as f64
    };
    println!(
        "mean sample per channel: {:.0} {:.0} {:.0}",
        mean(0),
        mean(1),
        mean(2)
    );
    println!("mip level 1/{SHRINK}: {mw}x{mh}");
    if mode == "full" {
        println!("full buffer: {} MiB", full.len() * 2 / (1 << 20));
    }
}
