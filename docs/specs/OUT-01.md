# OUT-01 · Export (M1 part)

M1's export writes the photo open in Develop as a JPEG file, rendered through the same pipeline as the viewport. The user picks where, the size, and the quality. It runs in the background while editing goes on. M4 builds the rest of the W7 export view (presets, the queue of several photos, other formats, naming templates, sharpening, watermarks).

Sources: the technical spec (export renders through the same GPU pipeline, in tiles; the budget "Export, 24 MP JPEG: 1 per second or better"), the development plan (M1 backend step 10, frontend item 14, contract 0.6.0: `ExportJpeg`, `ExportProgress`), FEATURES.md's OUT-01 entry, screen W7 of the UI handoff, `docs/specs/DEV-01.md` (stage 9 and the export target, *Tiling and the mip pyramid*), and `docs/specs/DEV-03.md`. Where this spec and the technical spec disagree, the technical spec wins and this file is corrected.

**Owner decisions (2026-11-07):**
- M1 exports **the current photo only**. Exporting several photos at once needs a selection of several, which M2's Library brings; the export queue that runs them is M4's (W7).
- **Size:** the full cropped image, or *Resize to fit* a long edge in pixels, never enlarged.
- **Metadata:** all of the source's EXIF, **location (GPS) included**, with the orientation reset. Removing location is an M4 option.

**Out of scope for M1**, with their milestone:
- export presets, the queue of several photos and size estimates: M4 (W7);
- AVIF, TIFF, PNG and DNG; email and "another app" destinations; file naming templates: M4;
- output sharpening, watermarks, the *Remove location* and *Remove people names* options, resolution (ppi): M4;
- color spaces other than sRGB: M4;
- a limit on file size: M4.

## Behavior

### Starting an export

- **Export** is the quick action in Develop's top bar (APP-03) and the shortcut Ctrl Shift E (macOS: Cmd Shift E). In M1, the Export module tab (APP-01) opens the same dialog. It is disabled when no photo is open, or the open photo does not render (DEV-01, *States*).
- The dialog, a subset of W7's *File settings*, *Image sizing* and *Location*:
  - **File:** the destination path, shown with a *Choose…* button that opens the system save dialog. The default name is the photo's file name with `.jpg` for its extension (`DSC_0412.NEF` → `DSC_0412.jpg`). The default folder is the last one exported to this session, or else the photo's own folder.
  - **Format:** JPEG, shown and fixed.
  - **Quality:** 1 to 100, default 85 (W7).
  - **Color space:** sRGB, shown and fixed.
  - **Size:** *Full size* (default), or *Long edge* with a number of pixels (default 2048, W7's). A long edge larger than the photo's exports at full size: never enlarged (W7's *Don't enlarge*, always on in M1).
  - **Metadata:** one line of text, "All EXIF metadata, including location. XMP and IPTC are not included.", so the user knows the GPS position goes out with the file, and that keywords or captions kept only in XMP or IPTC do not.
  - **Export** and **Cancel** buttons. Enter exports; Esc cancels.
- The dialog keeps its values for the session: the next export opens with the last quality, size and folder.

### What is exported

- **The photo's edit at the moment Export is pressed.** Later edits, undo, and switches to another photo do not change an export in progress.
- **The shown image** (DEV-01, *Geometry*): crop, straighten, quarter turns and flip applied, orientation upright. Nothing outside the crop.
- **Color:** stage 9 with sRGB as the target (DEV-01, *Stage 9*): the output transform, the gamut compression, the sRGB transfer function and the 8-bit dither, as the viewport draws it on an sRGB monitor. The file embeds an sRGB ICC profile.
  - On a Display P3 monitor, the viewport and the histogram (DEV-03) show colors that sRGB cannot hold. The export brings them inside sRGB by the same gamut compression, so some saturated colors come out less saturated than on screen, without a clipping warning there.
- **An unedited non-RAW photo** follows stage 9's own rule: not dithered, so its codes go out as the viewport shows them. A JPEG in sRGB re-exported unedited at full size keeps its pixel values, re-encoded at the chosen quality.

### How it renders (`export`, `engine::export`)

An export is a job that holds everything it reads. It never uses the open photo's pyramid, level-0 cache or view textures. So a switch to another photo, or a drag, does not disturb it, and it does not evict what the viewport holds.

- **What the job holds:**
  - The photo's decoded input, shared with the engine (`Arc`), so a switch does not free it. A photo opened from a prefetched level 1 whose input is not decoded yet (LIB-09, *Prefetch*) starts its export once that decode attaches.
  - A copy of the photo's base layer for Highlights and Shadows (DEV-01, stage 6), taken when Export is pressed. The base layer is built from the pyramid, which the job does not keep. Its coefficients take 9.4 MB at the 1536 px working size.
  - The edit and the geometry, as they were.
  - Its own GPU textures for one region at a time (see *GPU memory*).
- **Full size:** the output is rendered in regions of 1024 × 1024 output pixels, stages 1 to 9, as the 1:1 view renders its tiles (their stage 1–4 tiles are the configured sizes, 512 px for X-Trans, inside each region) (DEV-01, *At 1:1 and during export*). The dither is anchored at each pixel's place in the shown image. So **a full-size export is, before JPEG encoding, the same codes as the 1:1 view**, pixel for pixel (criterion 1).
- **Resized:** each region is rendered at full resolution through stages 1 to 8. It is reduced to the output size by box means in the working space (linear AP1, before the output transform). Each output pixel is then the mean of the full-resolution pixels it covers, split as `preview::reduce` splits them. Stage 9 runs on the reduced pixels, with its dither anchored at output pixels.
  - That is a definition, not a Fit frame. A Fit frame samples the pyramid (DEV-01, *Readback*), and the job holds no pyramid. Reducing in linear light is the right average for a downscale. A resized export is close to the Fit frame of the same size but not equal to it (criterion 2).
  - **An output pixel can need more than one region:** at a large reduction, the full-resolution pixels it covers span several regions, and at the smallest long edge (32 px, DEV-01's crop minimum) one output pixel covers more than 250 regions of a 48 MP image. So the box sums are not taken inside a region. Each region is read back in the working space and added into an accumulator of the output rows it touches (f64 sums, the output's width by those rows), on the CPU (`genzo_export::resize`). Stage 9 runs on an output row once every region it covers has added into it, on the CPU with DEV-01's reference math (`color::output::pixel`), on a pool of half the cores; then the row goes to the encoder. The accumulator holds the output rows of one row of regions, plus one shared at each edge: at most the output width × (1024 × scale + 2) rows.
  - **The smallest long edge is 32 px**, as DEV-01's smallest crop; the dialog does not go lower.
- **Order:** regions go in rows, top to bottom, left to right, so the encoder receives complete output rows in order.
- **Scheduling on the render thread:** one region per idle slot. The order of priority is:
  1. Develop's renders;
  2. the whole-photo histogram (DEV-03);
  3. the attach decode of the current photo (LIB-09);
  4. export regions;
  5. neighbor prefetch builds.

  A drag waits at most one region; criterion 7 measures what that costs. Every region waits for the rest of a white balance rebuild, which the next part would find stale.
- **No region starts during a photo switch** (`RenderThread::pause_exports`, held by the switch as the thumbnails' pause is): the region in progress finishes, then the export waits until the switch has shown its first pixels and opened the photo. Without the pause, the 48 MP first pixels took about 15 ms more, past their budget; with it, they are as without an export.
- **The export's stages are made once** and kept on the render thread for the next export, within `EXPORT`: made at each export, their textures and pipelines delayed its first region. Prefetch builds wait for the export, so switching during an export can find a neighbor not yet prefetched.
- **One export at a time.** A second Export while one runs is queued behind it, and the dialog says so ("Starts after the export in progress").

### Encoding and writing

- **JPEG:** baseline, quality as chosen. Chroma is subsampled 4:2:0 below quality 90, and kept whole (4:4:4) from 90. The encoder takes rows as they are rendered (`jpeg-encoder`'s interleaved mode), so the job holds one row of regions of the 8-bit output, not the whole image: 33 MB for an 8064 px wide image (8064 × 1024 × 4 bytes).
- **On a one-thread pool**, paused during a photo switch, like the thumbnails (LIB-09), so the encode never delays a switch's first pixels.
- **Written to a temporary file in the destination folder**, then renamed to its name. A cancel, a failure or a crash leaves no half-written file under the destination name. A temporary file left by a crash has a name the next export of the same path removes.
- **An existing file at the destination is replaced only when the user said so.** `ExportJpeg` carries `replace`. The frontend sends it true only for the path the system save dialog returned unchanged, since that dialog has asked already. Otherwise the engine refuses a destination that exists ("DSC_0412.jpg already exists"), and the dialog asks "Replace it?" before sending again with `replace`. The rename then replaces the file in one step.
- **A destination a running or queued export will write** is refused ("An export to this file is in progress"), whatever `replace` says: neither save dialog could have seen that file, which does not exist yet. The user cancels that export or picks another name.
- **The originals are never written** (CLAUDE.md, *Architecture rules*). The engine refuses a destination that resolves, after canonicalization, to any file it has listed this session: the opened folder's, and those of every folder opened before (LIB-09 keeps their ids by canonical path). Exporting `harbor.jpg` over itself, or over a JPEG of a folder opened earlier, is refused with "Genzo does not write over the photos it opens. Choose another name or folder."
- **The extension:** the save dialog offers `.jpg`. A name the user typed without `.jpg` or `.jpeg` (any case) gets `.jpg` added by the frontend. That is no longer the path the dialog checked, so it goes with `replace` false, and a file already there is asked about as above.

### Metadata

- **All of the source's EXIF goes out** (owner, 2026-11-07): IFD0's descriptive tags (make, model, date and time, artist, copyright, software), the Exif IFD (capture time with its sub-seconds and offset, exposure, ISO, aperture, focal length, lens, flash, and the rest), and the GPS IFD, location included.
- **It is rewritten, not copied byte for byte.** A RAW file's IFD0 also holds its image structure (strip offsets, SubIFDs, DNG private data), and a CR3 splits its EXIF across its CMT boxes. So the engine reads the tags from each format and writes a new EXIF segment (`imgio`'s EXIF writer):
  - **Orientation** is 1: the pixels are upright.
  - **Pixel dimensions** (PixelXDimension and PixelYDimension) are the exported size.
  - **Software** is "Genzo" and its version. The source's software tag goes out as ProcessingSoftware.
  - **Dropped:** IFD1 and its thumbnail (stale), image-structure tags (offsets, byte counts, compression, bits per sample, photometric, planar configuration, SubIFDs, DNG tags), and the interoperability IFD.
  - **The MakerNote** goes out as it is when the segment still fits in 64 KB (APP1's limit). When it does not fit, it is dropped, and the export says so in its result ("The camera's maker notes were too large to keep"). Its offsets refer to its own block, so moving it is safe for most cameras. Those that point outside it break, as in other tools.
- **Where each format keeps it:** the EXIF segment of a JPEG, the `eXIf` chunk of a PNG, the EXIF item of a HEIC, IFD0 and its Exif and GPS IFDs in TIFF-based RAW, DNG and TIFF, CR3's CMT1 to CMT4 boxes, and RAF's embedded JPEG. A file whose metadata cannot be read exports without EXIF and says so; the pixels are not affected.
- **The ICC profile** is the sRGB profile, in an APP2 segment.
- **XMP and IPTC** are not written in M1 (M2 writes sidecars; MET-06).

### Progress and cancel

- The export sends `ExportProgress` as each region row is written: its job, its photo, the rows done and their total, and its state.
- The status bar shows "Exporting DSC_0412.jpg, 40 %", then "Exported DSC_0412.jpg" for a few seconds (APP-07). The quick action shows the progress as a ring around its icon.
- **Cancel:** clicking the status bar's progress, or Export again during the export, offers "Cancel export". When the export the status bar follows ends while an older one still runs, the older one's next progress takes the status bar back. A cancelled export stops after the region it is rendering and deletes its temporary file.
- **Closing the app during an export** asks first ("An export is in progress. Quit anyway?", *Quit* or *Keep exporting*, the system's dialog). Quitting cancels every export and waits, at most 3 s, for them to end, so none leaves its temporary file: Tauri ends the process without dropping the engine.
  - **On macOS, the app menu's Quit (⌘Q)** closes the window, so it asks the same question: the shell replaces Tauri's default Quit, which ended the process without asking.
  - **Quitting any other way** (the Dock's Quit, logging out) cannot ask: on the way out the shell stops every export (`Engine::stop_exports`, which waits for each to end), so none leaves its temporary file.

## GPU memory

The job adds its own textures to the 1 GB budget (DEV-01, *GPU memory*; `pipeline::budget`):

- A region's stage 1–4 scratch is the shared `TILE_SCRATCH`. The render thread runs one job at a time, so export uses it between other work and never beside it. Exporting an X-Trans photo while a Bayer one is open (or the reverse) makes that scratch again on each switch between them; it is freed before it is made, so it is never held twice.
- **New, `EXPORT` (80 MB):** a level-0 atlas of 8 × 8 slots of 256 px, enough for a 1024 px region under a 45° straighten (33.6 MB), view stages for one region (29.4 MB), and the base layer's coefficients (up to 9.4 MB at 1536 × 1536). Measured at a whole region: 69 MB, with a base layer of 6.4 MB (`pipeline/tests/export.rs`). The budget's items now sum to 779 MB.

## Inputs and shortcuts

| Command | Shortcut | Also in |
|---|---|---|
| Export | Ctrl Shift E (macOS: Cmd Shift E) | Quick actions (APP-03), the Export module tab (APP-01), App menu › File (APP-02) |
| Export (in the dialog) | Enter | |
| Close the dialog | Esc | |

All are registered in the shortcut registry.

## States

| State | What the user sees | What the user can do |
|---|---|---|
| **No photo open, or one that does not render** | Export disabled; its tooltip says why | Open a photo that renders |
| **Dialog open** | The fields with their last values | Change them, Export, Cancel |
| **A destination that is an original** | The dialog's message, Export disabled | Choose another name or folder |
| **A destination that exists** (not from the save dialog) | "Replace it?" | Replace, or choose another name |
| **A destination an export in progress will write** | "An export to this file is in progress" | Choose another name, or cancel that export |
| **Running** | The status bar's progress; the quick action's ring | Keep editing, switch photos, cancel |
| **Queued** (a second export) | "Starts after the export in progress" | Cancel it |
| **Finished** | "Exported DSC_0412.jpg" in the status bar | Export again |
| **Failed** (disk full, no permission, the folder gone) | The reason in the status bar, and no file written | Export again elsewhere |
| **Cancelled** | "Export cancelled"; no file written | Export again |

## Data

Nothing is stored on disk but the exported file. The engine holds the running job and the one queued, if any. The dialog's last values live in the frontend for the session.

## Contract (0.6.0)

- `ExportJpeg { photo, path, longEdge: number | null, quality, replace }`: answers `Export { job }`. A refused destination (an original, an existing file without `replace`, a path a job will write), or a photo that does not render, answers the command's error with the reason.
- `CancelExport { job }`: answers `Done`.
- `ExportProgress { job, photo, path, rowsDone, rowsTotal, state }`. `state` is `queued`, `running`, `finished` with notes (the dropped maker notes, unreadable metadata), `failed` with the reason, or `cancelled`.
- `cli export FILE OUT [--long-edge N] [--quality Q]` opens a file, applies an edit given as JSON if any, and exports it; `cli run` takes `ExportJpeg` too.

## Edge cases

- **A switch, a drag or an undo during an export** changes nothing in it (criterion 5).
- **The exported photo's file goes missing during the export:** the job already holds its input, so the export completes.
- **A photo larger than JPEG's 65,535 px on a side** (none in the corpus): refused with "Too large for JPEG; choose a long edge of 65,535 px or less".
- **A very tight crop** (DEV-01's 32 px minimum) exports at that size.
- **The disk fills during the write:** the temporary file is deleted, and the export fails with the system's reason.
- **The destination folder is the opened folder:** allowed, under any name that is not one of the strip's files. The new JPEG is not added to the strip; there is no watching in M1 (LIB-09).
- **Two exports to the same path:** the second is refused while the first is queued or running (see *Encoding and writing*). Once the first has finished, the file exists, and the second asks to replace it as for any file.
- **The app quits during an export:** see *Progress and cancel*.

## Acceptance criteria

1. **Full size equals 1:1:** before encoding, a full-size export's codes equal the 1:1 view's at every pixel, for Bayer, X-Trans, linear DNG and non-RAW sources with an edit that moves every stage (crop and straighten included). Region boundaries leave no trace: 1024 px regions give the same codes as one region (engine test, synthetic sources).
2. **Resized:** a long-edge export matches the CPU reference (full-resolution stages 1–8, box means in linear AP1, stage 9) code for code, at long edges whose output pixels span several regions, down to 32 px. Against the Fit frame of the same size, the median pixel is under ΔE2000 1, and the mean and the 99th percentile are recorded (`export/tests/resized.rs`).
   - **Why not the mean** (changed 2026-11-07, from "mean ΔE2000 under 1", a guess made before measuring): the two differ only on hard edges, by definition. The export averages pixels the tone curve and the local tone mapping have already run on, over each output pixel's exact span; Fit samples a box-filtered pyramid level bilinearly and tones that. On the synthetic scene, whose edges are hard, the median is 0.00 at 500 and 211 px and 0.49 at 40 px; the mean is 0.95 to 2.77, and the 99th percentile 12 to 32, all on edges.
3. **The file:** decodes to the rendered codes within the JPEG's error at quality 100 (PSNR above 45 dB). Its APP2 segment is the sRGB profile, and it is 4:4:4 from quality 90 and 4:2:0 below.
4. **Metadata:** on every corpus file, ExifTool reads the same capture time, make, model, lens, ISO, exposure time, f-number, focal length and GPS position from the export as from the source. Orientation is 1, and the pixel dimensions are the export's. A MakerNote over the limit is dropped and reported. The EXIF reader and writer have a fuzz target (`exif_rewrite`).
5. **A job holds its inputs:** an export started, followed by a switch, an edit and an undo, writes the edit as it was when it started, the same bytes as an export run alone.
6. **Never an original:** a destination equal to any file listed this session, of the current folder or one opened before, through a different spelling of the path or a link, is refused, and nothing is written. An existing destination without `replace`, and one a queued or running export will write, are refused too. A cancelled or failed export leaves no file under the destination name, and the original's bytes and modification time are unchanged (engine test).
7. **Editing stays fast:** with an export of a 48 MP photo running, `bench gpu --export`'s drags stay within the drag budgets (16 ms GPU, 50 ms from slider to pixels, as the command's wall time), and `bench open --export`'s first pixels and neighbor switch within theirs (100 ms, 150 ms), each recorded beside the same run without the export (`docs/m1-results.md`).
   - **Changed** (2026-11-08, from "the RTX 3060 baseline + 10 %", a bound written before measuring): a drag shares the GPU with the region in progress, about 5 ms more at p95, so the baseline's 10 % cannot hold while an export runs; the budgets are what the user feels.
8. **Throughput:** a 24 MP full-size export at quality 85 takes 1 s or less on the reference desktop, alone (`cli bench export`, recorded).
9. **Progress:** `ExportProgress` rows are monotonic and end in exactly one of finished, failed or cancelled. Cancel stops within one region.
10. **Frontend** (Vitest and Playwright against the fake engine): the dialog's defaults and remembered values, the extension rule, the disabled states, progress and cancel in the status bar, compared with W7's sections.
