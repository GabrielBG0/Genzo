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
- **The embedded preview**, also through `imgio::decode_preview`.
- **The gain table map**, read and kept, never applied in M1.

Which decoder reads which format:

- **Linear DNG** (every ProRAW generation, Samsung DNG): `dng::decode_linear`. The LinearizationTable is applied exactly, and samples past its end take its last entry.
- **CFA DNG and camera RAW:** rawler. Our own TIFF tag reader adds the DNG tags rawler leaves empty: the opcode lists, ForwardMatrix and the gain table map.
- **Nikon High Efficiency:** the JPEG XS decoder (decided), behind the `jpeg-xs` cargo feature that release builds leave off (patent pool, see `docs/licenses.md`). Without the feature, and until the decoder lands, these files are *unsupported* (see States).
- **HEIC:** libheif. **JPEG:** zune-jpeg (the spec's choice). **TIFF, PNG:** pure-Rust decoders chosen in the implementation PR, after a license check.

Large linear DNGs decode their tiles in parallel and stream them into stage 1. No full-size `u16` copy is held beside the input, so a 48 MP ProRAW peaks below the 445 MiB measured in M0.

**Opcodes.** M1 applies what the corpus contains. The first DEV-01 implementation PR lists every opcode in the corpus by file, list and optional flag, and adds the result here.
- GainMap (OpcodeList2, used by Samsung and Pixel phones) is applied in M1 whatever the inventory shows.
- An opcode M1 does not implement is skipped when it is marked optional.
- When an unimplemented opcode is not marked optional, the photo is *unsupported*.
- The lens opcodes (WarpRectilinear, FixVignetteRadial) are skipped while marked optional, and stage 5 takes them up in M3.

### Stage 1: linearize

Runs on the GPU, per tile of the sensor image (see Tiling).
- OpcodeList1 runs first, on the stored values.
- Each value becomes `(v − black[position]) / (white[channel] − black[position])` as `f32`. Negative values are kept: they are noise, and clamping them would lift the shadows.
- Then OpcodeList2 runs.

The LinearizationTable is already applied by the decoder for linear DNG, and by rawler where it applies one.

### Stage 2: white balance and highlight clipping

- **White balance.** Each channel is multiplied by its white balance multiplier, from the Temp and Tint of the edit (see Data). The multipliers are normalized so the smallest is 1.
- **Highlight clipping.** Every channel is clipped at the level where the *first* channel saturates after white balance: `clip = min(multiplier[c])` in normalized units. A clipped area therefore comes out neutral white, not magenta or cyan. Real reconstruction of clipped color is later (decided 2026-10-18).

While a slider is dragged, white balance runs as a ratio after the cached demosaic (stage 4). The settled render re-runs this stage and the demosaic with the new multipliers, as M0 does.

### Stage 3: demosaic

- **Bayer: RCD,** the M0 port of darktable's `demosaic_rcd.cl`, with the one documented change. It is checked against the CPU port of `rcd.c`.
- **X-Trans: Markesteijn, 3 passes,** a WGSL port of darktable's `xtrans_markesteijn_interpolate`. It gets a literal CPU port as its test reference, like RCD. Interactive drags never re-demosaic, so the slower 3-pass variant only costs time on the settled render.
- **Linear DNG, monochrome planes and non-RAW** skip this stage. A monochrome plane is copied to R, G and B.
- OpcodeList3 runs after the demosaic, on camera RGB, before the matrix.

### Stage 4: camera matrix

- **The white point.** It comes from the edit's Temp and Tint (Data: *White balance*).
- **Camera to XYZ:**
  - With ForwardMatrix tags, they are interpolated by the white point, as the DNG specification describes.
  - Without them, ColorMatrix1 and 2 are interpolated by inverse correlated color temperature between their calibration illuminants. The result is inverted, then adapted from the white point to D65 with the Bradford transform.
- **XYZ to the working space:** XYZ (D65) to linear ACES AP1 primaries with a D65 white (decided 2026-10-14). The pyramid is stored as RGBA16F.
- **Non-RAW files** enter here: decoded values are linearized with their transfer function and converted from their primaries or ICC profile (moxcms) into the working space.
- **BaselineExposure** is applied here as a gain of 2^BaselineExposure (+2.50 EV on the iPhone 13 Pro Max file, so it decides the default look).
- **Highlight desaturation.** Pixels whose largest camera channel lies within the top 5 percent below the clip level are blended toward their own luminance, reaching neutral at the clip level. The edge of a clipped area then carries no color fringe.

The output of stage 4 builds the mip pyramid (see Tiling).

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

- **Tiles** are 1024 × 1024 output pixels for Bayer, linear and non-RAW input, and 512 × 512 for X-Trans. Each has a 24 px apron on each side: 24 is a multiple of both CFA periods, 2 and 6, and wider than RCD's and Markesteijn's support. The apron is computed and thrown away, so no tile edge shows. X-Trans tiles are smaller because 3-pass Markesteijn keeps several directional RGB buffers, about 200 bytes per pixel against RCD's 49.
- **The input** stays in CPU memory. Each tile's slice is uploaded through a staging buffer.
- **The pyramid** is image-wide and starts at **level 1** (half size), in RGBA16F. Each tile's stage 4 output is reduced into level 1 with a 2 × 2 box filter, and levels 2 and up follow. **Level 0 (full resolution) is never kept for the whole image.**
- **At 1:1 and during export**, level-0 tiles of 256 px are made on demand by running stages 1 to 4 on the region they need, apron included.
  - At 1:1 they are cached in an LRU with a fixed byte cap.
  - Order: visible tiles first, then a ring of one tile around them.
  - Export streams tiles through the whole pipeline and keeps none.
- **GPU memory for one 48 MP edit** (8064 × 6048) has the following worst case. The memory-accounting test asserts the total. Each cap is a fixed constant that the test checks.

| Item | Size |
|---|---|
| Pyramid, levels 1 and up, RGBA16F (98 MB for level 1, plus a third) | 131 MB |
| Stage 1 to 4 scratch for one tile, intermediates included: Bayer 49 B/px × 1072² = 56 MB; X-Trans about 200 B/px × 560² = 63 MB | 63 MB |
| Upload staging for one tile (linear DNG, 3 × u16) | 7 MB |
| Level-0 tile cache at 1:1 (fixed cap; about 480 tiles of 256², five 2560 × 1440 views with their rings) | 256 MB |
| Viewport stage caches (fixed cap; stage 4 sampled, stage 6, stage 7 at 2560 × 1920 RGBA16F, plus the base layer) | 128 MB |
| Output frame and two persistent readback buffers, RGBA8 | 60 MB |
| Histogram and clipping masks | 10 MB |
| **Total** | **655 MB**, under the 1 GB budget with 345 MB to spare |

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
- **As shot:** the white point is solved from AsShotNeutral, or from the camera's white balance coefficients, with the iterative method in the DNG specification. The matrix depends on the white point, so the two are solved together. Then it is expressed as Temp and Tint.
- **The multipliers** are the inverse of the camera neutral at that white point.
- **A file with no as-shot data** (the Kodak DCS760C) uses D65, and the status bar says the as-shot white balance is unavailable.

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
