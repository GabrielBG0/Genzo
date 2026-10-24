//! The edit struct (DEV-01, *Data*): one photo's develop settings, versioned
//! with serde from `schema_version` 1. The M2 catalog stores this same
//! struct; in M1 it lives in the engine's memory for the session.
//!
//! - Unknown fields are kept on load and written back, at every level. An
//!   edit of a newer schema is refused.
//! - Values outside their ranges are clamped ([`Edit::clamped`]); the engine
//!   clamps every delta and every loaded edit.
//! - An edit equal to its photo's defaults is unedited ([`Edit::is_edited`]).
//!   While white balance is as shot, Temp and Tint hold the as-shot values
//!   for the sliders and are left out of that comparison.
//!
//! Contract 0.5.0 exports it to TypeScript with the session commands.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// The schema this build writes. A change to a field or to the crop
/// coordinates bumps it.
pub const SCHEMA_VERSION: u32 = 1;

/// Fields this build does not know, kept as they were read.
pub type Unknown = Map<String, Value>;

/// The camera profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Profile {
    /// RAW: the camera matrix plus the base curve (DEV-02). The RAW default.
    GenzoStandard,
    /// RAW: the matrix only.
    GenzoLinear,
    /// Non-RAW, the only value there.
    Embedded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Treatment {
    #[default]
    Color,
    Monochrome,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum WhiteBalanceMode {
    #[default]
    AsShot,
    Custom,
}

/// White balance (DEV-01, *White balance: what Temp and Tint mean*). RAW:
/// `temp` in kelvin, `tint` on the DNG SDK's scale. Non-RAW: both are
/// relative shifts, 0 being the encoded white.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WhiteBalance {
    pub mode: WhiteBalanceMode,
    pub temp: f32,
    pub tint: f32,
    #[serde(flatten)]
    pub unknown: Unknown,
}

/// The crop on the oriented image, normalized 0 to 1 on each axis (DEV-01,
/// *Crop and rotation coordinates*): its center, its size as fractions of
/// the oriented image's, and the straighten angle in degrees, positive
/// clockwise. Whether it fits the image needs the image's size, so the
/// engine checks that (`geometry::fit`), not [`Edit::clamped`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Crop {
    pub cx: f32,
    pub cy: f32,
    pub w: f32,
    pub h: f32,
    pub angle: f32,
    #[serde(flatten)]
    pub unknown: Unknown,
}

impl Default for Crop {
    /// The whole image, straight.
    fn default() -> Self {
        Self {
            cx: 0.5,
            cy: 0.5,
            w: 1.0,
            h: 1.0,
            angle: 0.0,
            unknown: Unknown::new(),
        }
    }
}

/// What kind of photo an edit is for: its profile and white balance ranges
/// and defaults depend on it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PhotoKind {
    /// A RAW file, with its as-shot white balance as Temp and Tint.
    Raw {
        as_shot_temp: f32,
        as_shot_tint: f32,
    },
    NonRaw,
}

/// The temperature range of a RAW white balance, in kelvin.
pub const RAW_TEMP: (f32, f32) = (2000.0, 50000.0);
/// The tint range of a RAW white balance.
pub const RAW_TINT: (f32, f32) = (-150.0, 150.0);
/// The range of the sliders, and of a non-RAW Temp and Tint.
pub const SLIDER: (f32, f32) = (-100.0, 100.0);
/// The exposure range, in EV.
pub const EXPOSURE: (f32, f32) = (-5.0, 5.0);
/// The straighten angle's range, in degrees.
pub const ANGLE: (f32, f32) = (-45.0, 45.0);

/// Where a gesture is (DEV-01, *Inputs*): between *start* and *end* renders
/// are interactive, and the *end* records one history step (APP-10). A
/// delta without a gesture is an *end*.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Phase {
    Start,
    Update,
    End,
}

impl Phase {
    /// Whether the render is interactive.
    pub fn interactive(self) -> bool {
        self != Phase::End
    }
}

/// A field of the edit a reset can name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Param {
    Profile,
    Treatment,
    WhiteBalance,
    Exposure,
    Contrast,
    Highlights,
    Shadows,
    Whites,
    Blacks,
    Vibrance,
    Saturation,
    /// The crop and its angle.
    Crop,
    /// The quarter turns and the flip.
    Orientation,
}

impl Param {
    /// The control's name as the panels show it (APP-10's step labels).
    pub fn label(self) -> &'static str {
        match self {
            Self::Profile => "Profile",
            Self::Treatment => "Treatment",
            Self::WhiteBalance => "White balance",
            Self::Exposure => "Exposure",
            Self::Contrast => "Contrast",
            Self::Highlights => "Highlights",
            Self::Shadows => "Shadows",
            Self::Whites => "Whites",
            Self::Blacks => "Blacks",
            Self::Vibrance => "Vibrance",
            Self::Saturation => "Saturation",
            Self::Crop => "Crop",
            Self::Orientation => "Rotate",
        }
    }
}

/// One change an edit delta carries (DEV-01, *Inputs*: `ApplyEditDelta`).
/// The engine clamps every value, fits the crop to the image and refuses a
/// crop that cannot fit (DEV-01, *Crop*).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Change {
    Profile(Profile),
    Treatment(Treatment),
    /// Temp and Tint, a preset or a pick: the whole white balance.
    WhiteBalance {
        mode: WhiteBalanceMode,
        temp: f32,
        tint: f32,
    },
    Exposure(f32),
    Contrast(f32),
    Highlights(f32),
    Shadows(f32),
    Whites(f32),
    Blacks(f32),
    Vibrance(f32),
    Saturation(f32),
    /// The crop rectangle and its angle, on the oriented image.
    Crop {
        cx: f32,
        cy: f32,
        w: f32,
        h: f32,
        angle: f32,
    },
    /// A quarter turn, clockwise or not; the crop keeps its region.
    Turn {
        clockwise: bool,
    },
    /// A horizontal flip; the crop keeps its region.
    Flip,
    /// One field back to the photo's default.
    Reset(Param),
    /// The whole edit back to the photo's defaults.
    ResetAll,
}

impl Change {
    /// The field it changes, None for Reset all.
    pub fn param(&self) -> Option<Param> {
        Some(match self {
            Self::Profile(_) => Param::Profile,
            Self::Treatment(_) => Param::Treatment,
            Self::WhiteBalance { .. } => Param::WhiteBalance,
            Self::Exposure(_) => Param::Exposure,
            Self::Contrast(_) => Param::Contrast,
            Self::Highlights(_) => Param::Highlights,
            Self::Shadows(_) => Param::Shadows,
            Self::Whites(_) => Param::Whites,
            Self::Blacks(_) => Param::Blacks,
            Self::Vibrance(_) => Param::Vibrance,
            Self::Saturation(_) => Param::Saturation,
            Self::Crop { .. } => Param::Crop,
            Self::Turn { .. } | Self::Flip => Param::Orientation,
            Self::Reset(p) => *p,
            Self::ResetAll => return None,
        })
    }

    /// The history step's label (APP-10).
    pub fn label(&self) -> String {
        match self {
            Self::ResetAll => "Reset all".into(),
            Self::Reset(p) => format!("Reset {}", p.label()),
            c => c.param().expect("a field").label().into(),
        }
    }
}

/// Why an edit did not load.
#[derive(Debug)]
pub enum EditError {
    /// Not an edit: malformed, a field missing or of the wrong type.
    Json(serde_json::Error),
    /// Written by a newer build, with this schema version.
    Newer(u32),
}

impl std::fmt::Display for EditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Json(e) => write!(f, "not an edit: {e}"),
            Self::Newer(v) => write!(
                f,
                "edit schema {v} is newer than this build's ({SCHEMA_VERSION})"
            ),
        }
    }
}

impl std::error::Error for EditError {}

/// One photo's develop settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Edit {
    pub schema_version: u32,
    pub profile: Profile,
    pub treatment: Treatment,
    pub white_balance: WhiteBalance,
    pub exposure: f32,
    pub contrast: f32,
    pub highlights: f32,
    pub shadows: f32,
    pub whites: f32,
    pub blacks: f32,
    pub vibrance: f32,
    pub saturation: f32,
    pub crop: Crop,
    /// Clockwise, 0 to 3.
    pub quarter_turns: u8,
    pub flip_horizontal: bool,
    #[serde(flatten)]
    pub unknown: Unknown,
}

impl Edit {
    /// The defaults for a photo of `kind`: nothing moved, white balance as
    /// shot.
    pub fn defaults(kind: PhotoKind) -> Self {
        let (profile, temp, tint) = match kind {
            PhotoKind::Raw {
                as_shot_temp,
                as_shot_tint,
            } => (Profile::GenzoStandard, as_shot_temp, as_shot_tint),
            PhotoKind::NonRaw => (Profile::Embedded, 0.0, 0.0),
        };
        Self {
            schema_version: SCHEMA_VERSION,
            profile,
            treatment: Treatment::Color,
            white_balance: WhiteBalance {
                mode: WhiteBalanceMode::AsShot,
                temp,
                tint,
                unknown: Unknown::new(),
            },
            exposure: 0.0,
            contrast: 0.0,
            highlights: 0.0,
            shadows: 0.0,
            whites: 0.0,
            blacks: 0.0,
            vibrance: 0.0,
            saturation: 0.0,
            crop: Crop::default(),
            quarter_turns: 0,
            flip_horizontal: false,
            unknown: Unknown::new(),
        }
    }

    /// Reads an edit from JSON, then [`Edit::clamped`] for `kind`. An edit
    /// of a newer schema is refused: its known fields may mean something
    /// else there (a change to the crop coordinates bumps the version).
    pub fn from_json(text: &str, kind: PhotoKind) -> Result<Self, EditError> {
        let edit: Self = serde_json::from_str(text).map_err(EditError::Json)?;
        if edit.schema_version > SCHEMA_VERSION {
            return Err(EditError::Newer(edit.schema_version));
        }
        Ok(edit.clamped(kind))
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("an edit always serializes")
    }

    /// The edit with every value in its range for a photo of `kind`:
    /// - a profile the kind cannot use becomes its default;
    /// - while white balance is as shot, Temp and Tint are the as-shot
    ///   values, whatever was stored;
    /// - NaN becomes the default; numbers outside their ranges are clamped;
    /// - quarter turns wrap to 0 to 3; the crop's center and size stay
    ///   within 0 to 1 (its size above 0), the angle within ±45°.
    pub fn clamped(mut self, kind: PhotoKind) -> Self {
        let defaults = Self::defaults(kind);
        let raw = matches!(kind, PhotoKind::Raw { .. });
        if raw == (self.profile == Profile::Embedded) {
            self.profile = defaults.profile;
        }
        let wb = &mut self.white_balance;
        if wb.mode == WhiteBalanceMode::AsShot {
            wb.temp = defaults.white_balance.temp;
            wb.tint = defaults.white_balance.tint;
        }
        let (temp, tint) = if raw {
            (RAW_TEMP, RAW_TINT)
        } else {
            (SLIDER, SLIDER)
        };
        wb.temp = clamp(wb.temp, temp, defaults.white_balance.temp);
        wb.tint = clamp(wb.tint, tint, defaults.white_balance.tint);
        self.exposure = clamp(self.exposure, EXPOSURE, 0.0);
        for v in [
            &mut self.contrast,
            &mut self.highlights,
            &mut self.shadows,
            &mut self.whites,
            &mut self.blacks,
            &mut self.vibrance,
            &mut self.saturation,
        ] {
            *v = clamp(*v, SLIDER, 0.0);
        }
        let c = &mut self.crop;
        let full = Crop::default();
        c.cx = clamp(c.cx, (0.0, 1.0), full.cx);
        c.cy = clamp(c.cy, (0.0, 1.0), full.cy);
        c.w = clamp(c.w, (f32::MIN_POSITIVE, 1.0), full.w);
        c.h = clamp(c.h, (f32::MIN_POSITIVE, 1.0), full.h);
        c.angle = clamp(c.angle, ANGLE, 0.0);
        self.quarter_turns %= 4;
        self
    }

    /// Whether the edit differs from `defaults` (the photo's, from
    /// [`Edit::defaults`]): the filmstrip's dot and the panels' dots. Temp
    /// and Tint count only when white balance is custom; unknown fields and
    /// the schema version never count.
    pub fn is_edited(&self, defaults: &Self) -> bool {
        let wb = |e: &Self| {
            let w = &e.white_balance;
            match w.mode {
                WhiteBalanceMode::AsShot => (w.mode, None),
                WhiteBalanceMode::Custom => (w.mode, Some((w.temp, w.tint))),
            }
        };
        let crop = |e: &Self| {
            let c = &e.crop;
            [c.cx, c.cy, c.w, c.h, c.angle]
        };
        let values = |e: &Self| {
            [
                e.exposure,
                e.contrast,
                e.highlights,
                e.shadows,
                e.whites,
                e.blacks,
                e.vibrance,
                e.saturation,
            ]
        };
        self.profile != defaults.profile
            || self.treatment != defaults.treatment
            || wb(self) != wb(defaults)
            || values(self) != values(defaults)
            || crop(self) != crop(defaults)
            || self.quarter_turns != defaults.quarter_turns
            || self.flip_horizontal != defaults.flip_horizontal
    }
}

/// `v` within `range`, or `default` when it is NaN.
fn clamp(v: f32, range: (f32, f32), default: f32) -> f32 {
    if v.is_nan() {
        default
    } else {
        v.clamp(range.0, range.1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RAW: PhotoKind = PhotoKind::Raw {
        as_shot_temp: 5120.0,
        as_shot_tint: 7.5,
    };

    #[test]
    fn defaults_are_unedited_and_round_trip() {
        for kind in [RAW, PhotoKind::NonRaw] {
            let d = Edit::defaults(kind);
            assert!(!d.is_edited(&d));
            let back = Edit::from_json(&d.to_json(), kind).unwrap();
            assert_eq!(back, d);
        }
        assert_eq!(Edit::defaults(RAW).profile, Profile::GenzoStandard);
        assert_eq!(Edit::defaults(PhotoKind::NonRaw).profile, Profile::Embedded);
    }

    #[test]
    fn the_stored_form() {
        let json = Edit::defaults(PhotoKind::NonRaw).to_json();
        assert_eq!(
            json,
            r#"{"schema_version":1,"profile":"Embedded","treatment":"Color","white_balance":{"mode":"AsShot","temp":0.0,"tint":0.0},"exposure":0.0,"contrast":0.0,"highlights":0.0,"shadows":0.0,"whites":0.0,"blacks":0.0,"vibrance":0.0,"saturation":0.0,"crop":{"cx":0.5,"cy":0.5,"w":1.0,"h":1.0,"angle":0.0},"quarter_turns":0,"flip_horizontal":false}"#
        );
    }

    #[test]
    fn unknown_fields_are_kept_at_every_level() {
        let mut v: Value = serde_json::from_str(&Edit::defaults(RAW).to_json()).unwrap();
        v["texture"] = 12.into();
        v["mask"] = serde_json::json!({"kind": "brush", "points": [1, 2]});
        v["white_balance"]["picker"] = "spot".into();
        v["crop"]["aspect"] = "3:2".into();
        let e = Edit::from_json(&v.to_string(), RAW).unwrap();
        assert_eq!(e.unknown["texture"], 12);
        assert_eq!(e.white_balance.unknown["picker"], "spot");
        assert_eq!(e.crop.unknown["aspect"], "3:2");
        // Written back as read, and still unedited: unknown fields never
        // count.
        let back: Value = serde_json::from_str(&e.to_json()).unwrap();
        assert_eq!(back, v);
        assert!(!e.is_edited(&Edit::defaults(RAW)));
    }

    #[test]
    fn a_newer_schema_is_refused() {
        let mut v: Value = serde_json::from_str(&Edit::defaults(RAW).to_json()).unwrap();
        v["schema_version"] = (SCHEMA_VERSION + 1).into();
        assert!(matches!(
            Edit::from_json(&v.to_string(), RAW),
            Err(EditError::Newer(2))
        ));
    }

    #[test]
    fn missing_fields_are_an_error() {
        let mut v: Value = serde_json::from_str(&Edit::defaults(RAW).to_json()).unwrap();
        v.as_object_mut().unwrap().remove("exposure");
        assert!(Edit::from_json(&v.to_string(), RAW).is_err());
    }

    #[test]
    fn as_shot_ignores_stored_temp_and_tint() {
        let mut e = Edit::defaults(RAW);
        e.white_balance.temp = 3000.0;
        e.white_balance.tint = -40.0;
        // Recomputed on load, and not an edit.
        assert!(!e.is_edited(&Edit::defaults(RAW)));
        let e = e.clamped(RAW);
        assert_eq!((e.white_balance.temp, e.white_balance.tint), (5120.0, 7.5));
        // Custom counts, even at the as-shot values.
        let mut c = e.clone();
        c.white_balance.mode = WhiteBalanceMode::Custom;
        assert!(c.is_edited(&Edit::defaults(RAW)));
    }

    #[test]
    fn values_are_clamped_to_their_ranges() {
        let mut e = Edit::defaults(RAW);
        e.profile = Profile::Embedded;
        e.white_balance.mode = WhiteBalanceMode::Custom;
        e.white_balance.temp = 900.0;
        e.white_balance.tint = 400.0;
        e.exposure = 9.0;
        e.contrast = -300.0;
        e.saturation = f32::NAN;
        e.crop.w = 0.0;
        e.crop.cx = 1.5;
        e.crop.angle = -60.0;
        e.quarter_turns = 6;
        let e = e.clamped(RAW);
        assert_eq!(e.profile, Profile::GenzoStandard);
        assert_eq!(
            (e.white_balance.temp, e.white_balance.tint),
            (2000.0, 150.0)
        );
        assert_eq!((e.exposure, e.contrast, e.saturation), (5.0, -100.0, 0.0));
        assert!(e.crop.w > 0.0 && e.crop.cx == 1.0 && e.crop.angle == -45.0);
        assert_eq!(e.quarter_turns, 2);

        let mut n = Edit::defaults(PhotoKind::NonRaw);
        n.profile = Profile::GenzoLinear;
        n.white_balance.mode = WhiteBalanceMode::Custom;
        n.white_balance.temp = 5000.0;
        let n = n.clamped(PhotoKind::NonRaw);
        assert_eq!(n.profile, Profile::Embedded);
        assert_eq!(n.white_balance.temp, 100.0);
    }

    #[test]
    fn any_moved_control_is_an_edit() {
        let d = Edit::defaults(RAW);
        let moved: [fn(&mut Edit); 9] = [
            |e| e.profile = Profile::GenzoLinear,
            |e| e.treatment = Treatment::Monochrome,
            |e| e.exposure = 0.01,
            |e| e.blacks = -1.0,
            |e| e.saturation = 1.0,
            |e| e.crop.angle = 0.5,
            |e| e.crop.w = 0.9,
            |e| e.quarter_turns = 1,
            |e| e.flip_horizontal = true,
        ];
        for (i, f) in moved.iter().enumerate() {
            let mut e = d.clone();
            f(&mut e);
            assert!(e.is_edited(&d), "change {i}");
        }
    }

    /// Every f32 survives the JSON round trip bit for bit, through the
    /// flattened structs too: the edited dots and the history compare exact
    /// values, and M2 stores this JSON.
    #[test]
    fn floats_round_trip_bit_for_bit() {
        let mut state = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let special = [
            0.1f32,
            1.0 / 3.0,
            5120.37,
            1e-7,
            -0.0,
            49_999.996,
            f32::MIN_POSITIVE,
        ];
        let fields = |e: &Edit| {
            let w = &e.white_balance;
            let c = &e.crop;
            [
                w.temp,
                w.tint,
                e.exposure,
                e.contrast,
                e.highlights,
                e.shadows,
                e.whites,
                e.blacks,
                e.vibrance,
                e.saturation,
                c.cx,
                c.cy,
                c.w,
                c.h,
                c.angle,
            ]
        };
        for i in 0..20_000 {
            let mut e = Edit::defaults(RAW);
            e.white_balance.mode = WhiteBalanceMode::Custom;
            let w = &mut e.white_balance;
            let c = &mut e.crop;
            let values = [
                &mut w.temp,
                &mut w.tint,
                &mut e.exposure,
                &mut e.contrast,
                &mut e.highlights,
                &mut e.shadows,
                &mut e.whites,
                &mut e.blacks,
                &mut e.vibrance,
                &mut e.saturation,
                &mut c.cx,
                &mut c.cy,
                &mut c.w,
                &mut c.h,
                &mut c.angle,
            ];
            for (k, v) in values.into_iter().enumerate() {
                *v = if i < special.len() {
                    special[(i + k) % special.len()]
                } else {
                    // Any finite f32: random bits, NaN and infinity skipped.
                    loop {
                        let f = f32::from_bits(next() as u32);
                        if f.is_finite() {
                            break f;
                        }
                    }
                };
            }
            let back: Edit = serde_json::from_str(&e.to_json()).unwrap();
            let (a, b) = (fields(&e), fields(&back));
            for (k, (x, y)) in a.iter().zip(&b).enumerate() {
                assert_eq!(x.to_bits(), y.to_bits(), "field {k}: {x:e} came back {y:e}");
            }
        }
    }
}
