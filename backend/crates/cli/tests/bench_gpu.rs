//! `genzo-cli bench gpu` on a small synthetic RAW: each case runs through
//! Develop's commands and writes its metrics. Skipped without a GPU adapter.

use std::process::Command;

#[test]
fn bench_gpu_measures_each_case_on_the_develop_path() {
    let json = std::env::temp_dir().join(format!("genzo-bench-gpu-{}.json", std::process::id()));
    let out = Command::new(env!("CARGO_BIN_EXE_genzo-cli"))
        .args(["bench", "gpu", "640", "480", "3", "--json"])
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
    let text = std::fs::read_to_string(&json).unwrap();
    let _ = std::fs::remove_file(&json);
    let metrics: serde_json::Value = serde_json::from_str(&text).unwrap();
    let timestamps = stdout.contains("timestamps: yes");
    assert!(timestamps || stdout.contains("timestamps: no"), "{stdout}");
    for case in [
        "exposure drag",
        "white balance drag",
        "exposure settled",
        "white balance settled",
    ] {
        let engine = metrics[format!("develop/{case}/engine")]["p95_ms"].as_f64();
        assert!(engine.is_some_and(|ms| ms > 0.0), "{case}: {metrics}");
        let gpu = metrics[format!("develop/{case}/gpu")]["p95_ms"].as_f64();
        assert_eq!(gpu.is_some(), timestamps, "{case}: {metrics}");
        assert!(gpu.is_none_or(|ms| ms > 0.0), "{case}: {metrics}");
    }
}
