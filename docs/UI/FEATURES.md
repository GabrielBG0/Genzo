# Fixer · feature list

Every feature contemplated in the UI mockups, grouped by area. Priorities: P0 core for a first usable version, P1 next, P2 later. Priorities say what is in v1. The build order and each feature's milestone are in `docs/development-plan.md`. Where this list and the technical spec disagree, the spec wins.

Screen codes refer to the mockups in `fixer-ui-handoff.html` (D = Develop, L = Library, W = Workflow and output, M = Menus). "not mocked" means there is no dedicated screen yet.

Spec template for each feature: **Behavior** (what it does, step by step), **Inputs and shortcuts**, **States** (empty, loading, error, disabled), **Data** (what is stored in the catalog), **Edge cases**, **Acceptance criteria**.

## APP · App shell and navigation

### APP-01 · Module switcher
- Priority: P0 · Screens: all
- Scope: Library, Develop, Map, Print and Export as pill tabs in the top bar. G goes to Library grid, D to Develop.
- Open questions: Should module state (zoom, open panels) be remembered per photo or per module?
- Spec: _to be written_

### APP-02 · App menu behind the logo
- Priority: P0 · Screens: M2
- Scope: Compact menu instead of a classic menu bar. Import, Export and recent catalogs on top, then File, Edit, Library, Photo, Develop, View, Window, Help, then Preferences, Catalog settings, Keyboard shortcuts, Customize quick actions, Quit. Opens with Alt.
- Open questions: On macOS, mirror the same structure in the system menu bar?
- Spec: _to be written_

### APP-03 · Quick actions in the top bar
- Priority: P1 · Screens: M5, all
- Scope: One-click buttons next to the app menu, different per module. Default Library: Import, Export, Quick Collection, Undo, Redo. Default Develop: Undo, Redo, Copy settings, Paste settings, Before/After, Reset all, Export. Customizable and reorderable per module, suggestions based on usage, "Pin to top bar" from any right-click menu.
- Open questions: Is usage tracking for suggestions local only and can it be turned off?
- Spec: _to be written_

### APP-04 · Command search (Ctrl K)
- Priority: P1 · Screens: W5
- Scope: One search over commands, presets, photos and collections, with shortcuts shown. Filter chips by type, ">" prefix for commands only.
- Open questions: Lightroom uses Ctrl K for "add keywords". Keep Ctrl K for search and move keywords elsewhere?
- Spec: _to be written_

### APP-05 · Shortcut sheet and custom shortcuts (Ctrl /)
- Priority: P1 · Screens: W6
- Scope: Overlay of shortcuts for the current module, grouped, filterable, Windows and macOS layouts, with a Customize editor.
- Open questions: Import Lightroom shortcut presets?
- Spec: _to be written_

### APP-06 · Right-click menus
- Priority: P0 · Screens: M3, M4
- Scope: Context menus on grid photos (acts on whole selection, mixed states shown with –), on the Develop photo (zoom, background, before/after, snapshot, copy/paste, overlays) and on sliders (reset, copy/paste value, apply to selection, pin).
- Spec: _to be written_

### APP-07 · Status bar
- Priority: P2 · Screens: all
- Scope: Bottom bar with current mode, selection count, active filter, edit count, color space, bit depth, preview status and catalog save state.
- Spec: _to be written_

### APP-08 · Panel visibility
- Priority: P1 · Screens: not mocked
- Scope: Tab hides side panels, Shift Tab hides all, T toolbar, F6 filmstrip, F7 left, F8 right. Alt-click a panel header for solo mode, Ctrl-click to open or close all.
- Open questions: Resizable side panels? Min and max widths?
- Spec: _to be written_

### APP-09 · Screen modes and Lights Out
- Priority: P2 · Screens: not mocked
- Scope: F cycles screen modes, L cycles Lights Out (dim, black), Shift L reverses, Ctrl Shift F full screen.
- Spec: _to be written_

### APP-10 · Undo and redo
- Priority: P0 · Screens: all
- Scope: Global undo/redo (Ctrl Z, Ctrl Shift Z) across library and develop actions, tied to the History panel in Develop.
- Open questions: Undo depth and whether library actions (ratings, moves) share the same stack as edits.
- Spec: _to be written_

### APP-11 · Preferences and catalog settings
- Priority: P1 · Screens: not mocked
- Scope: Ctrl , and Ctrl Alt ,. Defaults for import, previews, performance (GPU), file handling, interface, backups.
- Spec: _to be written_

### APP-12 · Light theme
- Priority: P2 · Screens: not mocked
- Scope: Second theme using the same tokens. Dark (#121212) stays the default.
- Spec: _to be written_

## CAT · Catalog, folders and import

### CAT-01 · Catalog database
- Priority: P0 · Screens: not mocked
- Scope: Single catalog file storing edits, ratings, flags, labels, keywords, metadata, collections and preview references. All editing is non-destructive.
- Open questions: Storage engine (SQLite?) and schema versioning. Read Lightroom catalogs or XMP sidecars for migration?
- Spec: _to be written_

### CAT-02 · Import
- Priority: P0 · Screens: L6
- Scope: Full-window import: source (devices, folders), Copy as DNG / Copy / Move / Add, duplicate skipping, eject after import, second backup copy, preview size, smart previews, rename template with sample, apply develop preset, metadata preset and keywords, destination with subfolder and date organization, import presets.
- Open questions: Watch folders / tethered capture in scope?
- Spec: _to be written_

### CAT-03 · Previews
- Priority: P1 · Screens: not mocked
- Scope: Standard and 1:1 previews, smart previews for offline editing, background building with progress in the status bar.
- Open questions: Preview cache size limits and location.
- Spec: _to be written_

### CAT-04 · Folders
- Priority: P0 · Screens: L1
- Scope: Folder tree mirrors the disk, with photo counts. New folder (Ctrl Shift N), show in Explorer (Ctrl R), rename (F2), remove (Delete).
- Open questions: Handling missing or moved folders.
- Spec: _to be written_

### CAT-05 · Collections and Quick Collection
- Priority: P0 · Screens: L1
- Scope: Collections and collection sets, independent of folders. Quick Collection: B adds, Shift B adds and advances, Ctrl B shows it. New collection Ctrl N.
- Spec: _to be written_

### CAT-06 · Smart collections
- Priority: P1 · Screens: L7
- Scope: Rule-based collections with match all/any, nested groups, drag to reorder rules, live match count and preview strip.
- Open questions: Full list of rule fields.
- Spec: _to be written_

## LIB · Library views and culling

### LIB-01 · Grid view (G)
- Priority: P0 · Screens: L1
- Scope: Thumbnail grid with index number, flag, stars, color label, virtual copy and edited badges. Thumbnail size, sort by capture time and others.
- Spec: _to be written_

### LIB-02 · Loupe view (E)
- Priority: P0 · Screens: L2
- Scope: Single photo with info overlay (I, Ctrl I cycles), Fit and 1:1 zoom, Space for temporary zoom, arrows for previous and next, floating rating toolbar.
- Spec: _to be written_

### LIB-03 · Compare view (C)
- Priority: P1 · Screens: L3
- Scope: Select vs candidate side by side with linked zoom, swap, promote candidate. Includes a Differences panel that highlights differing EXIF, focus, rating and label (new, not in Lightroom).
- Open questions: How is "focus: sharp/soft" computed?
- Spec: _to be written_

### LIB-04 · Survey view (N)
- Priority: P1 · Screens: L4
- Scope: Several photos tiled, the active one takes ratings and flags, rejected ones dim, remove from survey.
- Spec: _to be written_

### LIB-05 · Flags
- Priority: P0 · Screens: L1, L2
- Scope: P pick, X reject, U unflag. Shift P, Shift X, Shift U apply and move to next.
- Spec: _to be written_

### LIB-06 · Star ratings
- Priority: P0 · Screens: L1, L2
- Scope: 1 to 5 set stars, 0 clears, [ and ] change rating.
- Spec: _to be written_

### LIB-07 · Color labels
- Priority: P0 · Screens: L1
- Scope: 6 red, 7 yellow, 8 green, 9 blue, plus purple from menus.
- Open questions: Custom label names (label sets)?
- Spec: _to be written_

### LIB-08 · Filter bar
- Priority: P0 · Screens: L1
- Scope: Text, Attribute (flag, rating, label, kind) and Metadata columns (camera, lens, date, keyword...). Saved filters and a lock to keep the filter across sources.
- Spec: _to be written_

### LIB-09 · Filmstrip
- Priority: P0 · Screens: all
- Scope: Strip of the current source with selection ring and edited dot. Present in all modules.
- Spec: _to be written_

### LIB-10 · Stacks
- Priority: P2 · Screens: M3
- Scope: Group into stack (Ctrl G), expand and collapse.
- Spec: _to be written_

### LIB-11 · People view (O)
- Priority: P2 · Screens: L5
- Scope: On-device face detection and grouping, named and unnamed groups, name suggestions, merge by drag. Face data stays in the catalog. Optional write names to keywords.
- Open questions: Which detection model, and its license and size?
- Spec: _to be written_

### LIB-12 · Quick Develop
- Priority: P2 · Screens: L2
- Scope: Preset, white balance, exposure and contrast step buttons, Auto tone and Reset from the Library.
- Spec: _to be written_

## MET · Metadata and keywords

### MET-01 · Metadata panel
- Priority: P1 · Screens: L1, L2
- Scope: EXIF and IPTC fields, metadata presets, mixed values for multi-selection, sync metadata.
- Open questions: Which IPTC fields are editable in v1?
- Spec: _to be written_

### MET-02 · Keywording panel
- Priority: P1 · Screens: L1
- Scope: Keyword tags field with partial markers for keywords on only some of the selection, suggestions.
- Open questions: Shortcut for adding keywords if Ctrl K is command search.
- Spec: _to be written_

### MET-03 · Keyword list
- Priority: P1 · Screens: L8
- Scope: Hierarchical keywords with counts, filter, People/Other tabs, check states (all, some, none) for the selection.
- Spec: _to be written_

### MET-04 · Keyword sets
- Priority: P1 · Screens: L1
- Scope: Nine keywords applied with Alt 1 to 9, switchable sets.
- Spec: _to be written_

### MET-05 · Keyword painter (Ctrl Alt K)
- Priority: P2 · Screens: L8
- Scope: Click or drag over photos to apply keywords, Alt to erase.
- Open questions: Painter for other attributes (labels, ratings, presets)?
- Spec: _to be written_

### MET-06 · Save metadata to files
- Priority: P1 · Screens: not mocked
- Scope: Ctrl S writes XMP sidecars or embeds in DNG.
- Open questions: Auto-write XMP option?
- Spec: _to be written_

## MAP · Map

### MAP-01 · Map module
- Priority: P2 · Screens: L9
- Scope: Dark, satellite and terrain styles, place search, clustered pins with counts, popover with thumbnails, zoom controls.
- Open questions: Map tile provider and offline use.
- Spec: _to be written_

### MAP-02 · Geotagging
- Priority: P2 · Screens: L9
- Scope: Drag photos from the filmstrip onto the map, GPX tracklog import with time offset, auto-tag untagged photos.
- Spec: _to be written_

### MAP-03 · Saved and private locations
- Priority: P2 · Screens: L9
- Scope: Named locations with radius. Private locations are stripped on export.
- Spec: _to be written_

## DEV · Develop

### DEV-01 · RAW processing pipeline
- Priority: P0 · Screens: not mocked
- Scope: Non-destructive pipeline in this order: RAW, profile, white balance, tone, color, detail, optics, geometry, effects, local adjustments, crop, output.
- Open questions: RAW decoding library and supported cameras. GPU processing approach.
- Spec: _to be written_

### DEV-02 · Basic panel
- Priority: P0 · Screens: D1
- Scope: Profile, Color / B&W treatment (V), white balance presets and picker (W), Temp, Tint, Exposure, Contrast, Highlights, Shadows, Whites, Blacks, Texture, Clarity, Dehaze, Vibrance, Saturation. Auto tone Ctrl U, auto white balance Ctrl Shift U. Changed sliders shown bright, unchanged dim.
- Spec: _to be written_

### DEV-03 · Histogram and clipping warnings
- Priority: P0 · Screens: D1
- Scope: RGB histogram with EXIF row, clipping corner markers, J toggles shadow (blue) and highlight (red) overlays, Alt-drag on sliders previews clipping.
- Spec: _to be written_

### DEV-04 · Tone curve
- Priority: P0 · Screens: D2
- Scope: Parametric and point curves, RGB plus R, G, B channels, presets, input/output readout, targeted adjustment by dragging on the photo.
- Spec: _to be written_

### DEV-05 · Color mixer
- Priority: P0 · Screens: D3
- Scope: HSL and Color modes, Hue, Saturation, Luminance or All for 8 colors, colored slider tracks, targeted adjustment that links the sampled colors.
- Spec: _to be written_

### DEV-06 · Color grading
- Priority: P1 · Screens: D4
- Scope: 3-way wheels (shadows, midtones, highlights) plus global, luminance per range, blending and balance.
- Spec: _to be written_

### DEV-07 · Detail
- Priority: P0 · Screens: D5
- Scope: Sharpening (amount, radius, detail, masking with Alt preview), noise reduction (luminance, detail, contrast, color, smoothness), 1:1 detail preview.
- Spec: _to be written_

### DEV-08 · AI Denoise (Enhance)
- Priority: P0 · Screens: D5
- Scope: Enhance dialog with before/after preview and amount, output cached as a linear raster that the edit references (no new file), time estimate. Super resolution option planned for later.
- Open questions: Model choice and where it runs (GPU, CPU fallback).
- Spec: _to be written_

### DEV-09 · Lens corrections
- Priority: P1 · Screens: D6
- Scope: Remove chromatic aberration, profile corrections picked from EXIF with distortion and vignetting amounts, manual distortion and vignetting, defringe purple and green.
- Open questions: Lens profile database source (for example lensfun).
- Spec: _to be written_

### DEV-10 · Geometry and Upright
- Priority: P1 · Screens: D7
- Scope: Upright Off, Auto, Guided, Level, Vertical, Full. Guided mode with 2 to 4 drawn guides and a magnifier. Manual vertical, horizontal, rotate, aspect, scale, X and Y offset, constrain crop.
- Spec: _to be written_

### DEV-11 · Effects
- Priority: P1 · Screens: D8
- Scope: Post-crop vignette (style, amount, midpoint, roundness, feather, highlights) and grain (amount, size, roughness) with grain presets.
- Spec: _to be written_

### DEV-12 · Crop and rotate (R)
- Priority: P0 · Screens: D9
- Scope: Crop frame with handles, aspect presets and lock, angle slider and straighten tool, constrain to image, overlays (O cycles: thirds, golden, grid), X swaps orientation, rotate 90° (Ctrl [ ]), flip, pixel size readout.
- Spec: _to be written_

### DEV-13 · Panel states
- Priority: P1 · Screens: D1
- Scope: Collapsible panels with a dot on edited panels, per-panel reset, show "from preset" when a preset set the values, panel on/off.
- Spec: _to be written_

### DEV-14 · Calibration
- Priority: P2 · Screens: D8
- Scope: Shadows tint and RGB primaries hue and saturation.
- Spec: _to be written_

## MSK · Masking and retouching

### MSK-01 · Masking panel (Shift W)
- Priority: P0 · Screens: D10
- Scope: Floating mask list over the photo, masks made of components combined with add, subtract and intersect, overlay color and Alt O to cycle overlay mode, pins hidden with H, invert, delete.
- Spec: _to be written_

### MSK-02 · AI selections
- Priority: P0 · Screens: D10
- Scope: Subject, Sky, Background, Objects, People. Runs on device. AI assists selection only, never generates pixels.
- Open questions: Segmentation models and their licenses. Re-detection when masks are pasted to other photos.
- Spec: _to be written_

### MSK-03 · Brush (K)
- Priority: P0 · Screens: D11
- Scope: Brush A, Brush B and Erase, size, feather, flow, density, auto mask. [ ] resize, Shift [ ] feather.
- Open questions: Pen pressure support.
- Spec: _to be written_

### MSK-04 · Linear and radial gradients
- Priority: P0 · Screens: D10
- Scope: Linear (M) and radial (Shift M) gradient masks with pins and handles.
- Spec: _to be written_

### MSK-05 · Range masks
- Priority: P1 · Screens: D11
- Scope: Color range (Shift J), luminance range (Shift Q) and depth range as standalone masks or refinements, with sampler and range bar.
- Open questions: Depth range needs depth data. Source of depth?
- Spec: _to be written_

### MSK-06 · Mask adjustments
- Priority: P0 · Screens: D10
- Scope: Per-mask amount and sliders (temp, tint, exposure, contrast, highlights, shadows, clarity, dehaze, saturation, sharpness, noise) and mask presets.
- Spec: _to be written_

### RMV-01 · Remove, heal, clone (Q)
- Priority: P1 · Screens: D12
- Scope: Heal and clone. Detect objects as AI assist for the selection. Content-aware remove that fills from nearby pixels (no generative AI) comes after v1, per the technical spec. Movable source area and "/" for a new source. Size, feather, opacity, visualize spots, list of spots.
- Spec: _to be written_

### RMV-02 · Red eye
- Priority: P2 · Screens: not mocked
- Scope: Red eye and pet eye correction tool from the tool strip.
- Spec: _to be written_

## WF · Workflow

### WF-01 · Presets
- Priority: P0 · Screens: W1
- Scope: Preset browser with search, favorites, folders, hover preview on the photo and Navigator, amount slider (0 to 200), update, create, import, export, apply during import.
- Open questions: Preset file format, and import of Lightroom .xmp presets?
- Spec: _to be written_

### WF-02 · Before / After
- Priority: P0 · Screens: W2
- Scope: Y side by side, Alt Y top and bottom, Shift Y split, \ hold before. Copy after to before, before to after, swap. Choose what "before" is: import, a snapshot or a history step.
- Spec: _to be written_

### WF-03 · History
- Priority: P0 · Screens: D1
- Scope: Timeline of edits with values, click to go back, clear history.
- Spec: _to be written_

### WF-04 · Snapshots
- Priority: P1 · Screens: W3
- Scope: Named saved states of a version, created from a popover (Ctrl N), restorable.
- Spec: _to be written_

### WF-05 · Virtual copies
- Priority: P1 · Screens: W3
- Scope: Multiple versions of one RAW without duplicating the file (Ctrl '), version strip, each with its own history, set as master, compare, delete.
- Spec: _to be written_

### WF-06 · Copy, paste and sync settings
- Priority: P0 · Screens: W4
- Scope: Grouped checkboxes with All / Modified only / None, changed-on-this-photo markers, crop and spots off by default, paste to selection, paste previous (Ctrl Alt V), sync settings.
- Spec: _to be written_

### WF-07 · Reference view (Shift R)
- Priority: P2 · Screens: not mocked
- Scope: Pin a reference photo next to the active one while editing to match looks.
- Spec: _to be written_

## OUT · Output

### OUT-01 · Export
- Priority: P0 · Screens: W7
- Scope: Export presets, destination (folder, email, another app), file naming template, JPEG, AVIF, TIFF, PNG, DNG, quality, color space, size limit, resize to fit, resolution, output sharpening, metadata options including stripping location and people names, watermark, runs in background, size estimate.
- Open questions: Publish services (online destinations) in scope?
- Spec: _to be written_

### OUT-02 · Print
- Priority: P2 · Screens: W8
- Scope: Templates, single, grid and custom layouts, margins and cells, captions and footer, print to printer or file, print resolution, print sharpening, 16-bit output, printer profile, rendering intent, print adjustment, soft proofing.
- Spec: _to be written_

### OUT-03 · Book, Slideshow, Web
- Priority: Deferred · Screens: not mocked
- Scope: Skipped for now by decision.
- Spec: _to be written_
