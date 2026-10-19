//! The settings of stages 6 and 7 (DEV-01; the Basic panel, DEV-02) and the
//! per-photo base curve they apply. Until the contract's edit struct exists
//! (contract 0.5.0), the engine fills these from its own edit.

use genzo_color::tone::{
    CurveError, CurveInputs, CurveTable, ExposureRamp, choose_curve, total_baseline_exposure,
};
use genzo_imgio::raw::CameraColor;

/// The camera profile (DEV-01, *Data*).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Profile {
    /// The camera matrix plus the base curve (DEV-02).
    #[default]
    GenzoStandard,
    /// The matrix only: clamped at display white in stage 9.
    GenzoLinear,
    /// Non-RAW: display-referred already, so no base curve.
    Embedded,
}

/// What stages 6 and 7 apply: the Basic panel's tone and presence controls,
/// as stored (sliders −100 to 100, exposure in EV).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Look {
    pub profile: Profile,
    pub monochrome: bool,
    pub exposure: f32,
    pub contrast: f32,
    /// Highlights and Shadows need the base layer (local tone mapping), a
    /// later step; until then they do nothing.
    pub highlights: f32,
    pub shadows: f32,
    pub whites: f32,
    pub blacks: f32,
    pub vibrance: f32,
    pub saturation: f32,
}

impl Look {
    /// The defaults for a photo with `profile` (DEV-01).
    pub fn with_profile(profile: Profile) -> Self {
        Self {
            profile,
            ..Self::default()
        }
    }

    /// Whether stage 6 changes nothing: no tone control moved and no base
    /// curve.
    pub fn tone_is_identity(&self) -> bool {
        self.profile != Profile::GenzoStandard
            && [self.exposure, self.contrast, self.whites, self.blacks] == [0.0; 4]
    }

    /// Whether stage 7 changes nothing.
    pub fn color_is_identity(&self) -> bool {
        !self.monochrome && self.vibrance == 0.0 && self.saturation == 0.0
    }
}

/// The base curve of one photo (DEV-02): its curve table and what the ramp's
/// black needs. The table depends only on the file, so it is built once.
#[derive(Debug, Clone, PartialEq)]
pub struct BaseCurve {
    pub table: CurveTable,
    pub inputs: CurveInputs,
    /// The SDK's total BaselineExposure, in stops.
    pub baseline_exposure: f64,
    /// Why the file's own curve is not used, for the status bar.
    pub curve_error: Option<CurveError>,
}

impl BaseCurve {
    pub fn new(c: &CameraColor) -> Self {
        let inputs = CurveInputs {
            shadow_scale: c.shadow_scale,
            no_default_black: c.default_black_render == 1,
            scene_referred: c.colorimetric_reference == 0,
        };
        let (curve, curve_error) =
            choose_curve(c.profile_tone_curve.as_deref(), inputs.scene_referred);
        Self {
            table: CurveTable::new(&curve),
            inputs,
            baseline_exposure: total_baseline_exposure(
                c.baseline_exposure,
                c.baseline_exposure_offset,
            ),
            curve_error,
        }
    }

    /// The status bar's note when the file's curve can't be used (DEV-02).
    pub fn note(&self) -> Option<&'static str> {
        self.curve_error
            .map(|_| "The file's tone curve can't be read; using the default")
    }

    /// The exposure ramp at the user's `exposure` (EV): white 1, the black
    /// in Genzo's units (DEV-02, step 2).
    pub fn ramp(&self, exposure: f32) -> ExposureRamp {
        let black = self
            .inputs
            .black(self.baseline_exposure + f64::from(exposure));
        ExposureRamp::new(1.0, black, black)
    }
}
