//! DEV-01, stage 0: what `imgio::decode` reads from CFA files (camera RAW
//! and CFA DNG), in the same form as scripts/cfa-reference.py prints
//! LibRaw's reading, so the two can be compared line by line.
//!
//!   cargo run --release -p genzo-imgio --features raw --example cfa_report -- FILE...

use genzo_imgio::raw::AsShot;
use genzo_imgio::{DecodeOptions, Pixels, decode};

fn main() {
    for path in std::env::args().skip(1) {
        println!("{path}");
        let img = match std::fs::read(&path)
            .map_err(|e| e.to_string())
            .and_then(|d| decode(&d, DecodeOptions::default()).map_err(|e| e.to_string()))
        {
            Ok(img) => img,
            Err(e) => {
                println!("  Genzo: {e}");
                continue;
            }
        };
        let Pixels::Cfa { pattern, data, raw } = &img.pixels else {
            println!("  Genzo: not a CFA image");
            continue;
        };
        let (w, h) = (img.width as usize, img.height as usize);
        println!(
            "  sensor {w}x{h}, active area {:?}, default crop {:?}, orientation {}",
            raw.active_area, raw.default_crop, img.orientation
        );
        let rows: Vec<String> = (0..6)
            .map(|y| {
                (0..6)
                    .map(|x| ["R", "G", "B"][usize::from(pattern.color_at(y, x))])
                    .collect()
            })
            .collect();
        println!("  pattern from (0,0): {}", rows.join(" "));
        println!(
            "  black {:?} over {:?}, white {:?}",
            raw.levels.black, raw.levels.black_repeat, raw.levels.white
        );
        let wb = match raw.color.as_shot {
            Some(AsShot::Neutral(n)) => format!(
                "neutral {n:?}, as coefficients [{:.6}, 1, {:.6}]",
                1.0 / n[0],
                1.0 / n[2]
            ),
            other => format!("{other:?}"),
        };
        println!("  camera white balance {wb}");
        let mut sums = [0u64; 3];
        let mut counts = [0u64; 3];
        for y in 0..h {
            for x in 0..w {
                let c = usize::from(pattern.color_at(y, x));
                sums[c] += u64::from(data[y * w + x]);
                counts[c] += 1;
            }
        }
        let means: Vec<u64> = (0..3)
            .map(|c| (sums[c] + counts[c] / 2) / counts[c].max(1))
            .collect();
        let n = data.len() as u64;
        let total: u64 = sums.iter().sum();
        println!(
            "  means per color {means:?}, overall {}",
            (total + n / 2) / n
        );
        let checksum = data.iter().enumerate().fold(0u64, |acc, (i, &v)| {
            acc.wrapping_add(u64::from(v).wrapping_mul((i % 65521) as u64 + 1))
        });
        println!("  checksum {checksum}");
    }
}
