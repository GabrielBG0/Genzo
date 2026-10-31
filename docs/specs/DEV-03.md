# DEV-03 · Histogram and clipping warnings

The histogram sits at the top of Develop's right panel, above the tool strip (screen D1, the handoff's "Histogram + tool strip" component). It shows the photo's R, G and B distribution, the EXIF row under it, and two corner markers that light when shadows or highlights clip. J shows the clipped pixels on the photo. Alt while dragging a tone slider shows only what clips. M1 builds all of it (development plan, M1 backend step 7 and frontend step 5).

Sources: the spec's feature table ("Histogram, clipping warnings, before and after", M1), FEATURES.md's DEV-03 entry, screen D1 and the token set of the UI handoff (`label.red` for highlight clipping, `label.blue` for shadow clipping), and `docs/specs/DEV-01.md`. Where this spec and the technical spec disagree, the technical spec wins.

**What other specs own:** before and after (`\`) is WF-02; the tone sliders and their math are DEV-02 and DEV-01; the frame, its target and stage 9 are DEV-01.

**Out of scope for M1:**
- clicking or dragging in the histogram to change tones (M3, with the tone curve, DEV-04);
- the histogram in Library (M2);
- a luminance-only histogram mode.

## Behavior

### What the histogram measures (owner, 2026-10-30)

- **What the screen shows.** The histogram counts the codes of the frame stage 9 writes, in the frame's own target (sRGB or Display P3, DEV-01, *Stage 9*): after the output transform, after gamut compression, and after the dither when the frame is dithered. A color that gamut compression brings inside the target is not counted as clipped, because it is not clipped on screen. On a Display P3 monitor, colors inside P3 but outside sRGB do not warn, though an sRGB export would clip them. The M1 export (OUT-01) says so in its own spec.
- **The whole photo, after crop.** The histogram is of the shown image (DEV-01, *Geometry*): the crop, rotation and flip applied, and nothing outside the crop counted. It does not depend on zoom or pan.
- **At Fit**, the histogram comes from the frame itself: every pixel of the Fit frame is counted while it is written. While dragging at Fit, the frame is the half-size drag preview (DEV-01, *Re-runs*, Interactive), so the histogram updates with each drag frame from its pixels.
- **At 1:1** (owner, 2026-10-30), only the visible tiles are rendered, so the histogram comes from a second, whole-photo render at the Fit size. It runs after the visible tiles' settled render, at the lowest priority, and a newer request cancels it. While dragging at 1:1, the histogram keeps its last values and updates once on release.
  - **No new GPU memory.** The whole-photo render reuses the view's stage caches (DEV-01, *GPU memory*; the Fit size is never larger than the view), since a second set at 2560 × 1440 would take about 88 MB of the 10 MB reserved. It writes counts only, no frame. Afterwards, the caches no longer hold the 1:1 view, so the next drag or pan at 1:1 first samples the view again from the level-0 tile cache, whose tiles are still there. That costs at most the 1:1 cold render, 5.9 ms for 48 MP Bayer on the RTX 3060 (DEV-01, the 1:1 bench, which also renders the tiles). The bench of criterion 9 measures the resample itself.
  - **When the Fit frame reads level 0** (Decided: "Fit views larger than level 1": a photo whose level 1 is smaller than its Fit frame, such as 24 MP on a 3840 × 2160 view), a whole-photo render at the Fit size needs the whole image's level-0 tiles: 384 tiles for 6000 × 4000, against the cache's 448 slots, which the 1:1 view's own tiles share. **So that render reads pyramid level 1 at its own size instead** (owner, 2026-10-30), and its histogram is close to the Fit one but not bin for bin (criterion 4). Reading level 0 would evict the 1:1 view's tiles, which the next pan would render again (47 ms cold for 40 MP X-Trans); the histogram is a guide, and the pan's speed matters more.
  - The Fit size used is the one the view would have at Fit, so going from 1:1 back to Fit shows the same histogram, except in the level-0 case above, where the two are close but not equal.
- **Counted per channel, 256 bins each.** Bin `n` of channel R holds the number of pixels whose R code is `n`, as the frame stores it (RGBA8). The counts are exact integers; nothing is sampled or smoothed.

### What counts as clipped (owner, 2026-10-30)

- **A pixel's highlights clip** when any of its R, G and B codes is 255. **Its shadows clip** when any of them is 0.
- A pixel can clip in both: for example, a saturated blue with R at 0 and B at 255. So a photo with fully saturated colors lights both markers, as in Lightroom: pure sRGB red (255, 0, 0) clips its highlights in R and its shadows in G and B.
- The engine reports, for highlights and for shadows separately, the number of clipped pixels and which channels clip at all.

### The histogram panel

1. **The plot.** The three channels are drawn as filled areas over the 256 bins, from dark at the left to light at the right. Where channels overlap, the colors mix as the handoff draws them (two channels give the secondary color, three give gray). The colors come from the token set.
2. **Height.** The plot is scaled so that the tallest bin from 1 to 254 fills it. Bins 0 and 255 can be taller and are cut at the top, so that a clipped spike never flattens the rest of the plot. When every bin from 1 to 254 is empty (each pixel at 0 or 255 in every channel), the tallest of bins 0 and 255 fills it instead. With no pixels counted, the plot is empty.
3. **The corner markers.** A triangle in the top-left corner (shadows) and one in the top-right corner (highlights).
   - Dim when nothing clips.
   - Lit when something clips, in the color of the clipped channels: one channel's own color, or the mix of two. White when all three clip.
   - A click on a marker turns its overlay on or off (see *The J overlay*). A marker whose overlay is on has a ring (the handoff's selection indicator).
   - The tooltip says "Shadow clipping (J)" or "Highlight clipping (J)", and the share of pixels clipped, such as "0.4 % of pixels".
4. **The EXIF row** under the plot, in JetBrains Mono, as the handoff shows it: ISO, focal length, aperture and exposure time.
   - Formats: `ISO 100`, `24 mm` (one decimal below 10 mm: `6.9 mm`), `f/2.8` (one decimal, `f/8` with no `.0`), and `1/250 s` for times under 1 s (the denominator rounded), `1.3 s` and `30 s` otherwise (one decimal below 10 s).
   - The focal length is the lens's own, not the 35 mm equivalent.
   - A value the file does not carry is left out with its separator. When none is there, the row is empty but keeps its height, so the panel does not jump.
5. **While the photo is Opening** (the embedded preview is shown), the histogram is of the preview, as shown. The engine computes it on the CPU from the prepared preview (`engine::preview`). The markers work, but J has no effect until the first frame: the overlay is drawn by stage 9.

### Where the data come from

- **The counts** come from stage 9's output shader, and reach the frontend in a `HistogramReady` event (see *Inputs and shortcuts*). Each pixel it writes adds to the three bins of its codes and to the clipping counts, using workgroup-local atomics merged into one storage buffer per frame (3 × 256 + 4 counters, about 3 KB). The buffer is read back with the frame. It fits the 10 MB the GPU budget already reserves for the histogram and masks (DEV-01, *GPU memory*; `pipeline::budget::HISTOGRAM`).
- **The clipping test** runs on the codes before the overlay: what is counted is the photo, never the overlay's colors.
- **The EXIF values** come from `imgio`, which reads them from the EXIF IFD that `imgio::capture` already reads for the capture time: ExposureTime (tag 33434), FNumber (33437), the ISO (PhotographicSensitivity, 34855) and FocalLength (37386). The reader grows to return them with the capture time, for every format it covers (JPEG, TIFF and RAW files whose IFD0 is TIFF, PNG, HEIC, CR3 and the embedded preview's EXIF). It needs no new dependency.

### The J overlay

- **J turns both overlays on or off together.** A marker click turns one on or off (see above). **Their state stays on across photo switches** (owner, 2026-10-30), as in Lightroom, so a whole shoot can be checked for clipping: it holds until J or a marker turns it off. It is the one exception to the Decided row "Module state (APP-01)". Leaving Develop turns both off, as that row says, and nothing is saved when the app closes.
- **A setting from M2.** The Preferences window (APP-11, M2) gets an Interface option, "Keep clipping overlays on when switching photos", on by default. Turned off, each photo starts with both overlays off. M1 has no settings store, so it always keeps them on.
- **What it shows:** on the photo, every pixel whose highlights clip is drawn in `label.red`; every pixel whose shadows clip is drawn in `label.blue`. A pixel that clips in both is drawn red.
- **Drawn by stage 9 into the frame.** The overlay is part of the frame's pixels, so it needs no second image over the binary tile protocol, and it is right at Fit and at 1:1, while dragging too. The frontend sends the two colors from its token set with the overlay state (`SetClippingOverlay`, below), so no color is hard-coded in the engine. The overlay is flat, with no blend or blur.
- Turning an overlay on or off renders the frame again from the stage 7 cache, through stage 9 only (DEV-01, *Re-runs*).

### Alt while dragging a tone slider: clipping preview

While Alt is held during a drag of one of these sliders, the photo shows only what clips:

| Slider | Shows | Background |
|---|---|---|
| Exposure, Highlights, Whites | highlight clipping | black |
| Shadows, Blacks | shadow clipping | white |

- **Each pixel** is drawn in the color of its clipped channels: for highlights, a channel that is 255 is shown at full value and the others at 0 (red, yellow, white and so on); for shadows, a channel at 0 is shown at 0 and the others at full value (cyan, blue, black and so on), so that a pixel clipped in all three is black. A pixel that does not clip shows the background. These are the photo's own channel colors, not UI colors.
- Stage 9 draws it, as it draws the J overlay. The drag preview's half size and every other drag rule stay as DEV-01 sets them; only stage 9's output changes.
- **How it is sent.** The frontend sends `SetClippingPreview { preview }` with `Highlights` or `Shadows` when Alt goes down during the drag of one of these sliders, and `None` when Alt goes up. It is its own command, not a flag on `ApplyEditDelta`, because pressing or releasing Alt without moving the slider sends no delta. Each one renders again through stage 9 only.
- **Releasing Alt** during the drag shows the photo again at once; pressing it again brings the preview back. Releasing the slider ends the preview whether Alt is held or not: the engine clears it when the gesture ends, so a lost key-up never leaves it on.
- The other sliders ignore Alt: the frontend sends no preview for them, and the drag behaves as without Alt.
- The histogram keeps counting the photo, not the preview.

## Inputs and shortcuts

Every command is in the shortcut registry, with its shortcut shown in its tooltip and menu item.

| Command | Shortcut | Notes |
|---|---|---|
| Show clipping (both overlays) | J | Develop only; also in the View menu and the photo's right-click menu (APP-06) |
| Shadow clipping overlay | click the left marker | |
| Highlight clipping overlay | click the right marker | |
| Clipping preview | Alt while dragging Exposure, Highlights, Whites, Shadows or Blacks | `SetClippingPreview` |

**Contract** (M1, contract 0.5.0):
- `GetHistogram { photo }`, a query, as the plan lists it: the latest counts of the shown photo, or none.
- `HistogramReady { photo, generation, histogram }`, a new event, sent whenever the counts change: with each frame at Fit (drag frames included), after the whole-photo render at 1:1, and for the embedded preview while Opening. At 1:1 it comes after the `RenderFinished` of the visible tiles, since its render runs later and produces no frame. The counts are numbers, not pixels, so they travel as JSON: three arrays of 256 integers, the highlight and shadow clipping counts with their channels, and the number of pixels counted.
- `SetClippingOverlay { shadows, highlights, shadow_color, highlight_color }`, new: the overlays' state and their colors as RGBA8.
- `SetClippingPreview { preview }`, new: `None`, `Highlights` or `Shadows`, for the Alt preview during a drag.
- The EXIF values travel with the photo's metadata in `ListFolderPhotos`.

## States

| State | What the histogram shows | J and the markers |
|---|---|---|
| **No photo selected** | An empty plot, markers dim, the EXIF row empty | J does nothing |
| **Opening** (the embedded preview is shown) | The preview's histogram, the EXIF row filled | The markers work; J can be turned on, and is drawn from the first frame |
| **Opening, no embedded preview** | An empty plot until the first frame | As Opening |
| **Ready, at Fit** | The frame's histogram, live while dragging | Everything |
| **Ready, at 1:1** | The whole photo's; while dragging, the last one until release | Everything |
| **Unsupported or damaged file** (its embedded preview is shown, DEV-01) | The preview's histogram, the EXIF row filled when the file has EXIF | The markers work; J does nothing |
| **Unsupported, no preview** | An empty plot | J does nothing |
| **Software adapter, rendering** | As Ready; the histogram follows the frames as they come | Everything |

## Data

Nothing is stored in M1. The overlay state is the view's: kept across photo switches, reset when leaving Develop, and gone when the app closes. From M2, the APP-11 setting above is stored with the other preferences. The histogram is computed again with every frame. The EXIF values are read from the file each time it is opened.

## Edge cases

- **Monochrome and B&W:** the three channels are equal, so the plot shows one gray area and the markers are white when they light.
- **Gray non-RAW files** (JPEG, PNG): the same, since the frame is RGB.
- **A tiny photo** (a side under 32 px, which DEV-01 shows whole): its Fit frame is enlarged to fit (DEV-01, *Geometry*; owner, 2026-10-30); each frame pixel is counted, so the histogram is of the enlarged frame. Its shape is the photo's.
- **Dither:** a value between two codes is dithered to either, so a value just under 255 can count as clipped in some pixels. This is what the screen shows.
- **A file whose EXIF values are odd:** an exposure time or f-number of 0, or a rational with a zero denominator, is left out. An ISO of 65,535 (the EXIF cap) is shown as it is; the extended ISO tags are M2.
- **Several ISO values** (a list in tag 34855): the first is shown.
- **Rotation and crop:** the histogram counts only the shown image. A rotated crop's corners are inside the image (DEV-01, *Geometry*), so no padding is ever counted.

## Acceptance criteria

1. **The counts are exact.** For frames rendered on lavapipe from the synthetic scene and from the decoder's fixtures, at Fit and in the drag preview, the GPU's counts equal a CPU count of the frame's read-back bytes, bin for bin, and the clipping counts and channels equal the CPU's. *Check:* a pipeline test.
2. **What clips.** A synthetic frame with patches at codes (255, 128, 128), (255, 255, 255), (0, 128, 128), (0, 0, 255) and (128, 128, 128) gives highlight counts for the first, second and fourth patches, shadow counts for the third and fourth, the right channels for each, and none for the gray. *Check:* a pipeline test.
3. **Clipping is counted after gamut compression.** A synthetic Display P3 source of colors that a plain matrix conversion to sRGB puts below 0 or above 1 in a channel, and that compression brings to codes 1 to 254 (both checked on the CPU reference), shown in an sRGB frame, counts no clipped pixel. The same colors with compression off would count them. *Check:* a pipeline test.
4. **The whole photo at 1:1.** At 1:1, panned to a corner, the histogram equals the one at Fit for the same edit and view size, bin for bin, for a photo whose Fit frame reads level 1 (see *At 1:1* for the level-0 case). After the whole-photo render, a drag at 1:1 renders the same frame it would have rendered without it. *Check:* an engine test.
5. **The overlay.** With J on, every pixel whose highlights clip is `highlight_color` in the frame and every pixel whose shadows clip is `shadow_color`; red wins for a pixel that clips in both. Every other pixel is unchanged, byte for byte. The histogram is the same with the overlay on or off. *Check:* a pipeline test, and a golden image of the synthetic scene with both overlays.
6. **The clipping preview.** A drag frame with `Highlights` is black with each highlight-clipped pixel in its channel colors; with `Shadows` it is white with each shadow-clipped pixel in its colors; with `None` it is the normal frame. Ending the gesture with the preview still set gives the normal settled frame. *Check:* a pipeline test, an engine test for the end of the gesture, and a golden image for each of the two previews.
7. **The EXIF row.** The new fields are read from the same IFD as the capture time, so they cover the files `capture_time` reads (`imgio/tests/capture_corpus.rs`; the formats it handles: JPEG, TIFF-based RAW and DNG, ORF, RW2, CR3, PNG, RAF and HEIF). For each such corpus file and each decoder fixture with EXIF, ISO, exposure time, f-number and focal length equal what ExifTool reports. *Check:* an `imgio` corpus test, with expectations written by a new `scripts/exif-reference.py` from ExifTool's output, as the other corpus tests take theirs from reference tools. ExifTool is not installed on the reference desktop yet; the script needs it, and its version goes in the expectations file.
8. **Formatting.** 1/250 s, 1/8000 s, 0.3 s shown as 1/3 s, 1.3 s, 30 s; f/1.8, f/8, f/11; 6.86 mm shown as 6.9 mm, 24 mm, 105 mm; missing values left out. *Check:* a frontend unit test against the fake engine.
9. **It costs little.** On the RTX 3060, counting adds under 0.2 ms to stage 9 at 2560 × 1440, and the slider budgets of `scripts/gpu-check.sh` hold within its 10 % rule. The whole-photo render at 1:1 takes under 20 ms at 48 MP, and the resample after it is measured. *Check:* a criterion bench of stage 9 with and without counting, of the whole-photo render and of the resample, and `bench gpu`.
10. **The panel.** Against the fake engine: the plot scales to the tallest bin from 1 to 254, the markers light in the clipped channels' colors and toggle their overlays, J toggles both, the tooltips give the share of pixels, and the empty states show as listed. *Check:* Vitest and Playwright.
11. **The GPU budget holds.** The memory-accounting test (DEV-01) still passes with counting on and the 1:1 whole-photo render done, with no new item in `pipeline::budget`. *Check:* `pipeline/tests/memory.rs`.
12. **Fuzzing.** The EXIF reader's new fields are covered by the existing `capture_time` fuzz target's inputs, and the target asserts that reading them never panics. *Check:* the fuzz target, run for 10 minutes before merge.
