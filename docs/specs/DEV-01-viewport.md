# DEV-01 · The Develop viewport (M1 frontend)

The photo in Develop: Fit and 1:1, Space for a temporary zoom, pan, and the photo toolbar. This spec covers the plan's M1 frontend item 8 ("Fit and 1:1 zoom, with Space for a temporary zoom. Zoom and pan as a GPU transform over the tiles already held; pan reports the visible rectangle through `SetView`. The color rules from M0. Photo toolbar and badges on the flat Overlay surface"). The engine's side is DEV-01's (*Inputs*: `SetView`; *Readback*; *Frames*), already built.

Sources: the plan (M1 frontend item 8; *What M0 hands to M1*, items 3 to 6), DEV-01.md, the handoff's D1 screen (the photo toolbar: "Loupe · Before / After · … · Fit 48%") and its Overlay surface, FEATURES.md (LIB-02's "Fit and 1:1 zoom, Space for temporary zoom", which Develop shares; APP-05's zoom on the Develop photo's menu, which comes with APP-05), APP-08 (T, the toolbar). Where this spec and the technical spec disagree, the technical spec wins and this file is corrected.

**Out of scope for M1:** zoom levels other than Fit and 1:1 (Fill, 2:1, 1:4: M3, with the Navigator), the Navigator panel, the photo's context menu (APP-05), the side-by-side before and after (WF-02, M3), the rating and flag on the toolbar (M2).

## Behavior

### Zoom

- **Two levels:** *Fit*, the whole photo in the view, and *1:1*, one pixel of the photo per pixel of the screen (device pixels, so a 2× display shows 1:1 at its own resolution).
- **Z** toggles Fit and 1:1 (`view.zoom`). From Fit, 1:1 opens on the point under the pointer when it is over the photo, otherwise on the photo's center.
- **A click on the photo** (pressed and released without moving more than 3 px) zooms to 1:1 on the clicked point at Fit, and back to Fit at 1:1. With the white balance picker on, a click picks instead (DEV-02).
- **Space held** zooms to 1:1 on the point under the pointer (the center when the pointer is elsewhere), and releasing it returns to Fit (`view.zoomHold`). A drag while it is held pans. At 1:1 already, Space does nothing. Key repeats are ignored. With a button, slider, select or menu item focused, Space is that control's (it presses a button), as everywhere.
- **The zoom stays across photos**: a switch at 1:1 opens the next photo at 1:1, on its center.
- **The readout** in the toolbar: "Fit 48%" (the Fit zoom in percent of 1:1, rounded) or "1:1".

### Pan

- At 1:1, a **drag** on the photo pans it (the grab cursor, grabbing while dragging), and so does the **wheel** or a trackpad scroll (Shift with a wheel pans sideways). At Fit nothing pans.
- The photo never leaves a gap it could fill: a side larger than the view is clamped to its edges, and a side smaller than the view is centered.
- At 1:1 the view's top left is always on a whole photo pixel, as the engine snaps it (DEV-01, *Readback*), so the tiles held stay valid across pans.
- A window resize at 1:1 keeps the point at the view's center.

### What the frontend draws (GPU transform over what it holds)

- The **Fit frame** stays held as the backdrop. At 1:1 it is drawn scaled up, under the 1:1 tiles, so a newly exposed area shows at once, soft, until its tiles come.
- The **1:1 tiles** are those of the newest 1:1 frame. Each frame names every tile visible (DEV-01, *Readback*), so the frontend keeps one frame's tiles, placed by their rect in the shown image. A pan moves them on the GPU before the engine answers; the engine's next frame replaces them.
- Zooming is a change of the same transform, so the photo moves to 1:1 at once (soft), then sharpens.
- Back at Fit, the 1:1 tiles are dropped.
- Frames older than the newest drawn are dropped (DEV-01, *Frames*), and a switch clears both layers until the new photo's first frame.
- **Color rules** (M0, item 6) are unchanged: `unpackColorSpace` stays sRGB, `drawingBufferColorSpace` follows the frame, JPEG tiles decode with `colorSpaceConversion: 'none'`.

### What the engine is told (`SetView`)

- At Fit: `{ type: 'fit' }` and the view's size in device pixels, as before.
- At 1:1: `{ type: 'region', zoom: 1, x, y }`, the view's top left in shown-image pixels (negative on a side smaller than the view, which is centered).
- During a pan, at most once per animation frame, and once more when the pan stops.
- After a switch has answered, again: the engine opens each photo at Fit (`Develop::select`), so at 1:1 the new photo's region follows its first reply.
- The shown image's size comes with every edit reply (`PhotoEdit.shown`, contract 0.6.3, additive), with the oriented size before the crop (`PhotoEdit.oriented`, for DEV-12). They follow the crop and the orientation, and the before's while the before shows.

### The photo toolbar

- On the flat **Overlay** surface (no blur, CLAUDE.md), at the bottom center of the photo, 42 px tall, a pill.
- **Before / After** (`develop.beforeAfter`, \\), on while the before shows (WF-02). **Clipping** (`develop.clipping`, J), on while either overlay is (DEV-03). **Fit · 1:1**, a segmented control with one sliding indicator (APP-01), and the readout.
- Each button's tooltip names its shortcut. They are disabled as their commands are.
- **T** hides and shows the toolbar (`view.toolbar`), for the session. It shows only while a photo renders.
- The **Before** badge (WF-02) stays at the top left.

### The picker and the histogram at 1:1

- **The picker** maps a click through the same transform to the shown image's normalized coordinates, as at Fit (DEV-02).
- **The histogram** at 1:1 is the engine's whole-photo count (DEV-03); the session takes it as any count.

## Inputs and shortcuts

| Input | Effect |
|---|---|
| Z | Fit ↔ 1:1, at the pointer |
| Space (held) | 1:1 at the pointer until released |
| Click on the photo | Fit → 1:1 at the point; 1:1 → Fit |
| Drag, wheel (at 1:1) | Pan |
| T | Hide or show the photo toolbar |

## States

| State | Viewport | Toolbar |
|---|---|---|
| No photo, or an unsupported one | The placeholder or the embedded preview, Fit | Hidden |
| Fit | The whole photo | Fit lit, "Fit 48%" |
| 1:1 | The Fit frame scaled up, the 1:1 tiles over it | 1:1 lit |
| Space held | As 1:1 | 1:1 lit |

## Data

Nothing is stored. The zoom and the toolbar's visibility are kept for the session.

## Edge cases

- **A photo smaller than the view:** at 1:1 it is centered and does not pan; Fit can be larger than 100% ("Fit 140%"), drawn from the Fit frame.
- **A crop or a turn at 1:1:** the shown size changes with the edit reply; the view keeps its center, clamped.
- **Before at 1:1:** the before's frames come as any frame; the shown size is the before's (the defaults have no crop).
- **A switch while Space is held:** the next photo opens at 1:1; release returns to Fit.
- **Space while a dialog or menu is open:** nothing (the registry's commands are off there).

## Acceptance criteria

1. **Zoom logic** (Vitest of the zoom state): Fit's zoom and origin; 1:1 at a point keeps that point under the pointer; the pan clamps and centers; the origin is whole pixels at 1:1; resize keeps the center; the picker's point maps through the transform at both levels.
2. **Commands** (Playwright against the fake engine): Z and the toolbar's 1:1 send a region view and the readout says 1:1; Z again and Fit send Fit; Space held zooms and its release returns; a drag at 1:1 sends a new origin; T hides the toolbar.
3. **Drawing** (Playwright): at 1:1 the fake's region tiles are drawn (a pixel probe differs from the scaled-up Fit frame where the fake draws 1:1 detail).
4. **Engine** (`tests/history.rs`): the sizes follow the orientation and the crop.
