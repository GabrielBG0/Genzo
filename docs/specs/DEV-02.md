# DEV-02 · Basic panel (M1 part)

The Basic panel is the first panel in Develop: profile, treatment, white balance, and the tone and presence sliders. This spec covers what M1 builds:

- the panel and its controls (screen D1);
- the Genzo Standard profile's base tone curve, which DEV-01 leaves to this spec;
- the white balance presets and the picker (W).

Sources: the spec's feature table (revision 89: "Camera profile selection (matrix plus base tone curve), white balance with picker", "Exposure, contrast, highlights, shadows, whites, blacks, vibrance, saturation", "Color or black and white treatment, edited markers"), the Decided table in `docs/development-plan.md` ("Camera profiles in M1"), screen D1 and the Slider row component (C6) of the UI handoff, and `docs/specs/DEV-01.md`. Where this spec and the technical spec disagree, the technical spec wins and this file is corrected.

**What DEV-01 owns, not this spec:** every edit parameter's range, unit and default (DEV-01, *Data*), and the math of exposure, Highlights, Shadows, Whites, Blacks, Contrast, Vibrance, Saturation and B&W (DEV-01, stages 6 and 7). This spec says how the panel shows and changes them.

**Out of scope for M1**, with their milestone:
- Texture, Clarity, Dehaze (M3)
- Auto tone (Ctrl U) and auto white balance (Ctrl Shift U) (M4)
- profiles other than Genzo Standard and Genzo Linear: DCP files, the hue and saturation tables embedded in DNG (not planned in M1, see *Genzo Standard*)
- the per-color B&W mix (M3)
- clipping preview while Alt-dragging a slider: DEV-03 owns it

## Behavior

### The panel

From top to bottom, as screen D1 lays it out:

1. **Header:** "BASIC", the edited dot (DEV-13) and the chevron. The handoff's Auto button is not shown in M1.
2. **Profile row:** a dropdown and the Color / B&W pill toggle.
3. **White balance:** the preset dropdown and the picker button, then Temp and Tint.
4. **Tone:** Exposure, Contrast, Highlights, Shadows, Whites, Blacks.
5. **Presence:** Vibrance and Saturation. The handoff's Texture, Clarity and Dehaze rows are not shown in M1. They appear in M3; a control that does nothing is never shown.

### Profile

- **The dropdown** lists Genzo Standard and Genzo Linear for RAW files. The handoff's "Camera Standard" label is a placeholder; the names are the plan's (Decided, "Camera profiles in M1").
- **Non-RAW files** show "Embedded", and the dropdown is disabled. It is the only value there (DEV-01, *Data*).
- Choosing a profile is one history step. It re-runs from the base curve (DEV-01, *Re-runs*).

### Genzo Standard: the base tone curve

Genzo Standard is the Adobe DNG SDK 1.4's default rendering (`dng_render.cpp`, `dng_reference.cpp`), ported step by step as stage 4 was. Two parts: the SDK's exposure ramp, which sets the black and white points, and a tone curve, applied so that hue holds. The curve is the file's own ProfileToneCurve when it has a valid one, and the SDK's default (`dng_tone_curve_acr3_default`) otherwise, as the SDK chooses (owner, 2026-10-19). With it, a RAW file looks as Adobe's own renderer shows it with the default settings and no hue or saturation tables.

It runs as the last step of stage 6, after exposure, Highlights, Shadows, Whites, Blacks and Contrast (DEV-01, *Stage 6*). It works per channel, not on luminance, unlike the steps before it. For each pixel:

1. **Into linear ProPhoto RGB.** The SDK applies its curve in linear ProPhoto RGB with a D50 white, and the result depends on the primaries. So the working-space value goes through one 3 × 3 matrix: AP1 to XYZ (D65), Bradford D65 to D50 (the same linearized transform as stage 4), XYZ to ProPhoto. These are the inverse of the steps stage 4 took from the SDK's connection space, so the pixel lands on the values the SDK holds at this point.
2. **The exposure ramp**, per channel (`dng_function_exposure_ramp`):
   - `y = (x − B) / (W − B)`, clipped to 0 to 1.
   - **The white point `W` is 1.** The SDK puts BaselineExposure and the user's exposure into its white point, as `1 / 2^E`. Genzo has already applied both as gains (BaselineExposure in stage 4, exposure in stage 6), so the same white is 1 here.
   - **The black point `B`** is the SDK's default black, `0.001 × 5 × ShadowScale`, carried into the same units: `B = 0.005 × ShadowScale × 2^(BaselineExposure + exposure)`, where BaselineExposure is the SDK's total, offset included (DEV-01, stage 0). The SDK's black is in scene units, before any exposure gain, and this keeps it there. Without the factor, the black of the iPhone 13 Pro Max file (+2.50 EV) would be 5.7 times too low. `B` is limited to `0.99 × W`, as in the SDK.
   - **The toe.** Near `B` the ramp is a quadratic of radius `r = min(0.5 × B, (W − B) / 16)`: below `B − r` it is 0, within `r` of `B` it is `(x − (B − r))² / (4 r (W − B))`, above it the line. This is the SDK's code with its `minBlack` equal to `B`.
   - ShadowScale is the DNG tag (default 1). Only DNGs carry it. In the corpus it is 1, except Adobe's conversion of the Canon CRAW file (0.906). The 5 is the SDK's default Shadows value.
   - **DefaultBlackRender** (DNG tag 51110): when the profile sets it to 1 (None), `B` is 0 and the toe goes with it, as in the SDK. No corpus file has it.
   - **ColorimetricReference** (DNG tag 50879): a DNG that says it is output-referred (1) gets no black, and the identity curve unless its profile has a valid ProfileToneCurve, which still applies, as in the SDK. No corpus file has it; ProRAW leaves the tag out, which means scene-referred.
3. **The tone curve, keeping hue** (`RefBaselineRGBTone`).
   - **Which curve.** The file's ProfileToneCurve (DNG tag 50940, in the profile of the main image) when it is valid, else the SDK's ACR3 table of 1025 entries. Valid is the SDK's rule: an even count of floats, at least 2 points, every coordinate in 0 to 1, and x strictly increasing. Genzo adds a cap of 4096 points; past it, or when the tag is invalid, the ACR3 curve is used and the status bar says "The file's tone curve can't be read; using the default". In the corpus only the ProRAW files (iPhone 12 Pro, 16 Pro) carry one, of 257 points. The Adobe Standard profiles in Adobe DNG Converter's references have none.
   - **The file's curve is a spline**, solved as the SDK solves it (`dng_spline_solver`: C1 and C2 continuous, slopes from the weighted neighbor slopes, then the tridiagonal solve), and evaluated per segment as a cubic Bezier (`EvaluateSplineSegment`). Below the first point and above the last it is constant.
   - **One table for both curves.** As the SDK does (`dng_1d_table`, not subsampled), the curve is evaluated at the 4097 points `i / 4096` in f64, stored as f32, and interpolated linearly. The table is built on the CPU once per photo and uploaded. For the ACR3 curve this gives the same values as interpolating its 1025 entries, since every one of them is a table point.
   - **RGBTone.** Each pixel's largest and smallest channels go through the curve; the middle one is placed at the same fraction between them as before: `mid' = min' + (max' − min') × (mid − min) / (max − min)`. Two equal channels go through the curve; three equal channels stay neutral.
4. **Back to AP1** with the inverse matrix of step 1.

The output is display-referred: 0 to 1 in each channel. With the ACR3 curve, middle gray (0.18 after exposure) lands at 0.379, sRGB code 166: the ramp's default black lowers it to 0.176 first, and the curve alone maps 0.18 to 0.388. Apple's curves are much darker in the midtones: alone, they map 0.18 to 0.122 (iPhone 12 Pro) and 0.125 (16 Pro), about sRGB code 98. Stage 7 and the output transform (stage 9) follow, as DEV-01 describes. Gamut compression still runs for RAW photos, though nothing past display white reaches it.

**Where Genzo differs from the SDK**, on purpose:
- **Negative exposure.** For an exposure below 0 EV in total, the SDK keeps its white at 1 and darkens its tone curve with a quadratic (`dng_function_exposure_tone`), which is not a gain. Genzo applies negative exposure as a gain, like positive exposure (DEV-01 criterion 12). The two agree only at a total exposure of 0 EV or more.
- **Values past white and below 0** are kept through stage 6 in Genzo, and the SDK clips them to 0 to 1 before its ramp. The ramp maps them to 0 and 1 anyway, so the result is the same.
- **Highlight desaturation** (DEV-01, stage 4) is Genzo's. The SDK has none, so pixels near the clip level differ.
- **The hue and saturation tables are ignored:** ProfileHueSatMap and ProfileLookTable. Adobe DNG Converter's references carry them (the Adobe Standard profile), and the SDK would use them. Reading them is a later profile, not part of Genzo Standard. A file's ProfileToneCurve is used, as the SDK uses it.

**Licensing.** The table and the RGBTone method are Adobe's code from the DNG SDK copy in gopro/gpr, under BSD-3-Clause, as `color::camera` is. The ported file carries the notice, and `docs/licenses.md` lists it.

### Genzo Linear

No ramp and no curve: the working-space values go on to stage 7 unchanged. Display white is scene white after exposure, and stage 9 clamps what lies past it (DEV-01). It is meant as a flat start for the M3 tone curve.

### Treatment: Color or B&W

- The pill toggle and V switch between Color and B&W. Each switch is one history step.
- **B&W** shows the luminance of the toned image (DEV-01, *Stage 7*). Vibrance and Saturation are hidden, since they do nothing there; their values are kept and come back with Color.
- B&W works with every profile, Embedded included.

### White balance

**The preset dropdown.** For RAW files:

| Preset | Defined as | Notes |
|---|---|---|
| As Shot | the file's white (DEV-01, *White balance*) | `mode: AsShot` |
| Daylight | CIE D55 (x 0.33242, y 0.34743) | on the daylight locus |
| Cloudy | CIE D65 (x 0.31271, y 0.32902) | 6504 K (DEV-01 criterion 10) |
| Shade | CIE D75 (x 0.29902, y 0.31485) | |
| Tungsten | CIE illuminant A (x 0.44757, y 0.40745) | 2856 K, tint 0 |
| Fluorescent | CIE F2, cool white (x 0.37208, y 0.37529) | |
| Flash | 5500 K on the Planckian locus | tint 0 |
| Custom | any other Temp and Tint | shown, not chosen |

- **Choosing a preset** sets `mode: Custom` and the preset's Temp and Tint, computed from its xy with the code stage 4 uses (`color::camera`, Robertson temperature, `Duv = −tint / 3000`). It is one history step.
- **What the dropdown shows:** "As Shot" in `AsShot` mode. In `Custom` mode, the name of the preset whose Temp and Tint equal the edit's, rounded as the sliders show them (1 K, 1 tint); "Custom" otherwise. The edit stores no preset name, so a preset is recognized by its values.
- The illuminants are those DEV-01's stage 4 table names, so a preset's name and its light agree. The xy values are the CIE's 2° observer values.

**For non-RAW files**, the dropdown has As Shot and Custom only, since Temp and Tint are relative shifts there (DEV-01, *White balance*).

**Temp and Tint sliders.**
- **RAW Temp** runs from 2000 to 50,000 K, with positions linear in mireds (1,000,000 / K): `p = (1/2000 − 1/K) / (1/2000 − 1/50000)`. Equal steps of the slider are then about equal shifts of color, and 5000 K sits at 62.5 %.
- **RAW Tint** runs from −150 to +150, linear.
- **Non-RAW Temp and Tint** run from −100 to +100, linear.
- **The tracks are color gradients** (the handoff's Slider row note): Temp from blue on the left to yellow on the right, Tint from green on the left to magenta on the right. Their colors come from the token set.
- Moving either slider sets `mode: Custom`. Its first value is the as-shot one, so it starts where As Shot was.
- **A monochrome sensor** has no white balance: the whole white balance group is disabled, with the tooltip "This camera records no color" (DEV-01, *Edge cases*).

**The picker (W).**
1. W, or the picker button, turns the picker on. The cursor becomes a crosshair over the photo. W again or Esc turns it off and changes nothing.
2. A click on the photo sends `PickWhiteBalance` with the point, in the shown image's normalized coordinates (after geometry, as the crop uses: DEV-01, *Crop*).
3. The engine samples the area around the point and makes it neutral:
   - **RAW:** it averages a 5 × 5 texel area of pyramid level 1 (about 10 × 10 sensor pixels), mapping the point back through the geometry. It takes the average back to white balanced camera values through the inverse of stage 4's matrix and gain, then to the camera's own values through the white balance multipliers. That camera neutral gives a white point through the same neutral-to-xy solve as As Shot (`NeutralToXY`), and so Temp and Tint.
   - **Non-RAW:** it averages the same area of the decoded image in the working space and finds its xy. Temp is the shift whose white (`153.8 − temp` mired, DEV-01) lies nearest that xy along the locus, and Tint is the remaining distance as `−3000 × Duv`.
   - **It refuses** an area where any sample is within 5 % of the clip level, or whose luminance is below 0.5 % of white. The edit does not change, and the status bar says "Can't set white balance here: the area is clipped" or "too dark".
   - Values past the sliders' ranges are clamped to them.
4. A successful pick sets `mode: Custom` with the new Temp and Tint, records one history step and renders. The picker then turns itself off.

### Sliders

Every slider in the panel is a Slider row (handoff component C6): a label column of 76 px, the track, and a value column of 42 px.

**Shown value:**

| Control | Shown as | Examples |
|---|---|---|
| Temp (RAW) | whole kelvin | 5400 |
| Temp (non-RAW), Tint | whole number, signed | +4, −12, 0 |
| Exposure | two decimals, signed | +0.35, −1.20, 0.00 |
| Contrast, Highlights, Shadows, Whites, Blacks, Vibrance, Saturation | whole number, signed | +12, −38, 0 |

Values are stored as f32 (DEV-01). The display rounds; it never changes the stored value.

**Changing a value:**
- **Drag** the handle, or press on the track to jump there and keep dragging. Pointer down sends the gesture *start*, each move an *update*, and pointer up the *end* (DEV-01, *Inputs*). Updates go once per animation frame at most; the engine coalesces the rest.
- **Keyboard:** a focused slider moves by its step with the left and right arrow keys, and by its large step with Shift. A held key is one gesture: *start* on the first key down, *end* on key up.

  | Control | Step | Shift step |
  |---|---|---|
  | Temp (RAW) | 50 K | 500 K |
  | Exposure | 0.05 EV | 0.25 EV |
  | the others | 1 | 10 |

- **Type a value:** a click on the value turns it into a field. Enter applies (one history step), Esc cancels. A value out of range is clamped to it.
- **Reset:** a double-click on the label or the handle resets that slider to its default; for Temp or Tint, it resets white balance to As Shot. A double-click on a group label ("Tone", "Presence", "White balance") resets the group. The panel's own reset is "Reset Basic panel" in the slider context menu (APP-06). Each reset is one history step.
- **Optimistic values:** the slider shows its own value at once. The engine's edit, which comes back with the render, replaces it, which matters only when the engine clamped it.

**Edited state** (DEV-13, the technical spec's "edited markers"):
- A slider whose value differs from its default is shown bright, one at its default dim. Temp and Tint are bright when `mode` is `Custom`, whatever their values (DEV-01: As Shot is unedited).
- The header's blue dot shows when anything in the panel differs from its default: profile, treatment, white balance or a slider.
- "Default" is DEV-01's default for this file.

### History step names

APP-10's History list names steps after what changed:
- one control: the control and its new value as shown, "Exposure +0.35" or "Temp 5400";
- a preset: "White balance: Daylight"; the picker: "White balance: picked";
- the profile and treatment: "Profile: Genzo Linear", "Treatment: B&W";
- resets: "Reset Highlights", "Reset Tone", "Reset Basic panel".

## Inputs and shortcuts

Every command is in the shortcut registry, with its shortcut shown in its tooltip and menu item.

| Command | Shortcut | Notes |
|---|---|---|
| Toggle Color / B&W | V | Develop only |
| White balance picker | W | toggles; Esc cancels |
| Reset Basic panel | none | slider context menu (APP-06), Develop menu |
| Nudge a focused slider | ← →, Shift ← → | |

Ctrl U and Ctrl Shift U are not registered until their commands exist (M4).

**Contract** (M1, contract 0.5.0):
- `ApplyEditDelta` and `ResetParam`, as DEV-01 describes, for every control here.
- `PickWhiteBalance { photo, point }`, new: the engine samples, applies and records the step. It returns the new edit, or the reason it refused.

## States

| State | What the panel shows | What the user can do |
|---|---|---|
| **No photo selected** | Every control disabled, values blank | Nothing |
| **Opening** (the embedded preview is shown) | The photo's values | Everything. The first render applies them (DEV-01) |
| **Ready** | The edit's values | Everything |
| **RAW, as-shot white unavailable** (no as-shot data, D55 used) | As Shot with D55's Temp and Tint; the status bar says so (DEV-01) | Everything |
| **Monochrome sensor** | The white balance group disabled | Everything else |
| **Non-RAW** | Profile "Embedded", disabled; relative Temp and Tint; two white balance entries | Everything else |
| **Unsupported or damaged file** | Every control disabled; the status bar gives the reason (DEV-01) | Nothing in this panel |
| **Picker on** | The picker button pressed; a crosshair over the photo | Click the photo, Esc, W |
| **Pick refused** | Unchanged; the reason in the status bar | Pick again |
| **Software adapter, rendering** | As Ready. Sliders keep their optimistic values while frames lag | Everything |

## Data

DEV-02 adds no fields. It reads and writes the edit struct in DEV-01, *Data*: `profile`, `treatment`, `white_balance` (`mode`, `temp`, `tint`), `exposure`, `contrast`, `highlights`, `shadows`, `whites`, `blacks`, `vibrance`, `saturation`.

The decoder carries four more DNG tags for the base curve, in the color data of DEV-01's stage 0: ShadowScale (50739, default 1), ProfileToneCurve (50940, the points as read, or absent), DefaultBlackRender (51110, default 0) and ColorimetricReference (50879, default 0).

The white balance presets are constants in `color` (their xy), not stored.

## Edge cases

- **A preset that matches As Shot**: the dropdown says "As Shot" while `mode` is `AsShot`. Choosing the preset with the same values sets `Custom`, and the dropdown then names the preset.
- **Two presets with the same rounded values**: cannot happen. The table's values differ by at least 100 K or 3 tint.
- **Temp past the slider's range after a pick** (light below 2000 K): clamped to 2000 K, and the photo stays slightly warm.
- **BaselineExposure below 0** (Pentax K10D: −0.5 EV, iPhone 12 Pro: −0.3 EV): Genzo's gain lowers the black with it, so the black stays at the SDK's scene level. The SDK instead darkens its curve (see *Negative exposure*), so at 0 EV these files do not match it.
- **A very dark or bright exposure:** `B` grows with the exposure gain, and is limited to 0.99 × W. At +5 EV and +2.50 EV BaselineExposure, `B` is 0.905, so almost everything is white; that is the SDK's behavior.
- **A file's curve that is not monotonic, or a spline that overshoots 0 to 1** between its points: Genzo applies it as the SDK does, without fixing it. The SDK's validity rule does not ask for monotonic y values. Both ProRAW curves in the corpus are monotonic and stay within 0 to 1 once solved.
- **A ProfileToneCurve with a huge point count** (a damaged or hostile file): past 4096 points the decoder does not read it, and the ACR3 curve is used (see *Which curve*). The decode fuzz target covers the tag.
- **Negative working-space values** (noise, colors outside ProPhoto): below the ramp's black, so 0. A color outside ProPhoto but inside AP1 loses that part here; AP1 reaches slightly past ProPhoto near its red primary.
- **Switching profile** keeps every slider value; only the base curve changes.
- **B&W, then Color**: Vibrance and Saturation come back with their values.
- **Typing "5400K" or "+0,35"**: unit suffixes are ignored, a comma is a decimal point; anything else that does not parse is refused and the old value stays.

## Acceptance criteria

1. **The curves are Adobe's.**
   - The `color` crate's ACR3 table equals the SDK's `dng_tone_curve_acr3_default` table, all 1025 values as f32. The curve evaluated on a grid of 10,000 points matches the SDK's `Evaluate` within 1e-6.
   - Genzo's spline, solved from the two ProRAW curves and from synthetic point sets (2 points, 3 points, uneven spacing, a non-monotonic set), matches the SDK's `dng_spline_solver::Evaluate` within 1e-9 on the same grid.
   - The 4097-entry table equals the SDK's `fToneCurve` table as f32, for ACR3 and both ProRAW curves, at exposure 0.
   - Invalid curves (odd count, one point, x not increasing, a value outside 0 to 1, 4097 points) fall back to ACR3.

   *Check:* `color` tests against values the probe prints.
2. **The ramp is Adobe's.** `dng_function_exposure_ramp` and Genzo's ramp agree within 1e-9 on a grid of inputs, for blacks of 0, 1e-4, 0.005, 0.05 and 0.5 and white 1, toe included. *Check:* a `color` test against the probe.
3. **RGBTone is Adobe's.** Genzo's CPU reference matches `RefBaselineRGBTone` within 1e-6 on 100,000 random triples, with ties (two or three equal channels), zeros and ones among them. The GPU matches the CPU reference within one f16 step. *Check:* `color` and pipeline tests.
4. **Properties.** The ACR3 curve is monotonic, maps 0 to 0 and 1 to 1, and maps 0.18 to 0.388 ± 0.001. Neutral pixels stay neutral (OkLCh chroma ≤ 0.001 after stage 9, before quantizing). The fraction (mid − min) / (max − min) in ProPhoto is the same before and after, within 1e-5, where max − min is at least 0.01 before and after (below that, the f32 arithmetic the SDK uses rounds by more). Middle gray through the whole step (ramp and curve) is 0.379 ± 0.001, sRGB code 166. *Check:* `color` tests.
5. **Whole render against the SDK.** The probe renders five corpus DNGs with no hue or saturation tables through `dng_render`: four on the ACR3 curve (Leica Q2, Pentax K10D, the Halide iPhone 16 Pro Bayer DNG, Galaxy S23 Ultra) and the iPhone 12 Pro ProRAW on its own ProfileToneCurve. The iPhone 16 Pro ProRAW is left out: its tiles are JPEG XL, which SDK 1.4 cannot decode. Its curve is still checked by criterion 1. Settings: the SDK's curve choice left as it is, Shadows 5, exposure +1 EV for every file and 0 EV for those whose BaselineExposure is 0 or more (not the Pentax or the iPhone 12 Pro), final space linear ProPhoto RGB as floats. Genzo renders the same files to the end of stage 6 and converts to the same space. Compared as 64 px block means over the default crop:
   - mean difference ≤ 0.5 % of white and the 99th percentile ≤ 2 % (the demosaics differ, and Genzo desaturates near the clip);
   - measured on the RTX 3060 (2026-10-19, `pipeline/tests/render_sdk.rs`, the SDK's OpcodeList3 cleared since Genzo skips lens opcodes in M1):

     | Render | Mean | 99th percentile | Worst block |
     |---|---|---|---|
     | Leica Q2, 0 EV | 0.038 % | 0.25 % | 12.3 % |
     | Leica Q2, +1 EV | 0.074 % | 0.50 % | 16.7 % |
     | Pentax K10D, +1 EV | 0.016 % | 0.046 % | 0.072 % |
     | Halide iPhone 16 Pro, 0 EV | 0.028 % | 0.28 % | 0.54 % |
     | Halide iPhone 16 Pro, +1 EV | 0.051 % | 0.42 % | 0.73 % |
     | Galaxy S23 Ultra, 0 EV | 0.018 % | 0.037 % | 0.34 % |
     | Galaxy S23 Ultra, +1 EV | 0.027 % | 0.038 % | 0.041 % |
     | iPhone 12 Pro ProRAW (its own curve), +1 EV | 0.022 % | 0.054 % | 0.062 % |

     The Leica's worst blocks are saturated reds whose camera red channel reaches the clip level: Genzo's highlight desaturation (DEV-01, stage 4) moves them toward neutral, and the SDK does not. With it switched off for a check, the worst block is 1.4 % at 0 EV and 2.0 % at +1 EV.

   Negative exposure is not compared (see *Where Genzo differs*). *Check:* a corpus test (local), with the probe extended by a `render_probe`.
6. **Golden images.** Stage 6 with Genzo Standard, and with Genzo Linear, on the CC0 set, which includes the iPhone 12 Pro ProRAW file and so its own curve: DEV-01 criterion 5's tolerances, as display output after stage 9.
7. **Presets.** Each preset's Temp and Tint turn back into its xy within 1e-4. Cloudy is 6504 K ± 2 K and Tungsten 2856 K ± 2 K with tint 0 ± 0.5, as DEV-01 criterion 10. No two presets round to the same shown values. *Check:* `color` tests.
8. **The picker.**
   - **RAW:** a synthetic sensor (the Pixel 4a profile run backwards, as in DEV-01's pyramid test) shows a gray patch lit by illuminant A, white balanced As Shot at D65. A pick on the patch gives 2856 K ± 20 K and tint 0 ± 2, and the patch renders with OkLCh chroma ≤ 0.005.
   - **Non-RAW:** an sRGB PNG with a gray patch, rendered warmer by Genzo at temp +40 and stored, is picked on the patch. The pick gives a negative temp and renders the patch with OkLCh chroma ≤ 0.01. The PR records the temp it measured.
   - A clipped patch and a black patch are refused, and the edit is unchanged.

   *Check:* engine tests.
9. **The panel against the fake engine** (Vitest, Playwright):
   - The Temp mapping is mired-linear and round-trips position to kelvin within 1 K.
   - Bright and dim follow the defaults, and the header dot follows the panel; Temp and Tint turn bright only in Custom mode.
   - A drag sends one *start*, updates and one *end*, and makes one history step. A held arrow key does the same. A typed value and a double-click reset each make one step.
   - V toggles the treatment and hides Vibrance and Saturation; their values come back.
   - W, then a click, sends `PickWhiteBalance` and turns the picker off; Esc cancels it.
   - A non-RAW photo shows "Embedded" disabled and two white balance entries. An unsupported photo disables the panel. A monochrome sensor disables white balance.
   - The dropdown names a preset when the values match it and says "Custom" when they do not.
   - The panel matches screen D1 of the handoff, apart from the rows M1 leaves out.
10. **Performance.** The base curve adds at most 1 ms to stage 6 at 2560 × 1920 on the RTX 3060, within DEV-01 criterion 14's 16 ms per slider. *Check:* the stage 6 criterion benchmark.
