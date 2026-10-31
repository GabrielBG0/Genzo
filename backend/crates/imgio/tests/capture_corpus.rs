//! Capture times of the local corpus (`corpus/`, gitignored): every file
//! the formats carry one for reads it, from its metadata only. Skipped when
//! the corpus is absent.
//!
//! And the shooting values (DEV-03, criterion 7) against ExifTool's EXIF
//! tags, from scripts/exif-reference.py; files missing from the corpus are
//! skipped.

use std::path::Path;

use genzo_imgio::capture::{capture, capture_time};

#[test]
fn corpus_files_have_capture_times() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../corpus");
    let mut files = Vec::new();
    let mut stack = vec![root.clone()];
    while let Some(dir) = stack.pop() {
        let Ok(read) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in read.flatten() {
            let p = e.path();
            if p.is_dir() {
                if !p.ends_with("reference") {
                    stack.push(p);
                }
            } else if p
                .extension()
                .is_some_and(|x| !["txt", "json", "md"].contains(&x.to_str().unwrap_or("")))
            {
                files.push(p);
            }
        }
    }
    if files.is_empty() {
        eprintln!("skipped: no corpus at {}", root.display());
        return;
    }
    files.sort();
    let mut missing = Vec::new();
    for f in &files {
        let data = std::fs::read(f).unwrap();
        let t = capture_time(&data);
        let name = f.strip_prefix(&root).unwrap().display().to_string();
        eprintln!("{name}: {t:?}");
        if t.is_none() {
            missing.push(name);
        }
    }
    eprintln!(
        "{} of {} without a capture time",
        missing.len(),
        files.len()
    );
}

/// What ExifTool reads from a corpus file's EXIF tags (DEV-03, criterion 7).
struct Shooting {
    file: &'static str,
    iso: Option<u32>,
    exposure_time: Option<f64>,
    f_number: Option<f64>,
    focal_length: Option<f64>,
}

/// ExifTool 13.59, EXIF group, -n: scripts/exif-reference.py, run on the local corpus.
const SHOOTING: &[Shooting] = &[
    Shooting {
        file: "bayer-dng/iphone/Halide_iPhone16Pro_bayer.dng",
        iso: Some(64),
        exposure_time: Some(0.02222222222),
        f_number: Some(2.2),
        focal_length: Some(2.220000029),
    },
    Shooting {
        file: "proraw/iPhone 16 Pro/IPhone16proRaw.DNG",
        iso: Some(64),
        exposure_time: Some(0.0001869857891),
        f_number: Some(1.779999971),
        focal_length: Some(6.764999866),
    },
    Shooting {
        file: "raw/Apple/iPhone 12 Pro/IMG_1361.DNG",
        iso: Some(32),
        exposure_time: Some(0.0001799856012),
        f_number: Some(1.6),
        focal_length: Some(4.2),
    },
    Shooting {
        file: "raw/Canon/Canon EOS R5m2/APS-C_CRAW.CR3",
        iso: Some(125),
        exposure_time: Some(0.06666666667),
        f_number: Some(3.2),
        focal_length: Some(135.0),
    },
    Shooting {
        file: "raw/Fujifilm/GFX100S/Fujifilm-GFX100S-14bits-compress-4_3.RAF",
        iso: Some(100),
        exposure_time: Some(0.005882352941),
        f_number: Some(4.0),
        focal_length: Some(45.0),
    },
    Shooting {
        file: "raw/Fujifilm/X-T5/DSCF0021.RAF",
        iso: Some(125),
        exposure_time: Some(0.00625),
        f_number: Some(4.0),
        focal_length: Some(18.0),
    },
    Shooting {
        file: "raw/Google/Pixel 4a/PXL_20201121_100251397.dng",
        iso: Some(55),
        exposure_time: Some(0.004005216993),
        f_number: Some(1.7),
        focal_length: Some(4.38),
    },
    Shooting {
        file: "raw/GoPro/HERO6 Black/GOPR8921.GPR",
        iso: Some(99),
        exposure_time: Some(5.0),
        f_number: Some(2.8),
        focal_length: Some(3.0),
    },
    Shooting {
        file: "raw/Hasselblad/H3D/A9391120.3FR",
        iso: Some(50),
        exposure_time: Some(0.002),
        f_number: Some(4.8),
        focal_length: Some(150.0),
    },
    Shooting {
        file: "raw/Kodak/DCS760C/86L57188.DCR",
        iso: None,
        exposure_time: Some(0.00625),
        f_number: Some(6.3),
        focal_length: Some(24.0),
    },
    Shooting {
        file: "raw/Leica/Q2/L1000750.DNG",
        iso: Some(100),
        exposure_time: Some(0.002),
        f_number: Some(6.3),
        focal_length: Some(28.0),
    },
    Shooting {
        file: "raw/Nikon/D850/Nikon-D850-12bit-compressed.NEF",
        iso: Some(64),
        exposure_time: Some(0.00625),
        f_number: Some(9.0),
        focal_length: Some(35.0),
    },
    Shooting {
        file: "raw/Nikon/Z 8/Nikon_Z8_high_efficiency_low.NEF",
        iso: Some(64),
        exposure_time: Some(0.0125),
        f_number: Some(8.0),
        focal_length: Some(24.0),
    },
    Shooting {
        file: "raw/Nikon/Z 9/Nikon_-_NIKON_Z_9_-_14bit_compressed_(Lossless).NEF",
        iso: Some(64),
        exposure_time: Some(0.003125),
        f_number: Some(9.0),
        focal_length: Some(18.0),
    },
    Shooting {
        file: "raw/Nikon/Z 9/Nikon_-_NIKON_Z_9_-_14bit_compressed_(Lossy_High_Efficiency).NEF",
        iso: Some(64),
        exposure_time: Some(0.003125),
        f_number: Some(9.0),
        focal_length: Some(18.0),
    },
    Shooting {
        file: "raw/Olympus/E-M1MarkII/Olympus_EM1mk2_Standard_20MP.ORF",
        iso: Some(400),
        exposure_time: Some(0.025),
        f_number: Some(2.8),
        focal_length: Some(28.0),
    },
    Shooting {
        file: "raw/OM Digital Solutions/OM-3/P5048182.ORF",
        iso: Some(200),
        exposure_time: Some(0.003125),
        f_number: Some(6.3),
        focal_length: Some(17.0),
    },
    Shooting {
        file: "raw/Panasonic/DC-S5M2/P1126458_mechanical.RW2",
        iso: Some(100),
        exposure_time: Some(0.025),
        f_number: Some(9.0),
        focal_length: Some(35.0),
    },
    Shooting {
        file: "raw/Pentax/K10D/IMGP5398.DNG",
        iso: Some(100),
        exposure_time: Some(0.008),
        f_number: Some(5.6),
        focal_length: Some(55.0),
    },
    Shooting {
        file: "raw/Phase One/P65+/CF027310.IIQ",
        iso: Some(200),
        exposure_time: Some(0.04),
        f_number: Some(8.0),
        focal_length: Some(80.0),
    },
    Shooting {
        file: "raw/Samsung/Galaxy S23 Ultra/20230817_120455.dng",
        iso: Some(100),
        exposure_time: Some(0.0303030303),
        f_number: Some(4.9),
        focal_length: Some(27.2),
    },
    Shooting {
        file: "raw/Sony/ILCE-7RM5/7RM5-LosslessCompressedLarge.ARW",
        iso: Some(100),
        exposure_time: Some(0.0005),
        f_number: Some(1.2),
        focal_length: Some(50.0),
    },
];

/// ExifTool prints 10 significant digits.
fn close(got: Option<f64>, want: Option<f64>) -> bool {
    match (got, want) {
        (Some(g), Some(w)) => ((g - w) / w).abs() < 1e-9,
        (g, w) => g == w,
    }
}

#[test]
fn corpus_files_are_shot_as_exiftool_reads_them() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../corpus");
    let mut checked = 0;
    let mut wrong = Vec::new();
    for want in SHOOTING {
        let Ok(data) = std::fs::read(root.join(want.file)) else {
            continue;
        };
        let s = capture(&data).shooting;
        let ok = s.iso == want.iso
            && close(s.exposure_time.map(|r| r.value()), want.exposure_time)
            && close(s.f_number.map(|r| r.value()), want.f_number)
            && close(s.focal_length.map(|r| r.value()), want.focal_length);
        if !ok {
            wrong.push(format!("{}: {s:?}", want.file));
        }
        checked += 1;
    }
    if checked == 0 {
        eprintln!("skipped: no corpus at {}", root.display());
        return;
    }
    assert!(
        wrong.is_empty(),
        "{} of {checked}:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}
