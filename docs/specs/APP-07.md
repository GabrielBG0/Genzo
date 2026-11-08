# APP-07 · Status bar (M1 part)

The bar along the bottom of the window. This spec covers what M1 builds (plan, M1 frontend item 11: "mode pill, color space, bit depth, preview status, the software-adapter notice, and why a photo shows only its embedded preview"), and the photo notes other M1 specs send to it.

The frontend part landed first; the photo notes need the engine's part (`PhotoEdit.notes`), which follows in its own PR. While the photo shows its embedded preview, the viewport draws it (DEV-01, *Opening*), which the frontend did not do before this spec.

Sources: FEATURES.md (APP-07), the handoff's Status bar component, DEV-01 (*States*: Opening, Rendering after a change, Unsupported, Software adapter; the notes on skipped lens corrections, an unusable color profile and a missing as-shot white), DEV-02 (the unreadable tone curve note, the picker's refusals), LIB-09 (a folder that cannot be read), OUT-01 (an export's progress). Where this spec and the technical spec disagree, the technical spec wins and this file is corrected.

**Out of scope for M1**, with their milestone: the selection count and the active filter (M2, with the Library's selection and filters), the edit count and the catalog's save state (M2, with the catalog), previews built in the background ("Previews 96/96", CAT-03).

## Behavior

The bar has three parts, left to right.

### Left: where you are

1. **The mode pill**: the module, "Develop" in M1.
2. **The folder's count**: "12 photos", or nothing before a folder is open.
3. **The photo's file name**, brighter: the photo shown.

### Middle: the notice

One message at a time, the most urgent first:

1. **An export's progress** (OUT-01): "Exporting DSC_0412.jpg, 40 %", "Exported …", "Export failed: …", "Export cancelled". It is a button that offers Cancel export while the export runs.
2. **A refusal** just made: the picker's "Can't set white balance here: the area is clipped", a folder that cannot be read (LIB-09), a command's error. It goes when the next command succeeds, or after 6 s.
3. **Why the photo shows only its embedded preview** (DEV-01, *Unsupported*): "Nikon High Efficiency RAW is not supported yet", with the Develop controls disabled.
4. **The photo's notes**: what the engine could not do as the file asks, while the photo renders. For example "Lens corrections from the file are not applied yet", "This photo's color profile can't be used; showing camera colors", "The as-shot white balance is unavailable; using D55", "The file's tone curve can't be read; using the default" (DEV-01, DEV-02). With several, the first shows with "+N"; its tooltip lists them all.
5. **The software-adapter notice** (DEV-01, *Software adapter*): why the GPU is not used, while it applies.

The notice is in the rating color (the handoff's warning tone), on one line, cut with an ellipsis when long; its tooltip has the whole text.

### Right: what you see

1. **The preview status**, of the photo shown:
   - "Opening" while the photo decodes and no frame of it is shown yet, with no embedded preview;
   - "Embedded preview" while its embedded preview is shown, before its first render, and for an unsupported photo that has one ("No preview" for one that has none);
   - "Interactive" during a slider gesture, whose frames are the drag preview (DEV-01);
   - "Settled" otherwise.
2. **The color space** of the viewport: "sRGB" or "Display P3", as the engine renders it (DEV-01, *Color rules*).
3. **The bit depth** of the frames shown: "8-bit".
4. **The adapter**: the GPU's name, or "Fake engine" in the browser build.

Each fact has a tooltip saying what it means ("Frames are rendered in sRGB").

## Inputs and shortcuts

None of its own. The export notice is a button (OUT-01). The bar is not focusable otherwise.

## States

| State | Left | Notice | Right |
|---|---|---|---|
| No folder open | Develop | The adapter notice, if any | sRGB, 8-bit, the adapter |
| A photo opening | Develop · count · name | as above | Opening or Embedded preview |
| A photo rendered | as above | its notes, if any | Settled |
| A drag | as above | as above | Interactive |
| An unsupported photo | as above | why | Embedded preview |

## Data

Nothing is stored.

## The engine's side (photo notes)

- `PhotoEdit.notes` (contract 0.6.2, additive): the open photo's notes, as the strings above, in the order the stages meet them. They come with every edit reply, so a switch brings the new photo's notes.
- The engine gathers them when the photo's input is decoded: stage 1's skipped non-optional lens opcodes, stage 4's `ColorFallback` (no usable profile; the as-shot white unsolved), and stage 7's base curve note. Until the input is decoded, `notes` is empty; the frontend asks `GetEdit` again after the photo's first frame, so its notes show from then on.

## Edge cases

- **A long file name** is cut with an ellipsis before the notice is; the right-hand facts never are.
- **An export ends while a refusal shows**: the refusal stays until its time ends, then the next notice.
- **A switch during an export**: the export's progress stays; the new photo's notes wait.
- **The software adapter and an unsupported photo**: the unsupported reason shows; the adapter notice comes back on the next photo.

## Acceptance criteria

1. **Left:** "Develop", the folder's count and the file name; nothing but the pill before a folder (Vitest of `StatusBar`, Playwright).
2. **Preview status:** Opening or Embedded preview after a switch, Settled after the first frame, Interactive during a drag (Vitest against the fake engine's events).
3. **Notice order:** with an export running, a refusal, a reason, notes and the adapter notice all present, the export's shows; each removed in turn shows the next (Vitest of the notice's choice).
4. **A refusal goes** after 6 s or when the next command succeeds.
5. **Notes:** several notes show the first with "+N" and all in the tooltip.
6. **Engine** (part 2): a DNG with a non-optional lens opcode, a camera without a usable profile, and a file with an invalid ProfileToneCurve each answer their note in `PhotoEdit.notes`; a plain JPEG none (engine tests, with synthetic files where the corpus has none).
