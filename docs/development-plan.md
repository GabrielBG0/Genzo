# Genzo: Development Plan

Built from these sources:

- **Spec:** *Genzo: Technical Spec (draft 2)*, revision 57. The artifact is still listed as "Photo Editor: Technical Spec (draft 1)".
- **UI handoff:** [`docs/UI/fixer-ui-handoff.html`](UI/fixer-ui-handoff.html). It holds the approved Carbon Glass direction, every mockup screen, the design system and the component list. "Fixer" was the working name before Genzo.
- **Feature list:** [`docs/UI/FEATURES.md`](UI/FEATURES.md). It gives every UI feature an ID, a priority (P0, P1, P2) and its screens.

The phases are the spec's milestones, M0 to M5, and each one ends on the exit gate the spec defines. Inside a phase, steps are split into four tracks:

- **Backend**: Rust crates under `backend/`.
- **Contract**: the Rust types, the generated TypeScript in `contract/`, and version bumps.
- **Frontend**: the Svelte app under `frontend/`, built against the fake engine first and the real engine second.
- **Quality**: tests, benchmarks and CI.

The appendix maps every FEATURES.md ID to a milestone.

## Naming used in this plan

- Screen codes follow the handoff:
  - **D1–D12**: Develop screens.
  - **L1–L9**: Library screens.
  - **W1–W8**: Workflow and output screens.
  - **Menu-1 to Menu-5**: the Menus screens. FEATURES.md calls these M1 to M5, which clashes with the milestone names, so they are renamed here.
- **M0–M5** always means a milestone.

---

## Resolution rule

Two rules decide how the sources combine:

1. **When the spec and the handoff/FEATURES.md conflict, the spec wins.**
2. **When only one source has something, it is added to the plan.**

Every item below has been resolved with these two rules. No open conflicts remain. The spec's feature table now lists the additions (spec revision 57), so it stays the single scope gate. The handoff and FEATURES.md were corrected where the spec won.

### How the sources fit together

1. **Milestones set the build order. P-levels do not.** The handoff says to build "P0, then P1, then P2", but this conflicts with the spec's milestones, which are ordered by risk, so the spec wins. In this plan:
   - **P0 and P1** mean the feature is in v1 (by M5).
   - **P2** means v1, unless the spec places it in Later.

   The milestone column in the appendix is the schedule.
2. **Architecture.** The handoff says the tokens should be ported "into the chosen Rust UI toolkit" and that "the toolkit is not decided yet". The spec decided on Tauri 2 with Svelte 5 and TypeScript, so the spec wins. The handoff's CSS ports almost directly into CSS variables. The handoff now says so.
3. **Look and interaction come from the handoff**, wherever the spec is silent: tokens, sizes, surfaces, the menus model, quick actions and shortcuts.
4. **Frontend work during M0.** The spec says "no feature work starts until the M0 gate passes". Tokens, shared components and the fake engine count as foundation work, so they run in parallel with the M0 spikes. Screens wired to real engine features wait for the gate.
5. **Specs per feature.** FEATURES.md has a spec template for each feature (Behavior, Inputs and shortcuts, States, Data, Edge cases, Acceptance criteria). Every entry still reads "to be written". **A feature's spec is written before its milestone starts.** The first task of every milestone is to write the specs for the features scheduled in it.

### Conflicts, resolved in favor of the spec

| Item | Handoff / FEATURES.md | Spec (adopted) |
|---|---|---|
| Build order | P0, then P1, then P2 | Milestones M0 to M5, ordered by risk |
| UI toolkit | Rust UI toolkit, not decided | Tauri 2, Svelte 5, TypeScript |
| Pipeline order (DEV-01) | Its own stage list | The spec's fixed stage order |
| AI denoise output (DEV-08) | Writes a new linear DNG, stacked with the original | A cached linear raster that the edit references. The before/after dialog, amount and time estimate from the handoff are kept. |
| Content-aware remove (D12, RMV-01) | P1, the main Remove mode | Later. Heal and clone ship in M3. |
| Map module and tab (L9, MAP-01 to MAP-03) | P2, a module tab | Later. The tab appears when the module ships. |
| Print module and tab (W8, OUT-02) | P2, a module tab | Later. The tab appears when the module ships. |
| People view (L5, LIB-11) | P2 | Later |
| Depth range mask (MSK-05) | Listed | Later, because the spec ignores Apple depth data in v1 |
| Red eye (RMV-02), reference view (WF-07) | P2 | Later |
| Backdrop blur on overlays over the photo | Real blur on the photo toolbar, badges and photo-anchored dialogs | Glass only on slider handles and selection indicators, flat elsewhere. Photo overlays keep the handoff's border and shadow, but use a flat, near-opaque fill with no `backdrop-filter`. |
| Split features (D1, D5, D7, D10) | One P-level for the whole screen | Split by the spec's milestones: texture, clarity and dehaze in M3, auto settings in M4, AI denoise in M4, Upright in M4, AI masks in M4 |
| Sidecar default (MET-06) | "Auto-write XMP option?" | Written automatically. Ctrl S forces a write. |
| Catalog engine (CAT-01) | "SQLite?" | SQLite in WAL mode, with schema versioning and migrations |

### Additions: in one source only, so added

**From the handoff and FEATURES.md, not in the spec:**

| Item | Milestone |
|---|---|
| Compact app menu behind the logo, also on Alt (APP-02). No classic menu bar. | M1, grows each milestone |
| Quick actions (APP-03): fixed defaults first, then customization, pinning and local usage suggestions | M1 and M2 defaults, M5 customization |
| Export as a module tab (APP-01) that opens the W7 export view | M1 (basic), M4 (full) |
| Command search, Ctrl K (APP-04) | M2, presets added in M3 |
| Shortcut sheet (APP-05), then custom shortcuts | M2 sheet, M5 editor |
| Right-click menus (APP-06) | M1 slider, M2 grid, M3 Develop photo |
| Status bar (APP-07), panel visibility (APP-08) | M1 |
| Preferences and catalog settings (APP-11) | M2 |
| Screen modes and Lights Out (APP-09) | M5 |
| Import extras (CAT-02): eject after import, second backup copy, rename templates, develop and metadata presets on import, import presets | M2 core extras, M3 presets on import |
| Copy as DNG on import (CAT-02), DNG export (OUT-01). Both need a DNG writer in `imgio`. | M4 |
| Smart previews for offline editing (CAT-03) | M4, built on the DNG writer |
| Quick Collection (CAT-05), keyword sets (MET-04) | M2 |
| Compare Differences panel, including a computed focus (sharp or soft) metric (LIB-03) | M2 |
| Keyword painter (MET-05) | M3 |
| Quick Develop (LIB-12) | M3, with Auto tone in M4 |
| Calibration (DEV-14) | M3 |
| Defringe (DEV-09), B&W treatment (DEV-02) | M3, M1 |
| Panel states (DEV-13) | M1 edited dots, M3 the rest |
| People AI mask (MSK-02) | M4 |
| Detect-objects assist for Remove (RMV-01) | M4 |
| Export extras (OUT-01): AVIF, email destination, watermark, stripping location and people names | M4 |
| Fonts: Public Sans and JetBrains Mono, bundled | M0 |

**From the spec, not in the handoff (already in the plan):**
- The software GPU fallback and its status bar notice.
- The `cli` crate and the performance budgets in CI.
- Catalog backup and the integrity check.
- The fingerprint relink for moved files.
- Edit in an external editor (TIFF round trip).
- Spec items in Later with no mockup: HDR and panorama merge, soft proofing, lens blur, point color, adaptive presets, raw details, super resolution.

## Parked decisions, with a deadline

These are open questions, not conflicts. Neither source answers them.

| Decision | Source | Decide by | Why then |
|---|---|---|---|
| Minimum OS versions | Spec | Start of M0 | The CI matrix and the WebView2/WebKitGTK baseline depend on it |
| Camera priority order | Spec | During M0 (the rawler gap check) | It sets which LibRaw fallbacks are wired in M1 |
| Undo scope: global across library and develop actions, or per photo | APP-10 | Before M1 | The shape of the `Undo` command and the history model are fixed in the M1 contract |
| Remember module state per photo or per module | APP-01 | Before M1 | View state store design |
| Offline only (no accounts, no cloud) | Spec | Before M2 | Affects the catalog and the AI model download policy |
| Multiple catalogs | Spec | Before M2 schema work | Catalog path handling in engine |
| Ctrl K: command search or Add keywords | APP-04, MET-02 | Before M2 | Shortcut registry defaults |
| Undo depth; preview cache size and location | APP-10, CAT-03 | Before M2 | Catalog settings |
| Color label sets; editable IPTC fields in v1 | LIB-07, MET-01 | Before M2 | Catalog schema |
| macOS: mirror the app menu in the system menu bar | APP-02 | Before M2 | macOS users expect a menu bar |
| Focus metric method for Compare | LIB-03 | Before M2 | It is computed during preview generation |
| Preset format; import of Lightroom .xmp presets | WF-01 | Before M3 | The preset store ships in M3 |
| Pen pressure for the brush | MSK-03 | Before M3 | Input handling in the viewport |
| Segmentation, people and denoise models, and their licenses | MSK-02, DEV-08 | Before M4 | The `ai` crate's model registry |
| Lightroom import (.lrcat, `crs:`, shortcut presets) | Spec, APP-05 | Before M5 | Best effort; could move to Later |
| Signing budget | Spec | Before M5 | Needed for the first public release |

---

## M0: Feasibility spikes

**Goal:** prove the two risks most likely to sink the project before any feature work. The first is pushing pixels through a webview fast enough. The second is decoding ProRAW.

**Exit gate:** slider input to pixels under 50 ms p95 at 2560 px on Windows, Linux and macOS, **and** ProRAW files from several iPhone generations decode. If the gate fails, apply the viewport fallback ladder on the failing OS only. If that does not work either, change the plan.

### 0.A Repository and CI scaffolding

1. Create the repo layout: `backend/`, `contract/`, `shell/`, `frontend/`, `docs/`.
2. Create the Cargo workspace with stub crates: `model`, `imgio`, `color`, `pipeline`, `catalog`, `cache`, `jobs`, `ai`, `export`, `engine`, `cli`. Dependency edges point downward only.
3. Make `engine` the only public crate. Everything below it stays workspace-private.
4. Set up the Tauri 2 `shell/` crate, depending on `engine` only.
5. Set up `frontend/`: Vite, Svelte 5, strict TypeScript, Vitest and Playwright.
6. Set up the contract generator (specta or ts-rs), writing into `contract/`, with the output checked in.
   - Add a `contract_version` constant.
   - Add a schema snapshot test: CI fails on a breaking change unless the version is bumped.
   - Make the shell refuse to start when the app and engine contract versions are incompatible.
7. Enforce the dependency rules in CI:
   - Nothing in `backend/` depends on `shell/`, `frontend/`, Tauri or any UI toolkit. Check this with `cargo metadata`.
   - `frontend/` touches the Tauri API in exactly one transport module. Use a lint rule.
   - `shell/` has no business logic. Use a review checklist and a size budget on the crate.
8. Set up the CI matrix: Windows, Linux, macOS on Apple silicon, macOS on Intel. GPU tests run on lavapipe on every push.
9. Set up the benchmark harness: criterion for stages, and `cli bench` for end-to-end latency. Store the baselines, and fail CI on a p95 regression above 10 percent.
10. Add `cargo-fuzz` targets for `imgio` and `cargo-deny` for licenses and advisories.

### 0.B License review (blocks adopting any dependency)

1. Review the licenses of rawler, LibRaw (LGPL 2.1), libheif and libde265 (LGPL), lcms2 or moxcms, wgpu, ort and ONNX Runtime, and lensfun data (CC BY-SA).
2. Review the fonts from the handoff, **Public Sans** and **JetBrains Mono** (both OFL), for bundling.
3. Write down the HEVC patent position.
4. Record the results in `docs/licenses.md` and encode them in the `cargo-deny` config.

### 0.C ProRAW and RAW decode spike

1. Build the test corpus:
   - ProRAW at 12 MP and 48 MP, and a Bayer DNG, for each available iPhone generation.
   - One file per camera maker from raw.pixls.us.
2. Decode each file with rawler. Check the levels, the matrices, the opcode lists and the gain table map, and list any missing tags.
3. Check Nikon High Efficiency RAW support in rawler and LibRaw.
4. Prototype the LibRaw FFI fallback on one file rawler cannot read.
5. Record reference renders from Apple Photos and Adobe DNG Converter.
6. Measure memory while streaming a 48 MP linear DNG into the mip pyramid.
7. Decide the camera priority order.

### 0.D GPU pipeline spike

1. Set up a wgpu device on a dedicated render thread. Other threads send it work over channels.
2. Implement a minimal path with WGSL shaders on RGBA16F textures: decode, white balance, demosaic (RCD), camera matrix, exposure, output transform.
3. Build the mip pyramid, with per-stage caching at viewport resolution.
4. Measure GPU time per slider change at 2560 px. Target: under 16 ms.
5. Check the software adapter: WARP on Windows, lavapipe or llvmpipe on Linux. Add the startup self-test and the driver blocklist.

### 0.E Pixels through the webview spike

1. Serve binary tiles over a custom URI scheme or a Tauri binary channel. Each tile carries a header with the generation number, rectangle, mip level and format.
2. Draw the tiles on a WebGL2 canvas behind the `Viewport` interface.
3. Measure end to end on each OS at 2560×1440 with RGBA8 frames: input, render, readback, IPC, upload, screen. Test WebKitGTK 4.1 on two distros.
4. If the gate fails on an OS, try ladder step 1 (a proxy while dragging), then step 2 (QOI or JPEG). Only after that, prototype step 3 (a native wgpu surface).
5. Verify color handling on each OS: sRGB and Display P3 tagged frames.

### 0.F Frontend foundation (parallel, against the fake engine)

1. **Port the handoff's design system** into CSS variables:
   - **Color tokens**, using the handoff's names: `bg`, `dialog`, `text.strong`, `text.body`, `text.secondary`, `text.muted`, `text.dim`, `line`, `accent`, `rating`, `label.red`/`label.yellow`/`label.green`/`label.blue`/`label.purple`, `mask.overlay` and `danger`.
   - **Type:** Public Sans 400/500/600 at a 12.5 px base. JetBrains Mono for numbers, shortcuts and file data. Panel headers in 11 px uppercase.
   - **Sizes:**
     - Top bar 44 px.
     - Left and right panels 252 and 304 px.
     - Filmstrip 90 px, status bar 22 px.
     - Padding and gaps 6 px.
     - Rows: slider 22, header 28, menu item 28, list row 24 px.
   - **Radii:** panes 12, menus 12, dialogs 18 to 20, photo 10, thumbnails 7, pills 99.
   - **Surfaces:** **Pane** (flat), **Lens** (selection: a gradient, no blur), **Overlay** (photo toolbar, badges and photo-anchored dialogs: the handoff's Glass border and shadow on a flat, near-opaque fill, **no backdrop blur**, per the spec) and **Slider** (3 px track, accent fill from the center or the left, glossy 18 × 13 handle).
   - A lint rule that forbids hard-coded colors, and room for a light token set.
2. **Bundle the fonts locally** instead of loading them from Google Fonts.
3. **Encode the handoff's rules in the components:**
   - Glass means something can be grabbed or selected. Flat means information. Following the spec, real glass (backdrop blur) is allowed only on slider handles and selection indicators. Everything else is flat.
   - The edited state is always visible: a blue dot on edited panels, a bright label and value on changed sliders, a dot on edited thumbnails.
   - Every command shows its shortcut wherever it appears.
   - Right-click menus act on the whole selection and show mixed states as "–".
   - Dialogs size to `min(target, 100% - 48px)` and their content wraps or scrolls.
4. **Build the shared components** from the handoff's Components table, with the props it lists:
   - Top bar (`module`)
   - Left panel · Develop (`latest`, `latestValue`)
   - Left panel · Library (`active`)
   - Histogram + tool strip (`tool`, `clip`, `notools`)
   - Panel header (`heading`, `meta`, `edited`, `open`)
   - Slider row (`label`, `value`, `pos`, `from`, `track`, `dim`, `hot`)
   - Grid cell (`idx`, `stars`, `flag`, `label`, `selected`, `badge`)
   - Library toolbar (`view`, `extra`)
   - Filmstrip (`selected`)
   - Status bar (`mode`, `info`)
   - Menu (items with label, shortcut, submenu, check, hot, danger, disabled, separators)
   - Photo viewport placeholder

   Put each in a Storybook-style gallery and compare it visually with "Open full size" in the handoff.
5. **App shell** with the handoff's 1440 × 900 layout: top bar, main row, filmstrip and status bar.
6. **Shortcut registry.** It is the single source for keyboard handling, menu labels, tooltips and, later, command search and the shortcut sheet.
7. **Fake engine** in TypeScript that implements the generated contract, with a static tile source. Add the **transport module**, the only file that touches Tauri.

**Exit checklist**
- [ ] Latency gate passed, or a ladder step chosen, on all three OSes.
- [ ] ProRAW decodes on every device in the corpus
- [ ] Software adapter runs the spike pipeline
- [ ] License review written, fonts included
- [ ] Performance targets confirmed or revised in the spec
- [ ] Tokens, fonts, shared components and app shell running against the fake engine
- [ ] The M1 parked decisions are settled (undo scope, module state)

---

## M1: Develop core

**Goal:** open a folder of files directly, without a catalog, and edit with the core Basic panel, crop, histogram and JPEG export, all rendering fast.

**First task:** write the specs, using the FEATURES template, for APP-01, APP-02, APP-03 (fixed defaults), APP-06 (slider part), APP-07, APP-08, APP-10, DEV-02, DEV-03, DEV-12, LIB-09, WF-02 (simple toggle) and OUT-01 (basic).

**Exit gate:** meets the slider and photo-open budgets on the reference GPU (RTX 3060 class).

| Target | Value |
|---|---|
| Photo open, first pixels (embedded preview) | under 100 ms |
| Full fit-view render | under 1 s for 12 MP, under 2 s for 48 MP |
| Slider GPU time at 2560 px | under 16 ms |
| Slider input to pixels | under 50 ms p95 |
| Switch to a prefetched neighbor | under 150 ms |
| 1:1 pan, newly exposed area sharp | under 100 ms |
| GPU memory, one 48 MP edit | under 1 GB (full-resolution intermediates only per tile) |
| Cold start to usable window | under 1.5 s |

### Backend
1. `model`: the versioned edit struct, with serde, a schema version and unknown fields preserved. It covers:
   - Profile, Color or B&W treatment, and white balance (temperature, tint).
   - Exposure, contrast, highlights, shadows, whites, blacks.
   - Vibrance, saturation.
   - Crop, angle, 90° rotation and flip.
2. `imgio`: the normalized decode output. Covers ProRAW, Bayer DNG, the LibRaw fallback for priority cameras, HEIC (libheif), JPEG, TIFF and PNG. Reads EXIF.
3. `color`: camera-to-working-space matrices (the working space fully contains Display P3), the display transform and ICC. Includes CPU reference math.
4. `pipeline`:
   - Stages 1–4, the core of stages 6 and 7, crop and rotate, and the output transform.
   - Re-runs start at the changed stage.
   - Interactive and settled quality levels.
   - White balance applied after the cached demosaic while dragging.
   - Generation numbers, latest-wins coalescing and cancellation.
   - 256 px tiles at 1:1, visible tiles first, then a ring around them.
5. `pipeline`: the histogram, and clipping masks for J.
6. `cache`: an in-memory LRU and a disk tier. `jobs`: a priority scheduler, rayon for CPU work and tokio for IO.
7. `engine`: an "open folder" session.
   - Show the embedded preview within one frame.
   - Prefetch two neighbors on each side.
   - Own the edit document and its undo and redo, in the scope decided before M1.
8. `export`: basic JPEG with size and quality, through the same GPU pipeline, tiled.
9. `cli`: `open`, `render`, `export`, and golden-image tests.

### Contract
- Commands: `OpenFolder`, `SelectPhoto`, `ApplyEditDelta`, `ResetParam`, `Undo`, `Redo`, `ExportJpeg`.
- Queries: `ListFolderPhotos`, `GetEdit`, `GetHistogram`.
- Events: `RenderFinished`, `ExportProgress`, `AdapterInfo` (for the software-fallback notice).
- Frames: the tile protocol from M0. Bump the version.

### Frontend (screens D1, D9; components Top bar, Left panel · Develop, Histogram + tool strip, Slider row, Panel header, Filmstrip, Status bar, Menu)
1. **APP-01 Module switcher**: pill tabs. Develop is active (D). Library is disabled until M2. The Export tab opens the basic export view. Map and Print tabs appear only when those modules ship (Later).
2. **APP-02 App menu** behind the logo, also opened with Alt. Start with the File, Edit, Photo, Develop, View and Help items that exist in M1. It grows every milestone.
3. **APP-03 Quick actions** with fixed Develop defaults: Undo, Redo, Before/After, Reset all and Export. Customization comes later.
4. **D1 Basic panel (DEV-02, core):**
   - Profile, the Color/B&W toggle (V), white balance presets, and the picker (W).
   - Temp and Tint on color-gradient tracks.
   - The tone and presence sliders.
   - Changed sliders shown bright, unchanged ones dim.
   - Double-click to reset.
   - Optimistic values while dragging.
   - *Texture, clarity and dehaze come in M3. Auto tone (Ctrl U) and auto white balance (Ctrl Shift U) come in M4.*
5. **DEV-03 Histogram**: the RGB histogram with an EXIF row, clipping corner markers and J overlays. Alt-drag on a slider previews clipping.
6. **D9 Crop and rotate (DEV-12)**:
   - Handles drawn as an overlay, aspect presets and lock, angle slider and straighten tool.
   - O cycles the overlays, X swaps orientation, Ctrl [ ] rotates by 90°, plus flip and a pixel size readout.
7. A simple before and after toggle (\\). The full WF-02 comes in M3.
8. **Viewport:**
   - Fit and 1:1 zoom, with Space for a temporary zoom.
   - Zoom and pan as a GPU transform over the tiles already held.
   - Photo toolbar and badges on the flat Overlay surface.
9. **LIB-09 Filmstrip** of the opened folder: a white ring on the current photo and a blue dot on edited ones.
10. **APP-06 Slider context menu** (part of Menu-4): reset, copy value, paste value.
11. **APP-07 Status bar**: mode pill, color space, bit depth, preview status and the software-adapter notice.
12. **APP-08 Panel visibility**: Tab, Shift Tab, F6, F7, F8, Alt-click for solo mode.
13. **APP-10 Undo and redo** (Ctrl Z, Ctrl Shift Z) for edits.
14. **OUT-01 (basic)**: an export dialog with JPEG, size and quality. It is a subset of W7.

### Quality
- Golden images per stage, on lavapipe and on real GPUs nightly. Fuzz targets for every decoder.
- The M1 budget table enforced in CI.

---

## M2: Library

**Goal:** a real catalog, with import, grid, loupe, compare, survey, culling, filters, collections, keywords and metadata.

**First task:** write the specs for the CAT, LIB, MET and APP features scheduled in M2 (see the appendix). Settle the M2 parked decisions.

**Exit gate:**
- A 100k-photo catalog scrolls at 60 fps, with no empty cell visible longer than 100 ms.
- Any filter combination returns in under 50 ms.
- Import in place runs at 20 files per second or better.
- Idle memory stays under 1.5 GB.

### Backend
1. `catalog` (CAT-01):
   - SQLite in WAL mode, with migrations.
   - Tables for Photo, Folder, Keyword (hierarchical), Collection and collection set, Quick Collection, Edit and History step.
   - Preview records keyed by edit version.
   - One writer thread and a pool of read connections.
2. Persistence: the edit is stored as versioned JSON. History steps are recorded per finished gesture. Library actions are covered by undo, in the scope decided before M1.
3. Indexed filter columns: capture time, rating, flag, label, folder, camera and lens. FTS5 for keywords, filenames and captions. Saved filters.
4. Fingerprints, so moved and renamed files relink. Missing and moved folder state (CAT-04).
5. **MET-06 XMP sidecars**: standard XMP for ratings, keywords and labels, and the app's own namespace for develop settings. They are written automatically or with Ctrl S, as decided, and the writes are debounced. Original files are never written to.
6. Catalog backup, and an integrity check on open.
7. **CAT-02 Import job:**
   - Copy, move or add in place, from devices or folders.
   - Skip duplicates. Eject after import. Second backup copy to another location.
   - Choose the preview size. Destination with subfolders and date organization.
   - Rename template with a sample, a metadata preset and keywords applied on import, and saved import presets.
   - Make thumbnails from embedded previews first, then two pre-generated thumbnail sizes.
   - Progress events.
8. **CAT-03 Previews**: standard and 1:1 previews built in the background, with progress.
9. **LIB-03 Focus metric**: a sharp/soft score computed while previews are built (method decided before M2) and stored on the Photo, for the Compare Differences panel.
10. A `cli` seed tool for a synthetic 100k catalog.

### Contract
- Commands:
  - Import.
  - SetRating, SetFlag and SetLabel. They act on a selection, and "and advance" variants exist.
  - Keyword add and remove.
  - Collection CRUD, and QuickCollection toggle.
  - Folder create, rename and remove.
  - SetMetadata.
- Queries: `QueryPhotos` (filter, sort and paging, returning ids plus thumbnail refs), `GetMetadata` (with mixed values for multi-select), the folder, keyword and collection trees with counts, and `CompareDiff`.
- Events: `ImportProgress`, `PreviewProgress`, `CatalogChanged`. Thumbnails travel over the binary protocol.

### Frontend (screens L1, L2, L3, L4, L6, Menu-2, Menu-3; components Left panel · Library, Grid cell, Library toolbar)
1. **L1 Grid (LIB-01)**: a virtualized grid that reads thumbnails only. Shows the index, flag, stars, label and badges (a reject dims the thumbnail), with thumbnail size and sort.
2. **LIB-08 Filter bar**: Text, Attribute and Metadata columns, saved filters, and a lock.
3. **L2 Loupe (LIB-02)**: the info overlay (I, Ctrl I), Fit and 1:1 zoom, arrow keys, and a floating rating toolbar.
4. **L3 Compare (LIB-03)**: linked zoom, swap, promote, and the Differences panel, which highlights differing EXIF, focus, rating and label.
5. **L4 Survey (LIB-04)**.
6. **Culling:**
   - LIB-05 flags: P, X, U, plus Shift variants that move to the next photo.
   - LIB-06 stars: 0–5, [ and ].
   - LIB-07 labels: 6–9, plus purple.
7. **CAT-04 Folders**, **CAT-05 Collections and Quick Collection** (B, Shift B, Ctrl B, Ctrl N).
8. **MET-01 Metadata panel**, **MET-02 Keywording panel** with partial markers, **MET-03 Keyword list** (the L8 list part), and **MET-04 Keyword sets** (Alt 1–9).
9. **L6 Import (CAT-02)**: the full-window import screen.
10. **APP-06 Grid context menu (Menu-3)**: acts on the selection and shows mixed states as "–".
11. **APP-02**: the compact app menu (Menu-2) gains the Library items.
12. **APP-04 Command search (Ctrl K, W5)**: commands, collections and photos, with filter chips and a ">" prefix for commands. Presets are added in M3.
13. **APP-05 Shortcut sheet (Ctrl /, W6)**: read-only for now, with Windows and macOS layouts.
14. **APP-11 Preferences and catalog settings**: import defaults, previews, GPU/performance, backups.
15. **APP-03**: Library quick action defaults (Import, Export, Quick Collection, Undo, Redo).
16. Develop now opens photos from the catalog.

### Quality
- A 60 fps scroll test with Playwright tracing on the 100k seed catalog.
- A query latency benchmark across filter combinations.
- Migration and unknown-field round-trip tests. XMP interoperability checks against other tools.

---

## M3: Develop complete

**Goal:** the remaining M3 Develop tools, masks without AI, heal and clone, the workflow tools, and the M3 Library extras.

**First task:** write the specs for the DEV, MSK, RMV (heal and clone part), WF and M3 Library features.

**Exit gate:** every stage matches its golden images on lavapipe and on every real GPU tested.

### Backend
1. Stage 5 (DEV-09, DEV-10 manual):
   - lensfun profiles picked from EXIF.
   - Chromatic aberration and defringe.
   - Manual distortion and vignetting.
   - Manual transform: vertical, horizontal, rotate, aspect, scale, offsets, constrain crop.
2. Stage 6 (DEV-04, DEV-02 rest): point and parametric curves per channel, texture, clarity, dehaze. The radii scale with the mip level.
3. Stage 7 (DEV-05, DEV-06, DEV-14): the HSL and color mixer, three-way color grading with blending and balance, and calibration (DEV-14: shadows tint, RGB primaries).
4. Stage 8 (DEV-07): sharpening with a masking preview, and noise reduction for luminance and color.
5. Stage 9 (DEV-11): post-crop vignette (all six parameters), and grain with presets.
6. Masks (MSK-01, MSK-03, MSK-04, MSK-05 color and luminance, MSK-06):
   - Components combined with add, subtract and intersect, plus invert.
   - Brush A, Brush B and Erase, with auto mask.
   - Linear and radial gradients.
   - Color and luminance range masks, as standalone masks or as refinements.
   - Per-mask adjustments, and mask presets.
7. **RMV-01, heal and clone part**: movable sources, size, feather, opacity, and a list of spots.
8. **WF-01 Presets** (with an amount from 0 to 200), **WF-03 History**, **WF-04 Snapshots**, **WF-05 Virtual copies** (each with its own history), **WF-06 Copy, paste and sync settings**.
9. **CAT-06 Smart collections**: nested rules with match all or any. **LIB-10 Stacks**.

### Contract
- Edit struct v2 with a migration function. Mask entities, and coalesced brush stroke deltas.
- Commands for presets, snapshots, virtual copies, stacks, sync and paste-previous. A live match count for smart collections.

### Frontend (screens D2–D8, D10, D11, D12 partly, W1–W4, L7, L8, Menu-4)
1. **D2 Tone curve** with targeted adjustment, **D3 Color mixer** with colored tracks and targeted adjustment, **D4 Color grading** wheels.
2. **D5 Detail** with the 1:1 inset. *The AI denoise section is hidden until M4.*
3. **D6 Optics**. **D7 Geometry**, manual controls. *Upright and guided mode are hidden until M4.*
4. **D8 Effects**. D1 gains texture, clarity and dehaze.
5. **D10 and D11 Masking**:
   - A floating mask list.
   - Overlay color, Alt O to cycle the overlay, H to hide pins.
   - Brush keys: K, [ ] for size, Shift [ ] for feather.
   - Gradients: M and Shift M.
   - Range samplers.
   - *The AI sources are hidden until M4.*
6. **D12 Remove, heal and clone modes only (Q)**. *The content-aware mode stays out (see the conflicts table).*
7. **DEV-13 Panel states**: edited dots, per-panel reset, a "from preset" marker, and an on/off toggle per panel.
8. **W1 Presets**: hover preview on the photo and in the Navigator, plus the amount slider. **W2 Before/After** in all modes (Y, Alt Y, Shift Y, \\), with copy and swap. **W3 Virtual copies and snapshots** (Ctrl ', Ctrl N). **W4 Copy and paste settings** (Ctrl Shift C, Ctrl Alt V).
9. The history timeline in the Develop left panel, and sync across the selection.
10. **L7 Smart collection rules**. **L8 Keyword painter** (MET-05, Ctrl Alt K, Alt to erase). Stack badges with expand and collapse (Ctrl G).
11. **APP-06 Develop photo context menu (Menu-4)**: the remaining parts.
12. **LIB-12 Quick Develop**: preset, white balance, exposure and contrast step buttons, and Reset. Its Auto tone button arrives in M4.
13. **APP-04**: command search gains presets.
14. **CAT-02**: a develop preset applied on import.

### Quality
- Golden images per stage and for full edits (ΔE2000 and PSNR). Side-by-side quality reviews against the corpus.

---

## M4: AI and output

**Goal:** AI masks, AI denoise, automatic perspective and auto settings, and full export.

**First task:** pick the models and check their licenses. Write the specs for DEV-08, MSK-02, RMV-01 (object detection), DEV-10 (Upright), CAT-02 (Copy as DNG), CAT-03 (smart previews) and OUT-01 (full).

**Exit gate:** editing stays within the M1 budgets while AI inference and batch export run in the background. Export of a 24 MP JPEG runs at 1 per second or better, with the batch running in parallel.

### Backend
1. `ai`:
   - Pick permissively licensed segmentation and denoise models, and verify each license.
   - Download them on first use, with checksums. They are not bundled.
   - Run them through `ort`, with a CPU fallback.
2. **MSK-02 AI selections** for subject, sky, background, objects and people, as background jobs cached as textures. Re-detect them when masks are pasted onto other photos.
3. **DEV-08 AI denoise** as a background job, with a time estimate and an amount. Per the spec, its output is a cached linear raster that the edit references, not a new DNG.
4. **RMV-01 Detect objects**: AI assist that proposes a heal or clone selection. It never generates pixels.
5. **DEV-10 Upright** (Auto, Guided, Level, Vertical, Full). **DEV-02** auto tone and auto white balance.
6. **DNG writer in `imgio`**: used by Copy as DNG on import (CAT-02), DNG export, and smart previews (CAT-03: lossy DNG proxies for offline editing, tracked in the catalog).
7. **OUT-01 export:**
   - JPEG, AVIF, TIFF, PNG and DNG.
   - Destinations: a folder, email (hand-off to the system mail client) or another app.
   - Watermark.
   - Quality, color space (sRGB, Display P3, Adobe RGB), size limit and resize, resolution.
   - Output sharpening.
   - Metadata options, including stripping location and people names.
   - Filename templates and presets.
   - A background batch queue with a size estimate.
   - External editor round trip as TIFF.

### Contract
- AI job commands and events: model download, progress, cancellation.
- Export preset entities, the queue, and size estimates.

### Frontend (screens D5 AI section, D7 Upright, D10 AI sources, W7)
1. Turn on the denoise section in D5 (the Enhance dialog with before and after), Upright and guided mode in D7 (2 to 4 guides with a magnifier), and the AI sources in D10.
2. UI for model download consent and progress.
3. Heal and clone in D12 gain the detect-objects assist. Import (L6) gains Copy as DNG and smart previews. The status bar shows smart preview state.
4. **W7 Export**, in full.
5. Background job indicators in the status bar.
6. Auto tone and auto white balance (Ctrl U, Ctrl Shift U) in D1 and in Quick Develop.

### Quality
- A latency benchmark that drags sliders while AI jobs and export run.
- Golden images showing that export matches the preview.

---

## M5: Hardening and release

**Goal:** a performance pass, packaging, signing, the light theme, customization and docs.

**Exit gate:** a clean install and a full workflow pass on Windows, Linux and macOS.

1. **Performance pass** across the whole budget table, including the integrated-GPU tier (Apple M1, Intel Iris Xe) and the software-adapter tier.
2. **APP-12 Light theme**: the second token set, plus an audit for hard-coded colors.
3. **APP-03 Quick actions customization (Menu-5)**: reorder, pin from any right-click menu, and suggestions from local usage data that the user can turn off.
4. **APP-05 Custom shortcuts**: the Customize editor in the shortcut sheet.
5. **APP-09 Screen modes and Lights Out**: F, L, Shift L, Ctrl Shift F.
6. Packaging: MSI or NSIS on Windows, deb, rpm and AppImage on Linux, dmg on macOS.
7. Signing, according to the signing-budget decision.
8. Lightroom import (`crs:`, presets, shortcuts), if it is in scope.
9. Docs: a user guide, a keyboard reference, and a contributor guide with the architecture rules.
10. A manual QA script for a full workflow: import, cull, develop, mask, export.

---

## Later (after v1, not scheduled)

- People view (L5, LIB-11)
- Map module and geotagging (L9, MAP-01 to MAP-03)
- Print (W8, OUT-02)
- Content-aware remove (D12 content-aware mode, RMV-01 part)
- Red eye (RMV-02)
- Reference view (WF-07)
- Depth range mask (MSK-05 part)
- Publish services, tethered capture and watch folders
- HDR and panorama merge
- Soft proofing
- Lens blur, point color and adaptive presets
- Super resolution and raw details enhancement
- Book, Slideshow and Web (OUT-03)
- Flatpak packaging

Generative remove, expand and fill stay **out**, by decision.

---

## Cross-cutting rules (every phase)

- Each feature's spec is written from the FEATURES.md template before its milestone starts. Its acceptance criteria become the feature's tests.
- Every contract change is reviewed through the generated TypeScript diff and comes with a version bump.
- Every new decoder gets a fuzz target. Every new stage gets golden images and a criterion benchmark.
- Every new screen works against the fake engine first, with Vitest and Playwright tests, and is compared with the handoff's full-size screen.
- No component hard-codes a color. Backdrop blur appears only on slider handles and selection indicators (spec). Everything else, photo overlays included, is flat.
- Every command is registered in the shortcut registry, so menus, tooltips, quick actions and search show the same shortcut.
- The cli can do everything the UI does except draw.

---

## Appendix: FEATURES.md → milestone

The **Source** column says where a row's milestone comes from:
- **spec**: the spec's feature table.
- **added**: only in the handoff or FEATURES.md, so it was added.
- **spec wins**: the sources conflicted and the spec was adopted (see the conflicts table).

| ID | Feature | P | Milestone | Source |
|---|---|---|---|---|
| APP-01 | Module switcher | P0 | M1 (Develop, Export), M2 (Library); Map and Print tabs Later | added; spec wins (Map, Print) |
| APP-02 | App menu behind the logo | P0 | M1, grows each milestone | added |
| APP-03 | Quick actions | P1 | M1 to M2 defaults, M5 customization | added |
| APP-04 | Command search (Ctrl K) | P1 | M2, presets added in M3 | added |
| APP-05 | Shortcut sheet, custom shortcuts | P1 | M2 sheet, M5 customization | added |
| APP-06 | Right-click menus | P0 | M1 slider, M2 grid, M3 Develop photo | added |
| APP-07 | Status bar | P2 | M1 | added |
| APP-08 | Panel visibility | P1 | M1 | added |
| APP-09 | Screen modes, Lights Out | P2 | M5 | added |
| APP-10 | Undo and redo | P0 | M1 edits, M2 library actions | spec (history); added (global scope) |
| APP-11 | Preferences, catalog settings | P1 | M2 | added |
| APP-12 | Light theme | P2 | M5 | spec |
| CAT-01 | Catalog database | P0 | M2 | spec |
| CAT-02 | Import | P0 | M2 core and extras, M3 develop preset on import, M4 Copy as DNG | spec; added (extras, DNG) |
| CAT-03 | Previews | P1 | M2 standard and 1:1, M4 smart previews | spec; added (smart previews) |
| CAT-04 | Folders | P0 | M2 | spec |
| CAT-05 | Collections, Quick Collection | P0 | M2 | spec; added (Quick Collection) |
| CAT-06 | Smart collections | P1 | M3 | spec |
| LIB-01 | Grid | P0 | M2 | spec |
| LIB-02 | Loupe | P0 | M2 | spec |
| LIB-03 | Compare, Differences panel, focus metric | P1 | M2 | spec; added (Differences, focus) |
| LIB-04 | Survey | P1 | M2 | spec |
| LIB-05 | Flags | P0 | M2 | spec |
| LIB-06 | Star ratings | P0 | M2 | spec |
| LIB-07 | Color labels | P0 | M2 | spec |
| LIB-08 | Filter bar | P0 | M2 | spec |
| LIB-09 | Filmstrip | P0 | M1 | spec |
| LIB-10 | Stacks | P2 | M3 | spec |
| LIB-11 | People view | P2 | Later | spec |
| LIB-12 | Quick Develop | P2 | M3 (Auto tone in M4) | added |
| MET-01 | Metadata panel | P1 | M2 | spec |
| MET-02 | Keywording panel | P1 | M2 | spec |
| MET-03 | Keyword list | P1 | M2 | spec |
| MET-04 | Keyword sets | P1 | M2 | added |
| MET-05 | Keyword painter | P2 | M3 | added |
| MET-06 | Save metadata to files | P1 | M2, written automatically, Ctrl S forces | spec wins (auto-write) |
| MAP-01 to MAP-03 | Map, geotagging, locations | P2 | Later | spec wins |
| DEV-01 | RAW pipeline | P0 | M0 to M1, in the spec's stage order | spec wins (stage order) |
| DEV-02 | Basic panel | P0 | M1 core and B&W; texture, clarity, dehaze M3; auto M4 | spec wins (split); added (B&W) |
| DEV-03 | Histogram, clipping | P0 | M1 | spec |
| DEV-04 | Tone curve | P0 | M3 | spec |
| DEV-05 | Color mixer | P0 | M3 | spec |
| DEV-06 | Color grading | P1 | M3 | spec |
| DEV-07 | Detail | P0 | M3 | spec |
| DEV-08 | AI denoise | P0 | M4, output as a cached linear raster | spec wins (output) |
| DEV-09 | Lens corrections, defringe | P1 | M3 | spec; added (defringe) |
| DEV-10 | Geometry, Upright | P1 | M3 manual, M4 Upright | spec wins (split) |
| DEV-11 | Effects | P1 | M3 | spec |
| DEV-12 | Crop and rotate | P0 | M1 | spec |
| DEV-13 | Panel states | P1 | M1 edited dots, M3 the rest | added |
| DEV-14 | Calibration | P2 | M3 | added |
| MSK-01 | Masking panel | P0 | M3 | spec |
| MSK-02 | AI selections, people included | P0 | M4 | spec; added (people) |
| MSK-03 | Brush | P0 | M3 | spec |
| MSK-04 | Gradients | P0 | M3 | spec |
| MSK-05 | Range masks | P1 | M3 color and luminance; depth Later | spec wins (depth) |
| MSK-06 | Mask adjustments | P0 | M3 | spec |
| RMV-01 | Remove, heal, clone | P1 | M3 heal and clone, M4 detect-objects assist; content-aware Later | spec wins (content-aware); added (assist) |
| RMV-02 | Red eye | P2 | Later | spec wins |
| WF-01 | Presets | P0 | M3 | spec |
| WF-02 | Before / After | P0 | M1 toggle, M3 full | spec |
| WF-03 | History | P0 | M3 (the data from M1) | spec |
| WF-04 | Snapshots | P1 | M3 | spec |
| WF-05 | Virtual copies | P1 | M3 | spec |
| WF-06 | Copy, paste, sync settings | P0 | M3 | spec |
| WF-07 | Reference view | P2 | Later | spec wins |
| OUT-01 | Export | P0 | M1 basic, M4 full (with AVIF, DNG, email, watermark) | spec; added (extra formats and destinations) |
| OUT-02 | Print | P2 | Later | spec wins |
| OUT-03 | Book, Slideshow, Web | Deferred | Later | spec |
