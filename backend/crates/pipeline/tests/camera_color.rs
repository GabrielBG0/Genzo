//! The decoder's camera color data against the tag values the DNG SDK read
//! (color/tests/fixtures/dng-sdk-color.jsonl): for every corpus DNG the
//! fixture covers, the profile Genzo builds is the one the color crate's
//! SDK test checked, and its as-shot white is the SDK's. The patched copies
//! keep the original's color tags, so the iPhone 16 Pro ProRAW is read from
//! the original; the single-illuminant variant has no original to read.

use genzo_color::Mat3;
use genzo_color::camera::{AsShot, ColorSpec};
use genzo_golden::corpus;
use genzo_imgio::dng::read_metadata;
use genzo_pipeline::camera_color::{as_shot, dng_profile};
use serde_json::Value;

const FIXTURE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../color/tests/fixtures/dng-sdk-color.jsonl"
));

fn numbers(v: &Value) -> Vec<f64> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_f64().unwrap())
        .collect()
}

fn mat(v: Option<&Value>) -> Option<Mat3> {
    let n = numbers(v?);
    Some([[n[0], n[1], n[2]], [n[3], n[4], n[5]], [n[6], n[7], n[8]]])
}

/// Equal up to the last bit or so: serde_json's default float parsing is not
/// always correctly rounded.
fn same(a: impl IntoIterator<Item = f64>, b: impl IntoIterator<Item = f64>) -> bool {
    let (a, b): (Vec<f64>, Vec<f64>) = (a.into_iter().collect(), b.into_iter().collect());
    a.len() == b.len()
        && a.iter()
            .zip(&b)
            .all(|(x, y)| (x - y).abs() <= 1e-15 * x.abs().max(1.0))
}

fn same_mat(a: Option<Mat3>, b: Option<Mat3>, key: &str) {
    let ok = match (a, b) {
        (Some(a), Some(b)) => same(a.into_iter().flatten(), b.into_iter().flatten()),
        (a, b) => a.is_none() && b.is_none(),
    };
    assert!(ok, "{key}: {a:?} vs {b:?}");
}

#[test]
fn decoder_reads_what_the_sdk_read() {
    let mut checked = 0;
    for line in FIXTURE.lines() {
        let r: Value = serde_json::from_str(line).unwrap();
        let key = r["key"].as_str().unwrap();
        if r["variant"].as_str() == Some("illuminant_2_unknown") {
            continue;
        }
        let Some(path) = corpus(key) else { continue };
        let (meta, _) = read_metadata(&std::fs::read(path).unwrap()).unwrap();
        let p = dng_profile(&meta.color).expect("a three-color profile");
        let i = &r["inputs"];
        let s = |k: &str| i.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
        let il = |k: &str| i.get(k).map_or(0, |v| v.as_u64().unwrap() as u16);
        same_mat(Some(p.color_matrix_1), mat(i.get("color_matrix_1")), key);
        same_mat(p.color_matrix_2, mat(i.get("color_matrix_2")), key);
        same_mat(p.forward_matrix_1, mat(i.get("forward_matrix_1")), key);
        same_mat(p.forward_matrix_2, mat(i.get("forward_matrix_2")), key);
        same_mat(
            p.camera_calibration_1,
            mat(i.get("camera_calibration_1")),
            key,
        );
        same_mat(
            p.camera_calibration_2,
            mat(i.get("camera_calibration_2")),
            key,
        );
        assert_eq!(
            (p.illuminant_1, p.illuminant_2),
            (il("illuminant_1"), il("illuminant_2")),
            "{key}"
        );
        let ab = i.get("analog_balance").map_or(vec![1.0; 3], numbers);
        assert!(same(p.analog_balance, ab), "{key}");
        assert_eq!(
            p.camera_calibration_signature,
            s("camera_calibration_signature"),
            "{key}"
        );
        assert_eq!(
            p.profile_calibration_signature,
            s("profile_calibration_signature"),
            "{key}"
        );
        match (as_shot(&meta.color), i.get("as_shot_neutral")) {
            (Some(AsShot::Neutral(n)), Some(want)) => assert!(same(n, numbers(want)), "{key}"),
            (got, want) => panic!("{key}: as-shot {got:?}, fixture {want:?}"),
        }

        // End to end: the white Genzo renders with is the SDK's.
        let spec = ColorSpec::new(&p).unwrap();
        let xy = spec.as_shot_xy(as_shot(&meta.color)).unwrap();
        let sdk = numbers(&r["sdk"]["specs"][0]["xy"]);
        assert!(
            (xy[0] - sdk[0]).abs() < 1e-11 && (xy[1] - sdk[1]).abs() < 1e-11,
            "{key}: {xy:?} vs {sdk:?}"
        );
        checked += 1;
    }
    println!("checked {checked} corpus DNGs");
}
