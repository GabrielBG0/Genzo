//! APP-10's acceptance criteria 1 to 7, on the edit and its history
//! (`engine::history`), without rendering.

use genzo_engine::api::edit::{Change, Param, Phase, PhotoKind, Treatment, WhiteBalanceMode};
use genzo_engine::history::{PhotoEdits, Refused};

const RAW: PhotoKind = PhotoKind::Raw {
    as_shot_temp: 5200.0,
    as_shot_tint: 6.0,
};

fn photo() -> PhotoEdits {
    PhotoEdits::new(RAW, (6000, 4000), 1)
}

/// A gesture: a start, `n` updates and an end.
fn drag(p: &mut PhotoEdits, n: usize, value: impl Fn(usize) -> Change) -> bool {
    p.apply(&[value(0)], Phase::Start).unwrap();
    for i in 1..=n {
        p.apply(&[value(i)], Phase::Update).unwrap();
    }
    p.apply(&[value(n)], Phase::End).unwrap()
}

#[test]
fn one_step_per_finished_change() {
    let mut p = photo();
    assert!(drag(&mut p, 50, |i| Change::Exposure(i as f32 / 50.0)));
    assert_eq!(p.steps().len(), 1, "a gesture of 50 updates");
    for (change, label) in [
        (Change::Contrast(30.0), "Contrast"),
        (Change::Reset(Param::Exposure), "Reset Exposure"),
        (Change::Treatment(Treatment::Monochrome), "Treatment"),
        (
            Change::WhiteBalance {
                mode: WhiteBalanceMode::Custom,
                temp: 3400.0,
                tint: -4.0,
            },
            "White balance",
        ),
        (Change::Turn { clockwise: true }, "Rotate"),
        (Change::Flip, "Rotate"),
        (Change::ResetAll, "Reset all"),
    ] {
        let before = p.steps().len();
        assert!(p.apply(&[change], Phase::End).unwrap(), "{label}");
        assert_eq!(p.steps().len(), before + 1, "{label}");
        assert_eq!(p.steps().last().unwrap().label, label);
    }
    assert!(!p.is_edited(), "Reset all is the defaults");
}

#[test]
fn no_step_for_no_change() {
    let mut p = photo();
    // A gesture that ends where it started.
    assert!(!drag(&mut p, 10, |i| Change::Shadows(if i == 10 {
        0.0
    } else {
        i as f32
    })));
    // A value clamped to the current one.
    p.apply(&[Change::Exposure(5.0)], Phase::End).unwrap();
    let n = p.steps().len();
    assert!(!p.apply(&[Change::Exposure(9.0)], Phase::End).unwrap());
    assert_eq!(p.steps().len(), n);
    // A reset of a value already at its default.
    assert!(
        !p.apply(&[Change::Reset(Param::Vibrance)], Phase::End)
            .unwrap()
    );
}

#[test]
fn undo_and_redo_round_trip() {
    let mut p = photo();
    let changes = [
        Change::Exposure(0.7),
        Change::Highlights(-40.0),
        Change::Crop {
            cx: 0.4,
            cy: 0.55,
            w: 0.5,
            h: 0.6,
            angle: 4.0,
        },
        Change::Turn { clockwise: false },
        Change::Vibrance(20.0),
    ];
    for c in &changes {
        assert!(p.apply(std::slice::from_ref(c), Phase::End).unwrap());
    }
    let last = p.current().clone();
    for _ in 0..changes.len() {
        assert!(p.undo());
    }
    assert!(!p.undo(), "nothing left to undo");
    assert_eq!(p.current(), p.defaults());
    for _ in 0..changes.len() {
        assert!(p.redo());
    }
    assert!(!p.redo());
    assert_eq!(*p.current(), last, "bit for bit");
}

#[test]
fn a_new_step_drops_the_redo_steps() {
    let mut p = photo();
    p.apply(&[Change::Exposure(1.0)], Phase::End).unwrap();
    p.apply(&[Change::Contrast(10.0)], Phase::End).unwrap();
    assert!(p.undo());
    assert_eq!(p.redo_label(), Some("Contrast"));
    p.apply(&[Change::Blacks(-5.0)], Phase::End).unwrap();
    assert_eq!(p.redo_label(), None);
    assert!(!p.redo());
    assert_eq!(p.current().contrast, 0.0);
    assert_eq!(p.current().blacks, -5.0);
}

#[test]
fn histories_are_per_photo() {
    let (mut a, mut b) = (photo(), photo());
    a.apply(&[Change::Exposure(1.0)], Phase::End).unwrap();
    a.apply(&[Change::Saturation(-30.0)], Phase::End).unwrap();
    b.apply(&[Change::Whites(15.0)], Phase::End).unwrap();
    let b_before = (b.current().clone(), b.steps().to_vec());
    assert!(a.undo());
    assert_eq!(a.current().saturation, 0.0);
    assert_eq!(a.current().exposure, 1.0);
    assert_eq!((b.current().clone(), b.steps().to_vec()), b_before);
}

#[test]
fn undo_and_redo_wait_for_a_gesture() {
    let mut p = photo();
    // One step to undo and one to redo.
    p.apply(&[Change::Exposure(1.0)], Phase::End).unwrap();
    p.apply(&[Change::Blacks(-8.0)], Phase::End).unwrap();
    assert!(p.undo());
    assert_eq!(
        (p.undo_label(), p.redo_label()),
        (Some("Exposure"), Some("Blacks"))
    );
    p.apply(&[Change::Contrast(5.0)], Phase::Start).unwrap();
    p.apply(&[Change::Contrast(25.0)], Phase::Update).unwrap();
    assert!(p.in_gesture());
    assert_eq!((p.undo_label(), p.redo_label()), (None, None));
    assert!(!p.undo() && !p.redo());
    // The gesture's edit is shown; the history is untouched until its end.
    assert_eq!(p.current().contrast, 25.0);
    assert_eq!(p.committed().contrast, 0.0);
    assert!(p.apply(&[Change::Contrast(25.0)], Phase::End).unwrap());
    assert_eq!(p.undo_label(), Some("Contrast"));
}

#[test]
fn labels_name_the_control() {
    let mut p = photo();
    drag(&mut p, 3, |i| Change::Whites(i as f32 * 10.0));
    assert_eq!(p.undo_label(), Some("Whites"));
    p.apply(&[Change::Reset(Param::Whites)], Phase::End)
        .unwrap();
    assert_eq!(p.undo_label(), Some("Reset Whites"));
    assert!(p.undo());
    assert_eq!(p.redo_label(), Some("Reset Whites"));
    // A pick or a preset sets the whole white balance at once.
    let wb = Change::WhiteBalance {
        mode: WhiteBalanceMode::Custom,
        temp: 6500.0,
        tint: 10.0,
    };
    p.apply(&[wb], Phase::End).unwrap();
    assert_eq!(p.undo_label(), Some("White balance"));
}

#[test]
fn a_crop_that_cannot_fit_is_refused() {
    let mut p = photo();
    let crop = Change::Crop {
        cx: 0.5,
        cy: 0.5,
        w: 0.5,
        h: 0.5,
        angle: 0.0,
    };
    p.apply(&[crop], Phase::End).unwrap();
    let kept = p.current().clone();
    // A center on the corner: no 32 px crop fits about it.
    let refused = p.apply(
        &[Change::Crop {
            cx: 0.0,
            cy: 0.0,
            w: 0.5,
            h: 0.5,
            angle: 30.0,
        }],
        Phase::End,
    );
    assert_eq!(refused, Err(Refused::CropDoesNotFit));
    assert_eq!(*p.current(), kept);
    // A crop that leaves the image shrinks about its center instead.
    p.apply(
        &[Change::Crop {
            cx: 0.5,
            cy: 0.5,
            w: 1.0,
            h: 1.0,
            angle: 10.0,
        }],
        Phase::End,
    )
    .unwrap();
    let c = &p.current().crop;
    assert!(c.w < 1.0 && c.h < 1.0 && c.angle == 10.0, "{c:?}");
}

#[test]
fn a_turn_keeps_the_region_and_undo_restores_it() {
    let mut p = photo();
    p.apply(
        &[Change::Crop {
            cx: 0.3,
            cy: 0.6,
            w: 0.4,
            h: 0.3,
            angle: 0.0,
        }],
        Phase::End,
    )
    .unwrap();
    let before = p.current().clone();
    for _ in 0..4 {
        p.apply(&[Change::Turn { clockwise: true }], Phase::End)
            .unwrap();
    }
    let c = &p.current().crop;
    assert_eq!(p.current().quarter_turns, 0);
    assert!(
        (c.cx - 0.3).abs() < 1e-6 && (c.cy - 0.6).abs() < 1e-6,
        "{c:?}"
    );
    for _ in 0..4 {
        assert!(p.undo());
    }
    assert_eq!(*p.current(), before);
}

#[test]
fn resets_keep_unknown_fields() {
    let mut p = photo();
    p.apply(&[Change::Exposure(1.0)], Phase::End).unwrap();
    // An edit loaded with fields this build does not know, at every level.
    let mut e = p.current().clone();
    e.unknown.insert("texture".into(), 12.into());
    e.white_balance
        .unknown
        .insert("picker".into(), "spot".into());
    e.crop.unknown.insert("aspect".into(), "3:2".into());
    let mut p = PhotoEdits::with_edit(RAW, (6000, 4000), 1, e);
    for c in [
        Change::Reset(Param::Crop),
        Change::Reset(Param::Orientation),
        Change::Reset(Param::WhiteBalance),
        Change::ResetAll,
    ] {
        p.apply(std::slice::from_ref(&c), Phase::End).unwrap();
        let e = p.current();
        assert_eq!(e.unknown["texture"], 12, "{c:?}");
        assert_eq!(e.white_balance.unknown["picker"], "spot", "{c:?}");
        assert_eq!(e.crop.unknown["aspect"], "3:2", "{c:?}");
    }
    assert!(!p.is_edited());
}
