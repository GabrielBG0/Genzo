//! The status bar's notes for a photo (APP-07): what the pipeline does not
//! do as the file asks, in the order the stages meet them.

use genzo_color::encoding::InputEncoding;
use genzo_pipeline::gpu::synthetic::{scene_raw, with_camera_profile};
use genzo_pipeline::raw::{RawSource, note};

fn notes(src: &RawSource) -> Vec<String> {
    src.notes()
}

#[test]
fn a_camera_with_a_profile_has_none() {
    assert!(with_camera_profile(scene_raw(64, 48)).notes().is_empty());
}

#[test]
fn each_fallback_is_said() {
    // No three-color matrix: camera values pass through.
    assert_eq!(notes(&scene_raw(64, 48)), [note::NO_PROFILE]);
    // A profile but no as-shot data: D55.
    let mut no_white = with_camera_profile(scene_raw(64, 48));
    no_white.color.as_shot = None;
    assert_eq!(notes(&no_white), [note::WHITE]);
    // An invalid ProfileToneCurve (an odd count): the default curve.
    let mut curve = with_camera_profile(scene_raw(64, 48));
    curve.color.profile_tone_curve = Some(vec![0.0, 0.0, 1.0]);
    assert_eq!(
        notes(&curve),
        ["The file's tone curve can't be read; using the default"]
    );
    // A skipped lens opcode comes first: stage 1 before stage 4.
    let mut lens = scene_raw(64, 48);
    lens.lens_corrections_skipped = true;
    assert_eq!(notes(&lens), [note::LENS, note::NO_PROFILE]);
}

#[test]
fn a_non_raw_image_has_no_color_notes() {
    let encoding = InputEncoding::srgb();
    let src = RawSource::from_encoded((2, 2), &[0; 16], &encoding).unwrap();
    assert!(src.notes().is_empty());
}
