# DEV-12 · Crop and rotate (R)

The crop tool: the crop frame over the whole photo, aspect presets and lock, the straighten angle and tool, quarter turns and flips, the overlays and the pixel size. This spec covers what M1 builds (plan, M1 frontend item 6: "Handles drawn as an overlay, aspect presets and lock, angle slider and straighten tool. O cycles the overlays, X swaps orientation, Ctrl [ ] rotates by 90°, plus flip and a pixel size readout"). The edit's geometry (its coordinates, the fit, the minimum, how a turn or flip moves the crop) is DEV-01's (*Crop and rotation coordinates*), already built; this spec adds the tool and the one engine command it needs.

Sources: FEATURES.md (DEV-12), the handoff's D9 screen (the panel: "Crop and rotate ●, Reset; Aspect Original · 3:2 ⌄ with 3:2, 4:5, 1:1, 16:9, Custom…; Angle, Rotate, +1.2°; Straighten, Auto; Constrain to image; ↺ 90°, ↻ 90°, Flip H, Flip V; W 5184, H 3456; Reset, Close"), DEV-01.md, DEV-02 (the slider rows, double-click reset), APP-01 (the sliding indicator). Where this spec and the technical spec disagree, the technical spec wins and this file is corrected.

**Decided** (owner, 2026-11-08): **while the tool is open, the photo shows whole and level, and the crop frame tilts over it** with the straighten angle. The engine renders the photo without its crop (`ShowUncropped`); closing the tool shows the cropped, straightened result. (Lightroom turns the photo under a level frame instead; that needs the engine to render outside the photo.)

**Out of scope for M1:** Auto straighten (M4, with the auto tools), Upright and the Transform panel (DEV-10), a typed custom ratio ("Custom…" is the free aspect in M1), crop presets, remembering the last aspect across launches, the crop at 1:1 (the tool is at Fit).

## Behavior

### Opening and closing

- **R** (`develop.crop`), or the crop tool in the histogram's tool strip, opens the tool for a photo that renders. R again, **Enter**, **Esc** or **Close** closes it; the strip's Edit tool too. Esc is the one cancel command (`develop.pickerCancel`): it turns the straighten tool off first, then closes. Esc does not undo: every change is already an edit step, which Ctrl Z undoes.
- While it is open:
  - the right panel shows **Crop and rotate** under the tool strip, in place of Basic; the tool strip's indicator is on the crop tool;
  - the viewport is at Fit (Z, Space and a click do not zoom) and shows the whole oriented photo, without its crop and level (`ShowUncropped`);
  - the white balance picker and the before are off, and their commands are disabled.
- A photo switch keeps the tool open, on the new photo. A photo that does not render closes it.

### The crop frame (the overlay)

- Drawn over the photo: outside the frame the photo is dimmed (the `--crop-dim` token, 55 % black); the frame has a 1 px white border, eight handles (corners and edge midpoints, 9 px squares, white with a dark ring), and the overlay inside it. It tilts with the angle, about its center.
- **Drag a corner** to resize from the opposite corner; **an edge** from the opposite edge. With the aspect locked, a corner keeps the aspect, and an edge resizes the other side about the center to keep it.
- **Drag inside** the frame to move it.
- **Drag outside** the frame to rotate it about its center (the angle follows the pointer's angle around the center).
- Rotate and flip (Ctrl [ , Ctrl ], Photo › Flip horizontal and Flip vertical) work outside the tool too.
- Each drag is one gesture: *start*, *updates* once per animation frame, *end* (as a slider's, DEV-02), and one history step ("Crop"). The engine fits every value it gets (DEV-01: inside the oriented photo, at least 32 px, the aspect kept, the center never moved), and the frame shows the engine's answer; a refused value keeps the last valid one.
- **The cursor** says what a drag does: resize arrows on handles, move inside, a rotate cursor outside.

### The overlay inside the frame

- **O** cycles: rule of thirds, golden ratio, grid (a 1/8 grid), none. It starts at thirds, and is kept for the session.
- Thin lines, white at 50 % opacity, flat.

### The panel

From top to bottom, as in the handoff:

1. **Header** "Crop and rotate", with the edited dot when the crop or the orientation differs from the defaults (DEV-13's rule).
2. **Aspect**: a select (Original, Free, 1:1, 4:5, 5:7, 3:2, 16:9) and the **lock** button.
   - *Original* is the oriented photo's own ratio. *Free* unlocks.
   - Choosing a ratio sets the lock and fits the largest crop of that ratio about the current center, inside the photo at the current angle (the engine shrinks it if needed).
   - The ratio's orientation follows the current frame: a landscape frame takes 3:2, a portrait one 2:3. **X** swaps it: the frame becomes portrait or landscape about its center, as large as fits.
   - The lock toggles with the button. Unlocked, handles resize freely; the select then shows *Free*.
3. **Angle**: a slider from −45° to +45°, 0.1° steps (Shift ×10 by the arrows), double-click resets to 0, its value shown "+1.2°". It is an *edit* slider (it changes the photo).
4. **Straighten**: a button that turns the straighten tool on. Then a drag on the photo draws a line, and on release the angle becomes the one that makes that line level (or plumb, when it is closer to vertical). The tool then turns off. Esc cancels it.
5. **Rotate and flip**: ↺ 90° (**Ctrl [**), ↻ 90° (**Ctrl ]**), Flip H, Flip V. Flip V is a horizontal flip and a half turn, one step. The crop follows the image (DEV-01).
6. **Size**: "W 5184 · H 3456", the crop's size in pixels (the oriented size times `w` and `h`, rounded), mono.
7. **Reset** and **Close** (Enter). Reset sends `Reset(Orientation)`, which resets the quarter turns, the flip and the crop together (DEV-01), as one step.

"Constrain to image" is always on (DEV-01: the engine never lets the crop leave the photo), so it is not a control.

### What the engine is asked

- **`ShowUncropped { photo, uncropped }`** (contract 0.6.4, additive): the photo renders without its crop and level, in its orientation, until the tool closes or the photo switches. Nothing is recorded. Every change to the edit keeps it. `PhotoEdit.uncropped` says it, and `PhotoEdit.shown` is then the oriented size.
- The crop goes as `Change::Crop` in gestures; turns as `Turn`, flips as `Flip`; resets as `Reset(Crop)` and `Reset(Orientation)`.
- The histogram counts what shows, the whole photo while the tool is open.

## Inputs and shortcuts

| Input | Effect |
|---|---|
| R | Open or close the tool |
| Enter, Esc | Close |
| O | Next overlay |
| X | Swap the aspect's orientation |
| Ctrl [ , Ctrl ] | Rotate 90° left, right |
| Drag a handle | Resize (keeping the aspect when locked) |
| Drag inside, outside | Move, rotate |
| Double-click the angle | Reset the angle |

## States

| State | Viewport | Panel |
|---|---|---|
| Closed | The crop | Basic |
| Open | The whole photo, the frame over it, Fit | Crop and rotate |
| Straightening | As open, a line while dragging | The Straighten button on |
| A photo that does not render | Its preview | The tool closes |

## Data

The crop, the quarter turns and the flip are the edit's (DEV-01). The aspect, its lock and the overlay are kept for the session.

## Edge cases

- **A tiny photo** (under 32 px on a side) cannot be cropped on that side (DEV-01): the engine keeps it whole.
- **A ratio that cannot fit** at the angle: the engine shrinks it about the center; the ratio holds.
- **Rotating with a large crop**: the engine shrinks the crop so its corners stay inside the photo.
- **Undo while open**: the frame follows the edit's crop again.
- **Export while open** exports the crop (the uncropped view is the tool's only).

## Acceptance criteria

1. **Engine** (`tests/before.rs`'s kind, GPU): `ShowUncropped` renders the oriented photo whole; a crop delta while it is on keeps it; a switch ends it; `PhotoEdit.shown` is the oriented size while it is on.
2. **Geometry in the frontend** (Vitest): handle drags with and without the lock, a move, a rotation about the center, a ratio about the center, X, and the straighten line's angle, in normalized oriented coordinates.
3. **Tool** (Playwright against the fake): R opens the panel and the frame; a corner drag records one "Crop" step and the size readout changes; 1:1 locks the aspect; Ctrl ] turns and the size swaps; Flip H; the angle slider; O cycles; Enter closes and the photo shows cropped.
