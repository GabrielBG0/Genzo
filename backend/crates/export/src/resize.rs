//! A resized export's reduction and stage 9 on the CPU (OUT-01, *Resized*).
//!
//! - The render thread hands the export's regions at full resolution, in
//!   the working space (linear AP1, after stages 1 to 7), row of regions by
//!   row, left to right.
//! - Each output pixel is the mean of the full-resolution pixels it covers:
//!   output pixel `i` of `n` covers source pixels `i * len / n` up to
//!   `(i + 1) * len / n`, as `preview::reduce` splits them. The sums are
//!   taken in f64 as the regions come, so an output pixel can span any
//!   number of regions; only the output rows the current row of regions
//!   touches are held.
//! - An output row is finished once every source row it covers has come.
//!   Stage 9 then runs on it (`color::output::pixel`): the output
//!   transform, the gamut compression, the transfer function, and the
//!   dither anchored at the output pixel, as DEV-01's CPU reference does.
//!   It runs on a pool of its own, half the cores: about 280 ns a pixel
//!   on one core, 0.8 s for a 2048 px export of a 24 MP photo. The global
//!   pool, where a switch decodes, is left alone, and so is half the CPU.

use std::collections::VecDeque;
use std::sync::OnceLock;

use rayon::prelude::*;

use genzo_color::output::pixel;
use genzo_model::frame::Rect;

pub use genzo_pipeline::gpu::export::Stage9;

/// Stage 9's pool: half the cores, at least one.
fn pool() -> &'static rayon::ThreadPool {
    static POOL: OnceLock<rayon::ThreadPool> = OnceLock::new();
    POOL.get_or_init(|| {
        let cores = std::thread::available_parallelism().map_or(2, |n| n.get());
        rayon::ThreadPoolBuilder::new()
            .num_threads((cores / 2).max(1))
            .thread_name(|i| format!("genzo-export-{i}"))
            .build()
            .expect("the export pool")
    })
}

/// Where `n` output pixels split `len` source pixels: the first source
/// pixel of each, and `len` at the end.
fn starts(len: u32, n: u32) -> Vec<u32> {
    (0..=n)
        .map(|i| (u64::from(i) * u64::from(len) / u64::from(n)) as u32)
        .collect()
}

/// For each of `len` source pixels, the output pixel it belongs to.
fn owners(starts: &[u32]) -> Vec<u32> {
    let mut out = Vec::with_capacity(*starts.last().unwrap_or(&0) as usize);
    for (i, w) in starts.windows(2).enumerate() {
        out.extend(std::iter::repeat_n(i as u32, (w[1] - w[0]) as usize));
    }
    out
}

/// The reduction of one export.
#[derive(Debug)]
pub struct Reducer {
    source: (u32, u32),
    size: (u32, u32),
    stage9: Stage9,
    col_of: Vec<u32>,
    row_of: Vec<u32>,
    col_starts: Vec<u32>,
    row_starts: Vec<u32>,
    /// Sums of the output rows from `first` on, not finished yet.
    rows: VecDeque<Vec<[f64; 3]>>,
    first: u32,
}

impl Reducer {
    /// The reduction of a `source` image to `size`, no larger on either
    /// side.
    pub fn new(source: (u32, u32), size: (u32, u32), stage9: Stage9) -> Self {
        assert!(size.0 >= 1 && size.1 >= 1 && size.0 <= source.0 && size.1 <= source.1);
        let col_starts = starts(source.0, size.0);
        let row_starts = starts(source.1, size.1);
        Self {
            source,
            size,
            stage9,
            col_of: owners(&col_starts),
            row_of: owners(&row_starts),
            col_starts,
            row_starts,
            rows: VecDeque::new(),
            first: 0,
        }
    }

    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// Adds region `rect`'s pixels (working space, row by row).
    pub fn add(&mut self, rect: Rect, rgb: &[[f32; 3]]) {
        assert_eq!(rgb.len(), rect.width as usize * rect.height as usize);
        assert!(rect.x + rect.width <= self.source.0 && rect.y + rect.height <= self.source.1);
        let width = self.size.0 as usize;
        for (r, line) in rgb.chunks_exact(rect.width as usize).enumerate() {
            let oy = self.row_of[(rect.y + r as u32) as usize];
            assert!(oy >= self.first, "a row already finished");
            let at = (oy - self.first) as usize;
            while self.rows.len() <= at {
                self.rows.push_back(vec![[0.0; 3]; width]);
            }
            let row = &mut self.rows[at];
            for (c, px) in line.iter().enumerate() {
                let ox = self.col_of[(rect.x + c as u32) as usize] as usize;
                let sum = &mut row[ox];
                for k in 0..3 {
                    sum[k] += f64::from(px[k]);
                }
            }
        }
    }

    /// The output rows finished once every source row before `done` has
    /// come: RGBA8, row by row, from output row `y`, with their count.
    /// Empty when none is.
    pub fn finish(&mut self, done: u32) -> (u32, Vec<u8>, u32) {
        let y = self.first;
        let width = self.size.0 as usize;
        let mut rows = Vec::new();
        while self.first < self.size.1 && self.row_starts[self.first as usize + 1] <= done {
            rows.push(
                self.rows
                    .pop_front()
                    .unwrap_or_else(|| vec![[0.0; 3]; width]),
            );
            self.first += 1;
        }
        let count = rows.len() as u32;
        let mut out = vec![0u8; rows.len() * width * 4];
        let (stage9, row_starts, col_starts) = (self.stage9, &self.row_starts, &self.col_starts);
        pool().install(|| {
            out.par_chunks_mut(width * 4)
                .zip(rows.par_iter())
                .enumerate()
                .for_each(|(i, (line, sums))| {
                    let oy = y + i as u32;
                    let height = f64::from(row_starts[oy as usize + 1] - row_starts[oy as usize]);
                    let pixels = line.as_chunks_mut::<4>().0.iter_mut().zip(sums);
                    for (ox, (px, sum)) in pixels.enumerate() {
                        let n = height * f64::from(col_starts[ox + 1] - col_starts[ox]);
                        let [r, g, b] = pixel(
                            sum.map(|s| s / n),
                            (ox as u32, oy),
                            stage9.target,
                            stage9.compress,
                            stage9.dither,
                        );
                        *px = [r, g, b, 255];
                    }
                });
        });
        (y, out, count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use genzo_color::output::Target;

    const STAGE9: Stage9 = Stage9 {
        target: Target::Srgb,
        compress: true,
        dither: true,
    };

    /// A linear image of `size`, with values from 0 to about 1.2.
    fn image(size: (u32, u32)) -> Vec<[f32; 3]> {
        (0..size.0 * size.1)
            .map(|i| {
                let (x, y) = (i % size.0, i / size.0);
                [
                    x as f32 / size.0 as f32 * 1.2,
                    y as f32 / size.1 as f32,
                    ((x * 7 + y * 13) % 97) as f32 / 97.0,
                ]
            })
            .collect()
    }

    /// The reference: each output pixel's mean taken over its source
    /// pixels directly, then stage 9.
    fn reference(src: &[[f32; 3]], source: (u32, u32), size: (u32, u32)) -> Vec<u8> {
        let (cs, rs) = (starts(source.0, size.0), starts(source.1, size.1));
        let mut out = Vec::new();
        for oy in 0..size.1 {
            for ox in 0..size.0 {
                let mut sum = [0.0f64; 3];
                let mut n = 0.0;
                for y in rs[oy as usize]..rs[oy as usize + 1] {
                    for x in cs[ox as usize]..cs[ox as usize + 1] {
                        let px = src[(y * source.0 + x) as usize];
                        for k in 0..3 {
                            sum[k] += f64::from(px[k]);
                        }
                        n += 1.0;
                    }
                }
                let [r, g, b] = pixel(sum.map(|s| s / n), (ox, oy), Target::Srgb, true, true);
                out.extend_from_slice(&[r, g, b, 255]);
            }
        }
        out
    }

    /// Feeds `src` as regions of `region` px, row of regions by row, and
    /// collects the finished rows.
    fn reduce(src: &[[f32; 3]], source: (u32, u32), size: (u32, u32), region: u32) -> Vec<u8> {
        let mut r = Reducer::new(source, size, STAGE9);
        let mut out = Vec::new();
        let mut next = 0;
        for ry in (0..source.1).step_by(region as usize) {
            let h = region.min(source.1 - ry);
            for rx in (0..source.0).step_by(region as usize) {
                let w = region.min(source.0 - rx);
                let pixels: Vec<[f32; 3]> = (0..h)
                    .flat_map(|y| {
                        let at = ((ry + y) * source.0 + rx) as usize;
                        src[at..at + w as usize].to_vec()
                    })
                    .collect();
                let rect = Rect {
                    x: rx,
                    y: ry,
                    width: w,
                    height: h,
                };
                r.add(rect, &pixels);
            }
            let (y, rows, count) = r.finish(ry + h);
            assert_eq!(y, next, "rows in order");
            next += count;
            out.extend(rows);
        }
        assert_eq!(next, size.1);
        out
    }

    #[test]
    fn regions_reduce_as_the_whole_image_does() {
        let source = (517, 389);
        let src = image(source);
        // Output pixels inside a region, across regions, and spanning
        // several (a 3 px output of a 517 px source over 100 px regions).
        for size in [(400, 301), (129, 97), (32, 24), (3, 2), (517, 389)] {
            let want = reference(&src, source, size);
            for region in [64, 100, 517] {
                let got = reduce(&src, source, size, region);
                assert_eq!(got.len(), want.len(), "{size:?}");
                let off = got.iter().zip(&want).filter(|(a, b)| a != b).count();
                assert_eq!(
                    off, 0,
                    "{size:?} in {region} px regions: {off} codes differ"
                );
            }
        }
    }

    #[test]
    fn a_flat_image_reduces_to_its_value() {
        let source = (300, 200);
        let src = vec![[0.18f32; 3]; 300 * 200];
        let got = reduce(&src, source, (77, 51), 128);
        let [r, _, _] = pixel([0.18; 3], (0, 0), Target::Srgb, true, false);
        // Dithered by at most one code around the undithered value.
        assert!(
            got.chunks(4)
                .all(|p| (i32::from(p[0]) - i32::from(r)).abs() <= 1)
        );
    }

    #[test]
    fn spans_cover_every_source_pixel_once() {
        for (len, n) in [(10u32, 3u32), (7, 7), (1000, 1), (6048, 2048)] {
            let s = starts(len, n);
            assert_eq!((s[0], *s.last().unwrap()), (0, len));
            assert!(s.windows(2).all(|w| w[1] > w[0]), "{len} into {n}");
            assert_eq!(owners(&s).len(), len as usize);
        }
    }
}
