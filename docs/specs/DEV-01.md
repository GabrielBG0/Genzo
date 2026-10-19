# DEV-01 · RAW processing pipeline (M1 part)

The pipeline turns a decoded file and an edit into pixels, the same way for the viewport and for export. This spec covers what M1 builds:

- decode
- stages 1 to 4
- the core of stages 6 and 7
- crop and rotate
- the output transform

Its rules bind every later stage.

Sources: the spec's *Image pipeline and performance* and *RAW and format support* sections (revision 85), the M1 section of `docs/development-plan.md`, and `docs/m0-results.md`. Where this spec and the technical spec disagree, the technical spec wins and this file is corrected.

**Out of scope for M1**, with their milestone:
- stage 5, lens and perspective (M3)
- the rest of stage 6: tone curve, texture, clarity, dehaze (M3)
- the rest of stage 7: HSL, color mixer, color grading, calibration (M3)
- stage 8, detail: sharpening and noise reduction (M3)
- stage 9 effects: vignette and grain (M3)
- local adjustments and masks (M3)
- AI denoise (M4)
- applying the ProRAW gain table map (later, opt-in)
- DCP profiles and the profile tables embedded in DNG (not planned)

The UI for the edit parameters is DEV-02 (Basic panel) and DEV-12 (crop). Their ranges, units and defaults are defined here, under Data, and only here.

## Behavior

### The stage order

The order is fixed, as the spec sets it. The table shows which stages M1 builds and where each runs.

| # | Stage | In M1 | Resolution |
|---|---|---|---|
| 0 | Decode: sensor data or pixels, levels, matrices, opcode lists, embedded preview | yes | file |
| 1 | Linearize: black and white levels, LinearizationTable, OpcodeList1 and 2 | yes | full, per tile |
| 2 | White balance and highlight clipping | yes | full, per tile |
| 3 | Demosaic (RCD for Bayer, Markesteijn for X-Trans), then OpcodeList3. Skipped for linear DNG and non-RAW | yes | full, per tile |
| 4 | Camera matrix into the working space, BaselineExposure, highlight desaturation | yes | full, per tile, then the pyramid |
| 5 | Lens and geometry correction | M3 | |
| 6 | Exposure and tone | exposure, contrast, highlights, shadows, whites, blacks, the profile's base tone curve | viewport |
| 7 | Color | vibrance, saturation, B&W treatment | viewport |
| 8 | Detail | M3 | |
| 9 | Effects, crop, output transform | crop and rotate, output transform | viewport |

"Viewport" means the resolution actually shown: the viewport size at Fit, or full resolution for the visible tiles at 1:1 and during export.

### Stage 0: decode

`imgio::decode` returns one normalized structure, and the pipeline never sees a file format. It holds:

- **Pixels**, one of:
  - a CFA mosaic (Bayer 2×2 or X-Trans 6×6, with its pattern), `u16`;
  - linear RGB (ProRAW and other LinearRaw DNG), `u16`, three samples per pixel;
  - a monochrome linear plane (one sample per pixel);
  - encoded RGB from a non-RAW file, `u16`, with its color encoding (sRGB, Display P3, NCLX or an ICC profile).
- **Levels:**
  - black levels per channel or per CFA position (an array, up to 36 entries for X-Trans);
  - white levels per channel;
  - the active area and default crop.
- **Color data:**
  - ColorMatrix1 and 2 with their calibration illuminants;
  - ForwardMatrix1 and 2 when present;
  - the as-shot neutral or white balance coefficients (only the first three; the fourth is NaN on every body tested);
  - BaselineExposure.
- **The opcode lists** (1, 2 and 3), parsed into a typed list. Each opcode keeps its optional flag.
- **Orientation**, the EXIF or DNG value 1 to 8, *not* applied to the pixels (see Geometry).
- **The embedded preview**, through `imgio::decode_preview`, as RGBA8 with its orientation (not applied, as for the image) and color encoding.
  - **How it is found:** every embedded baseline or progressive JPEG stream in the file is a candidate, found by its bytes, not by each format's container. One scan serves DNG, NEF, CR3, ARW, RAF, ORF, RW2 and the rest, and still works when the container is damaged. A JPEG file's own image is not its preview: only its APP1 EXIF block is searched, so the thumbnail is, and an Ultra HDR gain map or MPF image after the main image never is.
  - **Which one:** the smallest whose long edge is at least 1280 px, else the largest. Cameras store a 1616 to 2016 px preview beside the full-size one, and that decodes in 10 to 20 ms. Fujifilm, Olympus, Pentax, Samsung and Halide files carry only large previews: 44 to 98 ms on the reference desktop, within the 100 ms first-pixels budget but close to it. If that is not enough, the next step is the 160 px thumbnail as an instant first frame.
  - **False matches:** a candidate must hold DQT, SOF and SOS in order with sane values, end at its EOI, and be large enough for its size. It is decoded in strict mode, so a truncated stream fails and the next candidate is tried.
  - **Orientation and color:** the file's own declarations win (IFD0 of TIFF-based files, the CR3 CMT1 box, a JPEG's APP1), then the stream's EXIF. Cameras store previews unrotated; the iPhone DNGs (orientation 6) have landscape previews. HEIC previews come from libheif with the item's rotation applied, so they report 1.
  - **Files with none:** PNG, plain TIFF, GoPro GPR, Hasselblad 3FR, Kodak DCR and Phase One IIQ return none. Nikon High Efficiency files, which cannot be decoded yet, return their 1620 px preview.
  - **Checked** against an independent parser plus libjpeg-turbo on 21 corpus files (`scripts/preview-reference.py`, `imgio/tests/preview_corpus.rs`): same size, per-channel means within 0.5 step.
- **The gain table map**, read and kept, never applied in M1.

Which decoder reads which format:

- **Linear DNG** (every ProRAW generation, Samsung DNG): `dng::decode_linear`. The LinearizationTable is applied exactly, and samples past its end take its last entry. Tiles or strips (Samsung writes one strip), in JPEG XL, lossless JPEG or uncompressed.
  - Lossless JPEG goes through rawler's decompressor, which ignores restart intervals and returns wrong samples after the first restart, with no error. The Galaxy S23 Ultra writes a restart every 16 rows. So a stream with restarts is split at its RST markers and each interval decoded on its own, in parallel (`imgio/src/ljpeg.rs`). A restart layout other than whole rows of 1 × 1 sampled components is *unsupported*, never decoded.
- **CFA DNG and camera RAW:** rawler 0.8.0, vendored with patches (`backend/vendor/README.md`): curve lookups are exact, not dithered, and sizes read from the file are checked before anything is allocated. The output is the whole sensor mosaic, masked borders included, with its pattern relative to the first sample. The active area and default crop come in the metadata.
  - **Camera RAW:** levels, matrices, white balance and orientation are mapped from rawler. White balance coefficients become the as-shot neutral (G/R, 1, G/B). The two matrices with the lowest and highest color temperature become ColorMatrix1 and 2. A file with no white balance (Kodak DCR) has none; stage 2 decides the default.
  - **CFA DNG:** rawler supplies only the samples. The pattern and all metadata come from our own TIFF tag reader. That includes the tags rawler leaves empty (opcode lists, ForwardMatrix, the gain table map) and the pattern itself, because rawler shifts it by the ActiveArea offset in the wrong direction. A CFA DNG with lossless JPEG restart intervals or a LinearizationTable is *unsupported*: rawler decodes both wrongly, and no corpus file has either.
  - **Checked:**
    - The samples of 14 corpus files equal LibRaw 0.22.1's on every sample (`scripts/cfa-reference.py`, `imgio/tests/cfa_corpus.rs`). Phase One IIQ is the exception: rawler applies the camera's sensor calibration inside its decoder, so it can't be compared with LibRaw's uncalibrated data.
    - The matrices and as-shot neutral of 6 cameras match Adobe DNG Converter's DNGs, to 1e-4 and 1e-3.
    - White levels come from rawler's camera database and differ from LibRaw's on 9 of the 13 compared files (for example D850: 3880 against 4095; CR3: 14888 against 16383). Stage 2's highlight clipping depends on them, so they are checked again when stage 2 is built.
  - **Unsupported by name:** Nikon High Efficiency ("Nikon High Efficiency RAW is not supported yet") and GoPro GPR (VC-5).
- **Nikon High Efficiency:** the JPEG XS decoder (decided), behind the `jpeg-xs` cargo feature that release builds leave off (patent pool, see `docs/licenses.md`). Without the feature, and until the decoder lands, these files are *unsupported* (see States).
- **HEIC:** libheif.
- **JPEG:** zune-jpeg 0.5.15 (the spec's choice), to RGBA16.
  - The color encoding is the ICC profile if there is one; otherwise Adobe RGB when EXIF declares it the DCF way (ColorSpace 0xFFFF with Interoperability index "R03"), which cameras set to Adobe RGB write instead of a profile; otherwise sRGB.
  - Unsupported: CMYK and YCCK, 12-bit, arithmetic-coded and lossless JPEG.
  - Against libjpeg-turbo the samples differ by at most 1 step (8-bit) for grayscale, 2 for 4:4:4, 3 for 4:2:2 and 4 for 4:2:0, with a mean under 0.33 step (`imgio/tests/image_formats.rs`). The IDCTs agree to a step; the rest is chroma upsampling at color edges.
- **PNG:** the png crate 0.17.16. Palette, tRNS and 1, 2 and 4-bit gray are expanded; 16-bit stays 16-bit; Adam7 is deinterlaced; the orientation comes from eXIf.
  - Color by PNG 3's precedence: cICP, then iCCP, then sRGB.
  - A file with only gAMA or cHRM, or no color chunk, is taken as sRGB (decided for M1).
- **TIFF (plain images):** the tiff crate 0.11.3. 8 and 16-bit gray, gray and alpha, RGB and RGBA, in strips or tiles, chunky or planar, uncompressed, LZW, Deflate, PackBits or JPEG.
  - YCbCr from JPEG-in-TIFF is converted to RGB with the file's YCbCrCoefficients and ReferenceBlackWhite.
  - Associated (premultiplied) alpha is turned into straight alpha.
  - Unsupported: CMYK, Lab, palette (the tiff crate refuses it), floating point, bilevel.
  - **Routing:** a non-DNG TIFF goes to the camera decoder first. Only if that fails, and IFD0 holds a full image the way a plain TIFF does, is it read as a TIFF image. IFD0 must not be reduced-resolution and must have a TIFF 6 photometric. The file must also be accounted for, leaving at most 64 KiB of unexplained data: the image data of every IFD in the chain (pages), plus the bulk metadata tags (Photoshop layers and image resources, XMP, ICC). So an exported TIFF whose Make names a camera opens, as do layered, multi-page and XMP-heavy TIFFs. A truncated NEF, ARW or IIQ never comes back as its thumbnail: the first three mark IFD0 reduced, and an IIQ's raw data is referenced by no IFD.
- Every non-RAW decoder checks the size against the 400 MP limit before allocating. The decoders' own limits are raised to match: zune-jpeg's 16384 px per side, png's 64 MiB and tiff's 256 MiB defaults would refuse real panoramas and large TIFFs.

Large linear DNGs decode their tiles in parallel and stream them into stage 1. No full-size `u16` copy is held beside the input, so a 48 MP ProRAW peaks below the 445 MiB measured in M0.

**Opcodes.** M1 applies what the corpus contains. The inventory of the corpus (`cargo run -p genzo-imgio --features dng --example dng_inventory -- ../corpus`, 2026-10-18), on the files as the cameras wrote them:

| File | List | Opcode | Optional |
|---|---|---|---|
| Galaxy S23 Ultra (linear DNG) | 2 | GainMap × 4: no-ops (1 × 1, gain 1.0), their area left over from another sensor mode and larger than the image | yes |
| Google Pixel 4a (CFA DNG) | 2 | GainMap × 4: real lens shading, 30 × 40 points, gains 1.0 to 3.4, one per Bayer site | **no** |
| Google Pixel 4a | 3 | WarpRectilinear (the identity) | yes |
| Leica Q2 (CFA DNG) | 1 | FixBadPixelsConstant (constant 0; the file has no zero samples, so it changes nothing) | **no** |
| Leica Q2 | 3 | WarpRectilinear | **no** |
| iPhone 16 Pro Bayer DNG (Halide) | 3 | WarpRectilinear2, FixVignetteRadial | **no** |
| iPhone 12 Pro and 16 Pro ProRAW, Pentax K10D | | none | |

Adobe DNG Converter's output of the camera RAW files carries no opcodes. So M1 implements FixBadPixelsConstant and skips the lens opcodes whether or not they are optional (decided 2026-10-18, spec rev 86): the Leica Q2 and the Halide DNG render, without lens corrections.

- GainMap (OpcodeList2, used by Samsung and Pixel phones) is applied in M1 whatever the inventory shows.
- FixBadPixelsConstant (OpcodeList1, Leica) is applied in M1.
- **How they are applied** follows the Adobe DNG SDK (`dng_gain_map.cpp`, `dng_bad_pixels.cpp`), read from its source in GoPro's GPR repository. darktable and RawTherapee each cover only part of it.
  - The CPU reference is `genzo_pipeline::reference::opcodes`; the GPU stage 1 is tested against it. Each opcode's area is intersected with the image, keeping its row and column pitch from its own top left. OpcodeList1 runs on the raw image as stored; OpcodeList2 and 3 on the image after linearization, whose origin is the ActiveArea's top left.
  - **GainMap:** a pixel's relative position is that of its center, ((row + 0.5) / height, (col + 0.5) / width). The map index is (position − origin) / spacing, interpolated bilinearly and held at the first and last points. The value, normalized to white = 1, is multiplied by the gain and clipped at 1. Map plane = min(plane, MapPlanes − 1).
  - **FixBadPixelsConstant:** on the stored 16-bit values, a sample equal to the constant becomes the rounded mean of its same-color neighbors that are not bad, read from the original. A green site (row + col + phase + phase / 2 even) uses its four diagonal neighbors; a red or blue site the four two pixels away on its row and column. With four neighbors the mean is (sum + 2) / 4. With none, the sample is kept. Where the SDK pads the image at its edges, Genzo leaves the outside neighbors out.
  - **Checked:** synthetic tests of each rule, and the Pixel 4a's maps against Adobe DNG Converter's rendering of the same file (`pipeline/tests/gain_map.rs`). Per 64 px block, the ratio to Adobe is flat: a spread of 1.01 in green and 1.06 in red and blue, against 2.3 to 2.7 without the maps and 1.2 with the red and blue maps swapped.
- Opcodes are parsed and planned at decode (`imgio::opcodes::plan`): FixBadPixelsConstant in OpcodeList1 and GainMap in OpcodeList2 are applied; one whose parameters do not parse is skipped when optional, else the photo is unsupported.
- **In another list they count as not implemented** (decided 2026-10-18). A GainMap in OpcodeList3, for one, would run after stage 2 has white balanced the values, so its clip at 1 would be in the wrong units. No corpus file has either elsewhere. More than one FixBadPixelsConstant is unsupported too, since each would read the previous one's output.
- An opcode M1 does not implement is skipped when it is marked optional.
- When an unimplemented opcode is not marked optional, the photo is *unsupported*, except for the lens opcodes.
- The lens opcodes (WarpRectilinear, WarpRectilinear2, FixVignetteRadial) are skipped in M1 whether or not they are marked optional (decided 2026-10-18). When a non-optional one is skipped, the status bar says "Lens corrections from the file are not applied yet". Stage 5 takes them up in M3.

### Stage 1: linearize

Runs on the GPU, per tile of the sensor image (see Tiling), as the Adobe DNG SDK 1.4 builds its stage 2 image (`dng_linearization_info.cpp`):
- **OpcodeList1** runs first, on the stored values (in M1: FixBadPixelsConstant, at most one).
- **The levels.** Each value becomes `v · s − b[row, col]` as `f32`, where:
  - `s = 1 / (white[channel] − max black)`. The max black is the largest black level of the pattern after adding the largest DeltaH and DeltaV of each pattern phase (`MaxBlackLevel`), not the black at the position.
  - `b[row, col] = (pattern[row mod rows][col mod cols] + DeltaH[col] + DeltaV[row]) · s`.
  - The pattern, DeltaH and DeltaV start at the ActiveArea's top left. The decoder hands them over that way: rawler's black pattern starts at the sensor's first sample, so imgio shifts it.
- **Negative values are kept** for linear input: they are noise, and clamping them would lift the shadows. The SDK clips them to 0. The Bayer demosaic's input is clamped at 0 (after stage 2), since RCD's estimators divide by sums of neighbors.
- **Then OpcodeList2** runs (in M1: GainMap).

The LinearizationTable is already applied by the decoder for linear DNG, and by rawler where it applies one.

**Checked** against the SDK itself: `scripts/dng-sdk-probe/stage2_probe.cpp` writes the SDK's stage 2 image, and `scripts/stage1-reference.py` stores five 48 px crops and the 128 px block means of six corpus DNGs (`pipeline/tests/fixtures/sdk-stage2`):

| File | What it covers |
|---|---|
| Pixel 4a | per-site blacks, real GainMaps |
| Leica Q2 | FixBadPixelsConstant; an ActiveArea that ends 32 px inside the sensor |
| Pentax K10D, Halide iPhone 16 Pro | plain Bayer |
| Galaxy S23 Ultra, iPhone 12 Pro | linear, three planes |

- **The bound.** The CPU reference (`reference::stages`) and the GPU stage both match to the SDK's 16-bit rounding: half a code, or half a code times (1 + the largest gain) where a GainMap scales the SDK's rounded value. On the Pixel that is 2.1 codes.
- **Negative controls must fail:** no GainMaps, the black rows swapped, the active area two columns off.

### Stage 2: white balance and highlight clipping

- **White balance.** Each channel is multiplied by its white balance multiplier, from the Temp and Tint of the edit (see Data). The multipliers are normalized so the smallest is 1.
- **Highlight clipping.** Every channel is clipped at the level where the *first* channel saturates after white balance: `clip = min(multiplier[c])` in normalized units. A clipped area therefore comes out neutral white, not magenta or cyan. Real reconstruction of clipped color is later (decided 2026-10-18).

While a slider is dragged, white balance runs as a ratio after the cached demosaic. The pyramid holds working-space values after stage 4, so the ratio `r` (new multipliers over the pyramid's) is applied there as one matrix, `M_new · diag(r) · M_old⁻¹`, where `M_old` is the matrix the pyramid was built with (it keeps its white point, multipliers and matrix). The settled render re-runs this stage and the demosaic with the new multipliers, as M0 does.

### Stage 3: demosaic

- **Bayer: RCD,** the M0 port of darktable's `demosaic_rcd.cl`, with the one documented change. It is checked against the CPU port of `rcd.c`.
- **X-Trans: Markesteijn, 3 passes,** a WGSL port of darktable's `xtrans_markesteijn_interpolate`. It gets a literal CPU port as its test reference, like RCD. Interactive drags never re-demosaic, so the slower 3-pass variant only costs time on the settled render.
- **Linear DNG, monochrome planes and non-RAW** skip this stage. A monochrome plane is copied to R, G and B.
- OpcodeList3 runs after the demosaic, on camera RGB, before the matrix.

### Stage 4: camera matrix

- **The white point.** It comes from the edit's Temp and Tint (Data: *White balance*).
- **The math is the Adobe DNG SDK's** (version 1.4: `dng_color_spec`, `dng_camera_profile`, `dng_temperature`), ported step by step to f64 in `color::camera` and tested against the SDK itself (see *The DNG SDK as reference* below). It gives Lightroom's colors for the same profile.
- **Camera to XYZ D50** (the DNG profile connection space, white xy 0.3457, 0.3585):
  - With ForwardMatrix tags, they are interpolated by the white point, as the DNG specification describes.
  - Without them, ColorMatrix1 and 2 are interpolated by inverse correlated color temperature between their calibration illuminants. The result is inverted, after a Bradford adaptation from D50 to the white point.
- **XYZ to the working space:** the D50 result is adapted to D65 with the same linearized Bradford transform, then converted to linear ACES AP1 primaries with a D65 white (decided 2026-10-14). The pyramid is stored as RGBA16F.
- **The split between stages 2 and 4.** The SDK gives a camera white (the white point's camera values, the largest at 1) and a CameraToPCS matrix. Stage 2's multipliers are 1 / camera white, so the smallest is 1. Stage 4's matrix is `XYZ_to_AP1 × Bradford(D50 → D65) × CameraToPCS × diag(camera white)`, so a white balanced neutral (1, 1, 1) comes out as AP1 (1, 1, 1). Together they are the SDK's chain.
- **Non-RAW files** enter here: decoded values are linearized with their transfer function and converted from their primaries or ICC profile (moxcms) into the working space.
- **AnalogBalance and CameraCalibration** enter the matrix as the DNG specification describes (chapter 6): XYZ to camera is AnalogBalance × CameraCalibration × ColorMatrix, and with ForwardMatrix tags, camera to XYZ is ForwardMatrix × D × (AnalogBalance × CameraCalibration)⁻¹, where D scales the reference neutral to 1. Both are real in the corpus: the iPhone 16 Pro ProRAW has AnalogBalance (2.35, 1, 1.86) with AsShotNeutral (1, 1, 1), and the Galaxy S23 Ultra has CameraCalibration1 and 2 that are not identity. Absent tags mean identity.
- **BaselineExposure** is applied here as a gain of 2^BaselineExposure (+2.50 EV on the iPhone 13 Pro Max file, so it decides the default look).
- **Highlight desaturation.** Pixels whose largest camera channel lies within the top 5 percent below the clip level are blended toward their own luminance, reaching neutral at the clip level. The edge of a clipped area then carries no color fringe. Exactly (`reference::stages::stage4`):
  - the weight is `k = clamp((max(camera) − 0.95 · clip) / (0.05 · clip), 0, 1)`, where `camera` is the white balanced, demosaiced pixel and `clip` the clip level (the smallest multiplier);
  - the pixel goes through the camera matrix first, then becomes `w + (Y − w) · k`, where `w` is its working-space RGB and `Y` its luminance (the Y row of `WORKING_TO_XYZ`, so a neutral stays where it is);
  - BaselineExposure's gain comes last.
- **When the camera's color is unusable** (no three-color ColorMatrix, a profile the SDK refuses, a singular matrix), camera values pass through unchanged, balanced only by the as-shot neutral, and the status bar says the photo's color profile could not be used. When only the as-shot white cannot be solved, D55 is used (`camera_color::ColorFallback`).

The output of stage 4 builds the mip pyramid (see Tiling).

#### Camera color: the SDK's details

These are easy to miss and each changes the result. All are in `color::camera`, with the SDK function named in its comments.

- **Loading the profile.**
  - Each ColorMatrix is scaled so the D50 white's largest camera value is 1, but only when it is outside 0.99 to 1.01. It is then rounded to 4 decimals.
  - ForwardMatrix and CameraCalibration are rounded to 4 decimals.
  - A ForwardMatrix must map camera (1, 1, 1) to within 0.01 of D50, or the profile is refused. It is then scaled row by row to map to D50 exactly.
  - ForwardMatrix2 and CalibrationIlluminant2 count only with a ColorMatrix2.
- **The negative's values.**
  - AnalogBalance is scaled so its smallest gain is 1 and rounded to 6 decimals.
  - AsShotNeutral is scaled so its largest value is 1 and rounded to 6 decimals.
  - AsShotWhiteXY is rounded to 6 decimals.
- **CameraCalibration** is used only when CameraCalibrationSignature equals ProfileCalibrationSignature, two absent ones included. Both cases are in the corpus. The Galaxy S23 Ultra's own DNG has no signatures, so its calibration is used. Adobe's conversion of the same file has only a profile signature, so its calibration is not.
- **Calibration temperatures** come from the illuminant codes:

  | Illuminant | Temperature |
  |---|---|
  | A, tungsten | 2850 K |
  | ISO studio tungsten | 3200 K |
  | D50 | 5000 K |
  | D55, daylight, fine weather, flash, B | 5500 K |
  | D65, C, cloudy | 6500 K |
  | D75, shade | 7500 K |
  | Fluorescents | the middle of their ranges |
  | Unknown and other | 0 |

  - **One matrix** stands for both calibrations at 5000 K when there is no ColorMatrix2, when a temperature is 0, or when the two are equal.
  - **Reversed order:** two calibrations stored warm first are swapped with their forward matrices and calibrations. The Galaxy S23 Ultra stores D65 first, then A.
- **Interpolation.** One weight `g = (1/T − 1/T2) / (1/T1 − 1/T2)`, clamped to 0 to 1, is used for the color matrix, the forward matrix and the camera calibration. T is the white point's Robertson temperature.
- **The neutral to a white point** (`NeutralToXY`):
  - Iterate from D50 for at most 30 passes, until the step is under 1e-7 in |dx| + |dy|.
  - On the last pass, take the mean of the last two estimates.
  - A singular matrix gives no white point. The photo then falls back to D55 and the status bar says so.
- **The camera white** is pinned to 0.001 to 1 per channel. The Bradford scales are limited to 0.1 to 10.
- **Not read:** DNG 1.6's third calibration (CalibrationIlluminant3, ColorMatrix3, IlluminantData). SDK 1.4 predates it and no corpus file has it; `scripts/color-reference.py` asserts that. ColorimetricReference (output-referred DNG) is not read either. With it, the SDK uses D50 and ignores AsShotNeutral.
- **Only three color channels.** Four-color sensors (CYGM, RGBE) have no profile here.

#### The DNG SDK as reference

- `scripts/dng-sdk-probe/` holds a small C++ program that links the DNG SDK 1.4 copy in gopro/gpr. That copy is under the BSD-3-Clause license. The probe is a test tool and is never built into Genzo; `color::camera`, the port, carries Adobe's notice (`docs/licenses.md`).
- For each DNG, the probe prints the SDK's results: the as-shot white (xy, Temp, Tint), and the camera white and CameraToPCS at that white and at D50, D55, D65 and A.
- `scripts/color-reference.py` runs the probe over 17 corpus DNGs and two patched copies, and stores each file's color tags (read with tifffile) beside the SDK's results in `color/tests/fixtures/dng-sdk-color.jsonl`. The 17 are 6 camera and phone DNGs and Adobe DNG Converter's 11 references.
- **The patched copies:**
  - **The iPhone 16 Pro ProRAW** is DNG 1.7 with JPEG XL tiles, which SDK 1.4 refuses. Its copy says DNGBackwardVersion 1.4 and Compression 7. Only tags are read, so the color math is unaffected.
  - **The Leica Q2** copy has CalibrationIlluminant2 set to 0, which exercises the one-matrix branch.
- **Tests:**
  - **`color/tests/dng_sdk.rs`** needs no corpus. It holds Genzo to the SDK within 1e-11 in xy and camera white, 1e-10 in the matrix, 1e-7 K and 5e-9 in Tint; the measured differences are under 1e-11. Its negative controls must fail: a flipped calibration gate either way, the matrices in the wrong order, and a change of two rounding steps in one matrix entry.
  - **`pipeline/tests/camera_color.rs`** checks that the decoder reads the same tag values from the corpus files, and that the white it renders with is the SDK's.
  - **`dng-sdk-temperature.jsonl`** holds the SDK's conversion from Temp and Tint to xy, for a check of `xy_from_temperature`.
- **Non-DNG RAW files** go through the same code, with the color matrices and illuminants from rawler's camera data. These are in the DNG convention (XYZ to camera) and have no forward matrices, so they take the matrix-only path that the checks cover. The checks cover the math, not rawler's matrix values.

### Geometry: orientation, rotation, crop

Geometry changes which source pixel lands where. It changes no pixel values in M1, so it is applied as one coordinate transform where the pyramid is sampled into stage 6. Since it changes no values, that gives the same result as applying it at stage 9. Stage 5 (M3) composes into the same transform. The order:

1. The file's orientation (EXIF or DNG, values 1 to 8). The decoder never applies it, so every format follows one rule. For HEIC, libheif applies the container's rotation and mirror itself, and the decoder then reports orientation 1.
2. The edit's quarter turns and horizontal flip.
3. The straighten angle and the crop rectangle, together (see Data: *Crop*).

**Sampling.** The pyramid level is the smallest one at least as large as the output scale, sampled with a Lanczos-2 filter sized to the remaining scale factor. Upsampling happens only past 1:1.

### Stage 6 (M1 part): exposure and tone

This stage works on linear AP1 at viewport resolution. User adjustments change luminance only, and RGB is scaled by the luminance ratio, so hue and saturation hold. Luminance `Y` is AP1's Y row. Tone math works in log2 relative to middle gray: `t = log2(Y / 0.18)`.

1. **Exposure:** a gain of 2^EV.
2. **Highlights and Shadows: local tone mapping** (decided 2026-10-18).
   - **The base layer** is `B`, an edge-aware smoothing of `t`: a guided filter of `t` guided by itself, with a radius of 2 percent of the cropped image's long edge.
   - **Computed once per edit**, on the whole cropped image at a fixed working size (long edge 1536 px), independent of zoom. It is upsampled with the same guided filter at the output resolution. Fit, 1:1 and export therefore use the same base, and a 1:1 tile needs no context beyond it.
   - **Detail** is `D = t − B`, and is kept.
   - **Highlights** move `B` by `h × w_h(B)`, and **Shadows** by `s × w_s(B)`. The weights are smooth ramps: `w_h` rises from 0 at middle gray to 1 at +2.5 EV, and `w_s` mirrors it toward −4 EV. `h` and `s` scale the slider (−100 to +100) to at most ±2 EV.
   - Exposure is a constant shift in `t`, so changing it shifts `B` by the same amount and needs no new base. White balance, the crop, the straighten angle, the quarter turns and the flip recompute it. The profile does not, since both profiles share everything before stage 6.
   - During a crop gesture, the base from the gesture's start is used, and it is recomputed on *end*.
3. **Whites and Blacks:** global. They move the upper and lower end of the tone range, the points that map to display white and black after the base curve, by at most ±1.5 EV.
4. **Contrast:** global. An S-curve in `t` around middle gray, which leaves middle gray unchanged.
5. **The profile's base tone curve.**
   - Genzo Standard applies Genzo's base tone curve, which maps scene-referred values to display-referred 0 to 1. The curve and how it is applied are defined in the DEV-02 spec.
   - Genzo Linear applies none: values are clamped at display white, after the output transform's gamut compression.
   - Non-RAW files use "Embedded": no base curve, since they are display-referred already.

### Stage 7 (M1 part): color

Works in OkLCh, converted from linear AP1 through XYZ (D65).

- **Saturation** (−100 to +100) scales chroma uniformly: ×0 at −100, ×2 at +100.
- **Vibrance** (−100 to +100) scales chroma more where it is low: `C × (1 + v × weight)`, with `v` from −1 to +1. The weight is `1 − C / C_ref`, clamped to 0 or more, where `C_ref` is the chroma of the most saturated sRGB primary. So at +100 a nearly gray color doubles its chroma, and a color at `C_ref` is left alone. Skin hues (OkLCh hue 20° to 70°) get half the weight, so faces do not turn orange. At −100 it desaturates evenly, like Saturation.
- **B&W treatment.** Output is AP1 luminance in all three channels. Vibrance and saturation have no effect, and DEV-02 hides them. The per-color B&W mix is M3.

### Stage 9 (M1 part): output transform

1. **Gamut compression** (decided 2026-10-18). Convert linear AP1 to the target's linear RGB (sRGB, or Display P3). Then compress per channel toward the achromatic axis, using the ACES reference gamut compression method: distance from achromatic, a threshold, a limit and a power curve.
   - Colors inside the threshold are untouched.
   - Colors out of gamut keep their hue and land inside the gamut instead of clipping into flat patches.
   - The cost: colors between the threshold and the gamut edge move inward a little, which is what makes the edge soft.
   - The parameters for each target are set in the implementation PR, against the acceptance criteria below.

   **When it runs:**
   - **RAW photos:** always.
   - **Non-RAW photos:** only when the source gamut is not inside the target. Adobe RGB or ProPhoto RGB into sRGB qualifies, and so does Display P3 into sRGB. An sRGB JPEG into an sRGB frame does not, so its saturated colors stay exact. Colors an edit pushes past the edge are clipped per channel there.
2. **Clamp** to 0 to 1, then encode with the sRGB transfer function (also used by Display P3).
3. **Quantize** to 8 bits with deterministic dither: triangular, ±1 code, seeded by pixel position, so goldens are stable.
4. **Tag the frame** `rgba8Srgb`, `rgba8DisplayP3`, or the matching JPEG preview format. The webview converts to the monitor (spec v1 color policy).

Export (OUT-01 basic) uses this transform with sRGB as the target, and embeds an sRGB ICC profile.

### Tiling and the mip pyramid

Stages 1 to 4 run per tile, so full-resolution intermediates exist only for one tile at a time.

- **Tiles** are 1024 × 1024 output pixels for Bayer, linear and non-RAW input, and 512 × 512 for X-Trans. Each has a 24 px apron on each side: 24 is a multiple of both CFA periods, 2 and 6, and wider than RCD's and Markesteijn's support. The apron is computed and thrown away, so no tile edge shows. X-Trans tiles are smaller because 3-pass Markesteijn keeps several directional RGB buffers, about 200 bytes per pixel against RCD's 52.
- **How a tile computes what the whole image would** (`gpu::raw_stages`, `tiles::jobs`):
  - Its textures cover the padded rect: the output rect plus the apron, clamped to the active area. Reads clamp to that rect.
  - Everything that depends on where a pixel is uses image coordinates: the CFA color (from the stored image's phase), RCD's 9 px border, the black pattern and the GainMap positions.
  - The uploaded raw rect reaches 2 px further, clamped to the stored image, so FixBadPixelsConstant sees the same neighbors as on the whole image, masked border included.
  - RCD's support is 10 px (the final pass reads ±3 around values that reach 7) and the bad-pixel fix's is 2, both inside the apron.
  - **The test** (`pipeline/tests/raw_stages.rs`) holds a tiled render to the whole image as one tile, bit for bit. It uses a synthetic sensor with an odd active-area offset, per-site blacks with DeltaH and DeltaV, bad pixels at the active edge and on tile boundaries, and pitched GainMaps across tiles. A 6 px apron must fail it.
  - **RCD against `rcd.c`:** the GPU matches darktable's CPU code to f32 rounding (median 2e-7), except where RCD picks between two direction estimates by a comparison: a last-bit difference (fused multiply-adds on hardware GPUs) can flip the choice. On the RTX 3060 that is 9 of 146,000 values above 1 %; WARP, which does not fuse, flips none.
- **The input** stays in CPU memory. Each tile's slice is uploaded through a staging buffer.
- **The pyramid** is image-wide and starts at **level 1** (half size), in RGBA16F. Each tile's stage 4 output is reduced into level 1 with a 2 × 2 box filter, and levels 2 and up follow. **Level 0 (full resolution) is never kept for the whole image.**
  - **Sizes:** each level is half the one above, rounded down (a texture's mip chain). An odd last row or column folds into the last texel, which then averages 3 instead of 2, so every pixel counts once. Levels go up to the first whose long edge is 256 px or less.
  - **Stage 4 is fused into the reduction** (`tile_stage4.wgsl`): each level 1 texel runs stage 4 on its 2 × 2 block of the tile's demosaiced pixels and stores their mean. Tiles start on even pixels, and a last row or column of tiles only 1 px wide joins the one before, so a block, fold included, never straddles two tiles.
  - **Two tiles are in flight at most.** Each holds its upload staging until the GPU is done with it.
  - **A new photo frees the old pyramid first,** so two are never in GPU memory together.
  - **Tests** (`pipeline/tests/pyramid.rs`):
    - Tiled equals one tile, bit for bit, at every level.
    - Each level matches the CPU reference within one f16 step.
    - A synthetic sensor made by running the Pixel 4a's real profile backwards from a known scene renders back to it: 0.13 % on the gradients, neutral within 0.2 %.
- **At 1:1 and during export**, level-0 tiles of 256 px are made on demand by running stages 1 to 4 on the region they need, apron included.
  - At 1:1 they are cached in an LRU with a fixed byte cap.
  - Order: visible tiles first, then a ring of one tile around them.
  - Export streams tiles through the whole pipeline and keeps none.
- **GPU memory for one 48 MP edit** (8064 × 6048) has the following worst case. The memory-accounting test asserts the total. Each cap is a fixed constant that the test checks (`pipeline::budget`, `pipeline/tests/memory.rs`). The caps hold the sizes as drivers allocate them: NVIDIA's Vulkan driver pads textures by up to 7 % (measured 2026-10-18), and WARP allocates the exact sizes. The test measures what is allocated where the backend reports it (Vulkan and DX12 do), after the render and at its peak, sampled after each tile's submit while uploads are still staged.

| Item | Size |
|---|---|
| Pyramid, levels 1 and up, RGBA16F (98 MB for level 1, plus a third: 130 MB; 132.3 MB as NVIDIA allocates it) | 133 MB |
| Stage 1 to 4 scratch for one tile, intermediates included: Bayer 52 B/px × 1072² = 60 MB (the mosaic, seven RCD buffers, RGBA32F output; 64.2 MB on NVIDIA); X-Trans about 200 B/px × 560² = 63 MB | 65 MB |
| Uploads: the raw tile texture (linear DNG, 3 × u16: 6.9 MB; 7.4 MB on NVIDIA) and the staging of the two tiles in flight (2 × 6.9 MB); 21.2 MB at the measured peak | 22 MB |
| Level-0 tile cache at 1:1 (fixed cap; about 480 tiles of 256², five 2560 × 1440 views with their rings) | 256 MB |
| Viewport stage caches (fixed cap; stage 4 sampled, stage 6, stage 7 at 2560 × 1920 RGBA16F, plus the base layer) | 128 MB |
| Output frame and two persistent readback buffers, RGBA8 | 60 MB |
| Histogram and clipping masks | 10 MB |
| **Total** | **674 MB**, under the 1 GB budget with 326 MB to spare |

A software adapter uses the same sizes.

**Prefetched neighbors** are held in CPU memory, never on the GPU:
- **What is kept:** their pyramid (levels 1 and up), rendered on the GPU at low priority and read back.
- **Size:** 131 MB for a 48 MP photo, 33 MB for 12 MP.
- **The cap:** an LRU of 768 MB. That holds four 48 MP neighbors, two on each side.
- **On a switch,** the pyramid is uploaded and the photo renders at Fit without decoding.
- **The decoded input** is kept for the current photo only. It is what 1:1 and a settled white balance need, and for a neighbor it is decoded in the background after the switch.
- **Who owns it:** LIB-09 and the engine session.

### Re-runs, quality levels and generations

- **Cache per stage.** Each stage's output is cached, and a change re-runs from the earliest stage whose inputs changed.
  - Exposure, Whites, Blacks, Contrast and the tone of Highlights or Shadows re-run stage 6 and later.
  - Vibrance, Saturation and B&W re-run stage 7 and later.
  - The view (zoom, pan, viewport size) re-runs from the sampling of the pyramid.
  - Crop and rotation re-run from the sampling of the pyramid, and the base layer.
  - The profile re-runs the base curve in stage 6, and later.
  - White balance re-runs the base layer too, and on the settled render, stages 2 to 4 for the visible area.
  - The image re-runs everything.
- **Interactive** (during a gesture): the cheapest path to a correct-looking frame.
  - White balance is applied as a ratio after the cached demosaic.
  - At Fit, the frame goes as a half-size JPEG preview, the Windows ladder step that M0 measured.
  - At 1:1, the visible tiles go as JPEG at full scale (quality 80) instead of lossless. M0 measured full lossless frames at 111 to 125 ms p95 on Linux; JPEG tiles cut the bytes about twentyfold. The autobench's 1:1 drag case reports it, and the budget table does not gate it.
  - On the software adapter, the interactive frame is a 1280 px proxy.
- **Settled:** full quality, swapped in about 150 ms after the gesture ends.
  - White balance re-demosaics, visible area first: at Fit the pyramid is rebuilt tile by tile, starting with the tiles the view shows. At 1:1, the visible level-0 tiles go first.
  - The frame is lossless: the full frame at Fit, the visible tiles at 1:1.
- **Generations.** Every request carries a generation number, and requests coalesce latest-wins. A stale frame is dropped, and work in flight for an older generation is cancelled between tiles.
- **Readback** copies only what is sent: at 1:1, the visible 256 px tiles. It goes into staging buffers that stay mapped across frames.

## Inputs and shortcuts

DEV-01 has no shortcuts of its own; DEV-02 and DEV-12 own the controls. Its inputs reach the engine through the contract:

- **`ApplyEditDelta`**: one or more edit parameters (see Data) and the gesture phase: *start*, *update* or *end*.
  - Between *start* and *end*, renders are interactive. The settled render follows *end*.
  - A delta without a gesture, such as a typed value or a reset, is a start and an end in one.
  - The engine records one history step per *end* (APP-10).
- **`SetView`**: the viewport size in device pixels, the zoom (Fit or 1:1), and the visible rectangle in image coordinates after geometry. Panning at 1:1 sends `SetView`, and the engine renders what became visible.
- **`SelectPhoto`**: the photo whose edit the deltas apply to.

## States

| State | What the viewport shows | What the user can do |
|---|---|---|
| **Opening** | The embedded preview within one frame, if the file has one; otherwise a neutral `bg` fill and the file name. The fit render replaces it when ready | Edit. Sliders work, and the first render applies them |
| **Ready** | The rendered photo | Everything |
| **Rendering after a change** | The last frame stays until the new one arrives. The status bar shows the preview state (interactive or settled) | Everything |
| **Unsupported**: an unknown format, an unsupported compression (Nikon HE in builds without the `jpeg-xs` feature), a non-optional opcode M1 lacks, missing color data, CMYK JPEG | The embedded preview if there is one, and the reason in the status bar ("Nikon High Efficiency RAW is not supported yet") | View only. The Develop controls are disabled |
| **Damaged file**: truncated, or failing a decoder's bounds checks | As unsupported, with "This file is damaged" | View only |
| **Software adapter** | As Ready, slower, with the notice in the status bar | Everything |
| **Out of GPU memory** | The engine frees the level-0 tile cache and the viewport caches and retries once. If it fails again: the embedded preview, and "Not enough GPU memory to render this photo" | View only, until another photo is selected. The render thread stays alive (PR #22's error scopes) |
| **Device lost** | The engine opens the adapter again once and re-renders. If that fails, as above with "The graphics device stopped responding" | As above |

## Data

### The edit struct

The edit struct is defined in `model` and versioned with serde, starting at `schema_version: 1`.
- Unknown fields are kept on load and written back.
- An edit equal to the defaults is unedited. The filmstrip's edited dot and the panel dots compare against the defaults.
- In M1, edits live in the engine's memory for the session (decided); the M2 catalog stores this same struct.

| Field | Type, unit | Range | Default |
|---|---|---|---|
| `profile` | enum | `GenzoStandard`, `GenzoLinear` (RAW); `Embedded` (non-RAW, the only value there) | `GenzoStandard` for RAW, `Embedded` otherwise |
| `treatment` | enum | `Color`, `Monochrome` | `Color` |
| `white_balance.mode` | enum | `AsShot`, `Custom` | `AsShot` |
| `white_balance.temp` | f32, kelvin (RAW) or relative (non-RAW) | RAW: 2000 to 50000 K. Non-RAW: −100 to +100 | RAW: the as-shot value. Non-RAW: 0 |
| `white_balance.tint` | f32 | RAW: −150 to +150. Non-RAW: −100 to +100 | RAW: the as-shot value. Non-RAW: 0 |
| `exposure` | f32, EV | −5.00 to +5.00 | 0 |
| `contrast`, `highlights`, `shadows`, `whites`, `blacks` | f32 | −100 to +100 | 0 |
| `vibrance`, `saturation` | f32 | −100 to +100 | 0 |
| `crop` | `{cx, cy, w, h, angle}`, see Crop | see Crop | the whole image, angle 0 |
| `quarter_turns` | u8, clockwise | 0 to 3 | 0 |
| `flip_horizontal` | bool | | false |

Values are stored as f32, and the UI rounds what it shows (DEV-02). A vertical flip is a horizontal flip plus two quarter turns.

While `white_balance.mode` is `AsShot`, the engine fills `temp` and `tint` with the as-shot values so the sliders can show them. They are recomputed on load, never trusted from the stored edit, and left out of the "edited" comparison: `AsShot` is unedited whatever they hold.

### White balance: what Temp and Tint mean

**RAW.** Temp and Tint name the scene's white point.
- Temp is a correlated color temperature on Robertson's isotemperature lines.
- Tint is the distance from the Planckian locus, `Duv = −tint / 3000`, the scale of Adobe's DNG SDK. So Temp and Tint values read like Lightroom's.
- **As shot:** the white point is solved from AsShotNeutral, or from the camera's white balance coefficients, with the iterative method in the DNG specification. The matrix depends on the white point, so the two are solved together, with the same XYZ-to-camera matrix stage 4 uses (AnalogBalance × CameraCalibration × ColorMatrix). The iPhone 16 Pro needs that: its AsShotNeutral is (1, 1, 1), and its real gains sit in AnalogBalance. Then it is expressed as Temp and Tint.
- **The multipliers** are the inverse of the camera neutral at that white point.
- **A file with no as-shot data** (the Kodak DCS760C) uses D55, as the DNG SDK's renderer does (decided 2026-10-18; this spec said D65 before). The status bar says the as-shot white balance is unavailable.

**Non-RAW.** The pixels are already balanced, so Temp and Tint are relative shifts from the encoded white:
- **Temp:** the source white is the white whose reciprocal temperature is `153.8 − temp` mired (6500 K at 0). It lies on the CIE daylight locus from 4000 K up, and on the Planckian locus below. So −100 is about 3940 K, cooler, and +100 is about 18,600 K, warmer.
- **Tint:** moves that white by `Duv = −tint / 3000`.
- The image is adapted from that white to D65 with CAT16.

### Crop and rotation coordinates

The crop is defined on the image after the file's orientation, the quarter turns and the flip: the *oriented image*, normalized to 0 to 1 on each axis.
- `cx`, `cy`: the center of the crop rectangle.
- `w`, `h`: its width and height, as fractions of the oriented image's width and height.
- `angle`: the straighten angle in degrees, −45 to +45, positive clockwise, rotating the crop rectangle about its center. The rotation happens in pixel space: the rectangle is converted to pixels (`w × width` by `h × height`) first, so straightening never changes its aspect ratio.
- **The rotated rectangle must lie inside the oriented image.** The engine clamps a delta that would break this, by shrinking `w` and `h` about the center.
- **The minimum crop** is 32 source pixels on each side.
- **A quarter turn or flip** maps the existing crop into the new orientation, so the same region stays selected.

DEV-12 and the M2 catalog use these coordinates; a change here is a schema version bump.

### What is cached

| Where | What | Lifetime |
|---|---|---|
| GPU | The mip pyramid, levels 1 and up, of the current photo | Until another photo is selected or white balance settles |
| GPU | Level-0 tiles at 1:1, an LRU with a 256 MB cap | Until the view, the photo or an earlier stage changes |
| GPU | The viewport stage caches, 128 MB cap | Until the view or an earlier stage changes |
| CPU | The decoded input of the current photo | Until another photo is selected |
| CPU | The pyramids of the prefetched neighbors, an LRU with a 768 MB cap | The session |
| CPU and disk | Embedded previews | The session (the disk tier is the `cache` crate's) |

Nothing is written next to the original files, and nothing is written to them.

## Edge cases

- **LinearizationTable overrun:** clamped to the last entry and counted (M0).
- **Per-position black levels** (36 for X-Trans) and per-channel white levels: arrays everywhere, never averaged.
- **The fourth white balance coefficient** is ignored (NaN on every body tested).
- **ActiveArea and DefaultCrop** are applied at decode. The CFA phase is taken *after* the active area offset, so an odd offset does not swap R and B.
- **Older ProRAW** (iPhone 12 Pro) is not white balanced in the camera. AsShotNeutral is applied like any DNG's, and nothing assumes 1, 1, 1.
- **Lossy JPEG XL ProRAW** decodes within one table step of libjxl (M0). The goldens use our decoder's output, so this is not a test failure.
- **Images smaller than one tile:** one tile with an apron, clamped at the border.
- **Huge images:** the decoders' 400M-sample cap and fallible allocation hold. A larger file is *unsupported*, not a crash.
- **Negative values after the matrix** are kept through stage 7. Only the gamut compression and the clamp in stage 9 remove them.
- **NaN and infinity** are never written. Divisions guard their denominators, and a test feeds zero and saturated pixels through every stage.
- **Clipping:**
  - Saturated pixels in every channel come out neutral white.
  - A pixel clipped in one channel only is clipped at the first-saturating level (stage 2) and desaturated (stage 4).
- **Monochrome sensors** go through the linear path with one sample. White balance does not apply, and the white balance controls are disabled.
- **Non-RAW:**
  - **Alpha** is ignored; the image renders opaque.
  - **ICC profiles** (Adobe RGB, ProPhoto RGB and others) go through moxcms.
  - **CMYK JPEG** is unsupported in M1.
  - **A 16-bit TIFF or PNG** keeps its 16 bits through decode.
- **HDR gain maps** in HEIC and JPEG are ignored; the SDR base image is used (spec).
- **Orientation:**
  - All eight EXIF values, mirrored ones included.
  - A RAW file's orientation is applied after demosaic, never to the mosaic.
- **A 90° turn after a crop** keeps the cropped region (Data: Crop).
- **The straighten angle with a crop that no longer fits:** the crop shrinks about its center, and is never moved off the image.

## Acceptance criteria

Each criterion names the check that proves it. These become the tests. "Corpus" means the local files in `corpus/` (gitignored, the owner's machine). "CC0 set" means the raw.pixls.us files CI fetches by sha256.

1. **Decode coverage.** Every corpus file in camera priority tiers 1 to 3 decodes through `imgio::decode` with levels, a matrix and white balance. *Check:* the gap report becomes a corpus test (local). Its CC0 subset runs in CI.
2. **ProRAW samples.** The iPhone 12 Pro, 13 Pro Max and 16 Pro files match the references M0 recorded:
   - identical samples to libjpeg, for the lossless JPEG files;
   - libjxl's clamp count and means, for the JPEG XL files.

   *Check:* `dng_corpus.rs`, extended to go through `imgio::decode`.
3. **Demosaic against the references.** RCD against the CPU port of `rcd.c`, and Markesteijn against its CPU port: max ΔE2000 below 2, and at most 0.01 % of pixels above 1. That is M0's tolerance, measured on the synthetic scene and on one CC0 file per CFA type. *Check:* ground-truth tests on every adapter, lavapipe included.
4. **Stages 1 to 4 against Adobe.** Adobe DNG Converter's linear DNG of a file is run through our stage 4 and output transform. The same raw file is run through our stages 1 to 4. The two are compared with the same white balance:
   - mean ΔE2000 ≤ 1.0, and the 99th percentile ≤ 5 (the demosaic algorithms differ at edges).
   - These numbers are a target. The PR that first runs the test records the measured values here, with the reason for any change.

   *Check:* corpus test (local; the references are 58 to 350 MB each).
5. **Golden images.** Each stage on the CC0 set matches its stored golden image on lavapipe: mean ΔE2000 ≤ 0.5, max ≤ 2, PSNR ≥ 45 dB. How they are measured depends on what the stage outputs (`backend/crates/golden`):
    - **Linear data** (camera RGB in stages 0 to 3, scene-linear AP1 in stage 4) is compared over its full float range, negatives and values above 1 included. PSNR uses a peak of 1.0, the normalized white level, and the largest absolute error is reported. ΔE2000 is taken through a fixed view: a matrix into linear sRGB (the identity for camera RGB, which only has to stay fixed, not be accurate), a fixed exposure, a clip, then sRGB encoding.
    - **Display output** (after stage 9) is compared directly as encoded sRGB.
    - Goldens are small (about 256 px), stored losslessly as f16, and written only on lavapipe or by CPU code; `scripts/accept-goldens.py` refuses renders from any other adapter.
 The same goldens hold on real GPUs. *Check:* the golden harness in CI on every PR, and `scripts/gpu-check.sh` on the RTX 3060 before merging pipeline changes. Changing a golden needs a stated reason in the PR.
6. **No tile seams.** A 3000 × 2000 image rendered with 1024 px tiles and as one tile differs by at most 1 code. *Check:* a pipeline test on lavapipe.
7. **Fit matches 1:1.** The Fit render and the 1:1 render downsampled to Fit differ by mean ΔE2000 ≤ 1.0, with Highlights at −100 and Shadows at +100, which is where local tone mapping shows. *Check:* a pipeline test.
8. **Non-RAW is an identity when unedited.**
   - An sRGB JPEG, an sRGB PNG, and a 16-bit sRGB TIFF render within 1 code of their decoded values.
   - A Display P3 HEIC renders to a P3 frame within 1 code.
   - The test images include the target's red, green and blue primaries at full saturation, and colors between the compression threshold and the gamut edge, so a compression that should not run would fail.
   - An Adobe RGB JPEG matches moxcms's conversion to within ΔE2000 0.5.

   *Check:* pipeline tests.
9. **Gamut compression.** Over a sweep of AP1 colors, no output channel leaves 0 to 1 before quantizing. Colors inside the target gamut by more than the threshold change by at most 1 code. Compressed colors move in OkLCh hue by at most 3°. *Check:* a `color` crate test against the CPU reference math, and a GPU test matching it.
10. **White balance math.**
    - Temp and Tint to white point and back round-trips within 1 K and 0.1 tint.
    - AsShotNeutral to Temp and Tint and back is within 1e-4.
    - D65 lands at 6504 K ± 2 K. Illuminant A, a blackbody and so on the Planckian locus, lands at 2856 K ± 2 K and tint 0 ± 0.5.

    *Check:* `color` crate tests.
11. **Clipped highlights are neutral.** A synthetic scene with a patch clipped in one channel, and one clipped in all, renders the clipped area with OkLCh chroma ≤ 0.01, and its edge with no hue band. *Check:* a pipeline test.
12. **Tone controls behave.**
    - Middle gray stays at 0.18 under Contrast at ±100.
    - Highlights at −100 lowers the brightest stop by at least 1 EV while leaving middle gray within 0.05 EV.
    - Shadows mirrors that.
    - Exposure +1 doubles linear values before the base curve.

    *Check:* pipeline tests on the synthetic scene.
13. **GPU memory.** The accounted allocations for a 48 MP image stay under 1 GB, with the breakdown above as the expected values ±10 %. *Check:* the memory-accounting test (every texture and buffer goes through one tracking allocator), plus the bench kit's GPU process counter on the RTX 3060 (peak under 1 GB).
14. **Performance.** Each row is checked by the bench listed in the plan's M1 budget table, on the RTX 3060, with `scripts/gpu-check.sh`:
    - GPU time under 16 ms at 2560 px for every M1 slider, Highlights and Shadows included;
    - full fit render under 1 s at 12 MP and 2 s at 48 MP;
    - 1:1 newly exposed area under 100 ms.

    Each stage also has a criterion benchmark.
15. **Fuzzing.** Every decoder path (linear DNG, the rawler path, JPEG XS, JPEG, TIFF, PNG, HEIC) has a cargo-fuzz target. Each runs 10 minutes with no crash, hang or out-of-memory in the PR that adds it.
16. **Errors keep the app alive.** An injected out-of-memory, from a lowered device limit, returns an error. The retry frees the caches, the render thread stays alive, and the next photo renders. *Check:* a pipeline test (extends PR #22's).
17. **Unsupported files.** A Nikon HE file in a build without the `jpeg-xs` feature, a CMYK JPEG, and a DNG with a non-optional unknown opcode each show their embedded preview and the reason, and disable the controls. *Check:* engine tests; a Playwright flow against the fake engine for the UI part.

Apple Photos exports are a target for how a photo should look by default (DEV-02's base curve), not a pixel reference. No test compares against them.
