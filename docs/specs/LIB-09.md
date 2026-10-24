# LIB-09 · Filmstrip (M1 part)

The strip of photos along the bottom of the window. In M1 it shows the folder the user opened, and choosing a photo in it opens that photo in Develop. This spec also covers the engine's "open folder" session behind it: listing the folder, the order, and prefetching the neighbors.

Sources: FEATURES.md (LIB-09), the plan's M1 items (backend item 9, the engine's "open folder" session; frontend item 9; the budgets "Switch to a prefetched neighbor: under 150 ms"), the Decided table ("Edit storage in M1"), the UI handoff (the Filmstrip component: 90 px high, a white ring on the current photo, a blue dot on edited ones), and DEV-01 (*Tiling and the mip pyramid*: prefetched neighbors; *States*). Where this spec and the technical spec disagree, the technical spec wins and this file is corrected.

**Out of scope for M1**, with their milestone:
- a catalog source (folders, collections, filters): M2, when the strip shows "the current source" of the Library;
- stacks, ratings, flags and labels on the cells: M2;
- dragging photos out of the strip, or reordering them: not planned;
- watching the folder for changes on disk: not in M1.

## Behavior

### Opening a folder

- **File › Open folder** (APP-02) picks a folder with the system dialog. The engine lists it (`OpenFolder`) and selects the first photo.
- **That folder only** (owner, 2026-10-24): subfolders are not read. M2's folder tree (CAT-04) covers them.
- **Which files:** every file whose extension is one Genzo reads: the camera RAW formats, DNG, JPEG, HEIC/HEIF, TIFF and PNG. Hidden files, sidecars (XMP) and other extensions are skipped.
  - A file in a format Genzo recognizes but cannot render (Nikon High Efficiency without the `jpeg-xs` feature, CMYK JPEG, a non-optional opcode M1 lacks) is listed. It shows its embedded preview and the reason, and its controls are disabled (DEV-01, *States*).
- **The order:** by capture time (owner, 2026-10-24): EXIF DateTimeOriginal, with its sub-seconds when present. Ties, and files without a capture time, are ordered by file name with numbers compared as numbers (IMG_2 before IMG_10). Files without a capture time come after the dated ones.
- Opening another folder replaces the strip. The edits of the photos in the old folder stay in the engine for the session (decided "Edit storage in M1"), so reopening that folder shows them again.

### The strip

- 90 px high, one cell per photo, in the order above (the handoff's Filmstrip). Each cell shows the photo's embedded preview, scaled to fit, or a neutral cell with the file name when it has none.
- **The current photo** has the white ring. **An edited photo** has the blue dot: its edit differs from its defaults (DEV-01, *Data*: the same comparison as the panels' dots). Undoing back to the defaults clears it (APP-10).
- **Thumbnails load visible cells first**, then the rest. They come from the embedded previews, through the disk tier of the cache (`cache`), never from a render.
- The strip scrolls to keep the current photo in view.

### Choosing a photo

- **A click on a cell**, or **← and →** while Develop has focus and no text field does, selects the previous or next photo (`SelectPhoto`).
- The new photo shows its embedded preview within one frame, then its fit render replaces it (DEV-01, *States*: Opening).
- A gesture in progress ends first: the photo switch waits for its *end*, which records its step.
- The view resets to Fit on every switch. Zoom and pan are not kept per photo in M1.

### Prefetch

- After a switch, the engine decodes the two neighbors on each side, in the background, at low priority (plan item 9). It keeps their pyramids in the CPU cache (DEV-01, *Prefetched neighbors*: an LRU with a 768 MB cap).
- **The order follows the direction of travel** (owner, 2026-10-24). After moving right: next, next + 1, previous, previous − 1. After moving left, the mirror. On opening a folder, as after moving right.
- **A switch cancels prefetch work for photos that are no longer neighbors**, between tiles. A neighbor already cached is not decoded again.
- **Switching to a prefetched neighbor** renders at Fit from its cached pyramid, without decoding. Budget: under 150 ms (plan, M1 budgets; `cli bench open`). Its decoded input, which 1:1 and a settled white balance need, is decoded in the background after the switch (DEV-01).

## Inputs and shortcuts

| Command | Shortcut | Also in |
|---|---|---|
| Open folder | Ctrl O (macOS: Cmd O) | App menu › File (APP-02) |
| Previous photo | ← | |
| Next photo | → | |

All three are registered in the shortcut registry. ← and → do nothing while a text field has focus, or while a slider has keyboard focus (where the arrows move the slider, DEV-02).

## States

| State | What the strip shows | What the user can do |
|---|---|---|
| **No folder opened** | An empty strip with "Open a folder" and its shortcut | Open a folder |
| **Listing** | The previous strip, if any, until the list arrives | Wait. The list replaces it in one step |
| **A folder with no photos** | "No photos in this folder" and the folder's name | Open another folder |
| **Ready** | The cells, the ring on the current photo | Everything above |
| **A thumbnail not loaded yet** | A neutral cell | Select it; the photo opens as usual |
| **A file that went missing** after listing | Its cell, dimmed. Selecting it shows "This file is no longer there" | Select another photo |
| **A folder that cannot be read** (permissions, gone) | The previous strip stays, and the status bar says why | Open another folder |

## Data

Nothing is stored on disk in M1. The engine holds, for the session:

| Field | Type | Meaning |
|---|---|---|
| `photos` | list of `{id, path, file_name, capture_time, kind}` | The opened folder's photos, in the strip's order. `kind` says whether the file renders or is unsupported, and why (DEV-01, *States*) |
| `current` | photo id | The photo in Develop |
| `direction` | next or previous | The last move, for the prefetch order |
| per photo id | the edit and its history | APP-10, DEV-01 *Data*. Kept across folder switches for the session |

`ListFolderPhotos` returns `photos` with each one's edited flag. A photo's id is stable for the session, so the same file opened again keeps its edit.

## Edge cases

- **Thousands of files:** the strip's cells are virtualized, and capture times are read from EXIF only, in parallel, without decoding pixels.
- **Two files with the same capture time and name** (a RAW and a JPEG of one shot, `IMG_1.CR3` and `IMG_1.JPG`): both are listed, ordered by extension. M2's stacking decides whether they pair.
- **A file whose EXIF is damaged:** no capture time, so it sorts after the dated ones by name. It opens as usual, or shows as damaged (DEV-01, *States*).
- **The current photo is the first or last:** ← (or →) does nothing at that end. The strip does not wrap.
- **A neighbor that fails to decode** is not retried by prefetch. Selecting it shows its state (DEV-01, *States*).
- **A file that changes on disk** while the session is open is not detected in M1. The engine reads it again only when its cached pyramid is evicted.

## Acceptance criteria

1. **Listing:** a folder with RAW, DNG, JPEG, HEIC, TIFF and PNG files, XMP sidecars, a hidden file, a subfolder and an unknown extension lists exactly the image files directly inside it (engine test, a temporary folder of corpus copies).
2. **Order:** files with capture times sort by time with sub-seconds, ties and undated files by natural file name, undated after dated (engine test, synthetic EXIF).
3. **Unsupported files** are listed, with their reason, and their controls are disabled (engine test with a Nikon HE file where the corpus has one, else a synthetic one).
4. **Prefetch order** follows the direction of travel, and a switch cancels the work for photos that are no longer neighbors (engine test with a fake decoder that records its calls).
5. **Switching to a prefetched neighbor** renders at Fit without decoding, under 150 ms on the reference desktop (`cli bench open`, recorded in `docs/m1-results.md`).
6. **The edited dot** follows the edit: set by an edit, cleared by undoing back to the defaults, kept across a folder switch and back (engine test).
7. **Frontend** (Vitest and Playwright against the fake engine): the ring, the dot, ← and →, the empty and no-photos states, and visible-first thumbnails, compared with the handoff's Filmstrip.
