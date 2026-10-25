//! `genzo-cli bench open` on a folder of small PNGs: it opens the folder,
//! switches to a prefetched neighbor and renders photos not prefetched, and
//! writes each metric. Skipped without a GPU adapter.

use std::path::PathBuf;
use std::process::Command;

/// A `width` × `height` 8-bit RGB PNG of a gradient shifted by `seed`,
/// stored without compression.
fn png(width: u32, height: u32, seed: u8) -> Vec<u8> {
    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc = !0u32;
        for &b in bytes {
            crc ^= u32::from(b);
            for _ in 0..8 {
                crc = if crc & 1 != 0 {
                    (crc >> 1) ^ 0xEDB8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }
    fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
        out.extend((data.len() as u32).to_be_bytes());
        let start = out.len();
        out.extend(kind);
        out.extend(data);
        let crc = crc32(&out[start..]);
        out.extend(crc.to_be_bytes());
    }
    let mut raw = Vec::new();
    for y in 0..height {
        raw.push(0);
        for x in 0..width {
            raw.extend([(x as u8).wrapping_add(seed), y as u8, seed]);
        }
    }
    // zlib with stored deflate blocks, and its Adler-32.
    let mut z = vec![0x78, 0x01];
    let blocks: Vec<&[u8]> = raw.chunks(65535).collect();
    for (i, block) in blocks.iter().enumerate() {
        z.push(u8::from(i + 1 == blocks.len()));
        let len = block.len() as u16;
        z.extend(len.to_le_bytes());
        z.extend((!len).to_le_bytes());
        z.extend(*block);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &byte in &raw {
        a = (a + u32::from(byte)) % 65521;
        b = (b + a) % 65521;
    }
    z.extend(((b << 16) | a).to_be_bytes());

    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::new();
    ihdr.extend(width.to_be_bytes());
    ihdr.extend(height.to_be_bytes());
    ihdr.extend([8, 2, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", &[]);
    out
}

struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn bench_open_measures_the_switches() {
    let dir =
        TempDir(std::env::temp_dir().join(format!("genzo-bench-open-{}", std::process::id())));
    let _ = std::fs::remove_dir_all(&dir.0);
    std::fs::create_dir_all(&dir.0).unwrap();
    // Seven photos: a switch to photo 2, a fit render of photo 6.
    for i in 1..=7u8 {
        std::fs::write(dir.0.join(format!("p{i}.png")), png(320, 240, i * 30)).unwrap();
    }
    let json = dir.0.join("metrics.json");
    let out = Command::new(env!("CARGO_BIN_EXE_genzo-cli"))
        .args(["bench", "open"])
        .arg(&dir.0)
        .args(["160", "120", "1", "--json"])
        .arg(&json)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    if stdout.contains("adapter: none") {
        assert_ne!(
            std::env::var("GENZO_REQUIRE_GPU").as_deref(),
            Ok("1"),
            "no GPU adapter"
        );
        eprintln!("skipped: no GPU adapter");
        return;
    }
    assert!(out.status.success(), "{stdout}\n{stderr}");
    assert!(stdout.contains("switch to p2.png"), "{stdout}");
    assert!(stdout.contains("fit render of p6.png"), "{stdout}");
    let metrics: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&json).unwrap()).unwrap();
    for name in ["open/fit render", "open/neighbor switch"] {
        let p95 = metrics[name]["p95_ms"].as_f64();
        assert!(p95.is_some_and(|ms| ms > 0.0), "{name}: {metrics}");
    }
    // PNGs carry no embedded preview.
    assert!(metrics.get("open/first pixels").is_none(), "{metrics}");
}
