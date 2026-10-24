# APP-10 · Undo and redo (M1 part)

Undo and redo for develop edits, per photo. This spec covers what M1 builds: the history of each photo's edit, the `Undo` and `Redo` commands, and how the frontend offers them (Ctrl Z, Ctrl Shift Z, the quick actions and the app menu).

Sources: FEATURES.md (APP-10), the Decided table in `docs/development-plan.md` ("Undo scope (APP-10)": per photo, 2026-10-14; "Edit storage in M1": in memory for the session, 2026-10-17), the plan's M1 contract (`Undo`, `Redo`; one history step per finished gesture), DEV-01 (*Inputs and shortcuts*, *Data*) and DEV-02. Where this spec and the technical spec disagree, the technical spec wins and this file is corrected.

**Out of scope for M1**, with their milestone:
- library actions (ratings, flags, labels, moves) in the history: M2, in the history of the photo they change (decided);
- the History panel, with its list of steps and jumping to one: WF-03, M3, built on the steps this spec records;
- snapshots (WF-04, M3);
- a depth setting: catalog settings, before M2 (parked).

## Behavior

### What a step is

The engine owns each photo's edit and its history (DEV-01). A **step** is one finished change to the edit. It stores the whole edit after the change and a label naming what changed.

One step each:
- a slider gesture, from *start* to *end*: the drag of Temp, Tint, a tone or presence slider, the crop handles, the angle, or a held arrow key on a focused slider (DEV-02);
- a value typed into a slider's field;
- a reset: double-click on a slider, the slider menu's Reset (APP-06), the white balance preset As Shot;
- Reset all, which puts the whole edit back to the photo's defaults;
- a choice: the profile, the treatment (V), a white balance preset, a crop aspect;
- a white balance pick (W, `PickWhiteBalance`);
- a quarter turn (Ctrl [, Ctrl ]), a flip;
- a pasted value (APP-06).

A delta without a gesture is a start and an end in one, so it makes one step (DEV-01, *Inputs*). Between *start* and *end* no step is recorded; the frames are interactive.

**A change that leaves the edit as it was makes no step.** That covers a drag that ends where it started and a value clamped back to the current one. The edit is compared after clamping (DEV-01, *Data*).

**Labels** are the control's name as the panel shows it: "Exposure", "Crop", "Rotate" (a turn or a flip), "Reset Exposure", "Reset all". Everything that sets the white balance, whether Temp, Tint, a preset or a pick, is "White balance", because one delta sets the whole of it. A delta that changes several fields at once has one label. The app menu shows them ("Undo Exposure"), and so will the M3 History panel.

### Undo and redo

- **Undo** puts the current photo's edit back to what it was before its last step. **Redo** applies the step undone last again.
- The photo then renders settled, from the earliest stage the restored edit changes (DEV-01, *Re-runs*). Undoing a white balance step re-demosaics, visible area first.
- **A new step after an undo** drops the steps that could have been redone.
- **Per photo:** each photo has its own history. Switching photos keeps them all, and Ctrl Z acts on the photo shown (decided).
- **Depth:** unlimited in M1, for the session (owner, 2026-10-24). A step is a few hundred bytes, so 10,000 steps on one photo hold about 4 MB.
- **During a gesture**, Undo and Redo do nothing. The gesture ends first.
- **Repeated quickly:** every Ctrl Z is one step back. The renders coalesce latest-wins (DEV-01, *Generations*), so only the last one is drawn.
- After undo or redo, the sliders, the edited dots (DEV-13) and the filmstrip's dot (LIB-09) follow the restored edit. Undoing back to the defaults clears the dots.

### The engine's side

- `Undo { photo }` and `Redo { photo }` name the photo they act on (decided).
- Each returns the photo's edit after the change, and whether another undo or redo is possible, with their labels. `GetEdit` returns the same.
- A command for a photo that cannot undo (or redo) changes nothing and returns the edit as it is.

## Inputs and shortcuts

| Command | Shortcut | Also in |
|---|---|---|
| Undo | Ctrl Z (macOS: Cmd Z) | Quick actions (APP-03), app menu › Edit (APP-02) |
| Redo | Ctrl Shift Z (macOS: Cmd Shift Z) | Quick actions, app menu › Edit |

- Both are registered in the shortcut registry, like every command. The menu and the quick actions' tooltips show the label and the platform's shortcut: "Undo Exposure  Ctrl Z" on Windows and Linux, "Undo Exposure  ⌘Z" on macOS.
- **While a text field has focus** (a slider's typed value, a search box), Ctrl Z and Ctrl Shift Z are the field's own undo and redo. The edit's history is not touched.

## States

| State | Undo | Redo |
|---|---|---|
| No photo open, or the photo is unsupported or damaged (DEV-01, *States*) | Disabled | Disabled |
| A photo with no steps yet | Disabled | Disabled |
| Steps to undo | Enabled, labelled with the last step | As below |
| Undone steps to redo | As above | Enabled, labelled with the next step |
| A gesture in progress | Disabled until it ends | Disabled until it ends |

A disabled command shows dimmed in the menu and the quick actions, and its shortcut does nothing.

## Data

In memory in the engine, per photo, for the session (decided "Edit storage in M1"):

| Field | Type | Meaning |
|---|---|---|
| `steps` | list of `{label, edit}` | Every step, oldest first. `edit` is the whole edit struct after the step (DEV-01, *Data*) |
| `position` | integer, 0 to the number of steps | How many steps are applied. The current edit is `steps[position − 1].edit`, or the photo's defaults at 0 |

The M2 catalog stores these same steps in its History step entity. Nothing is written to disk in M1.

## Edge cases

- **Undo to the defaults:** the photo is unedited again; its dots go.
- **A step that changed white balance as shot:** As Shot is stored as a mode, and Temp and Tint are refilled from the file (DEV-01, *Data*). Undo restores the mode, not stale numbers.
- **The crop after a turn:** a turn maps the crop into the new orientation (DEV-01, *Crop*). Undoing it restores the crop as stored before, so the same region returns.
- **A refused delta** (a crop that cannot fit, DEV-01, *Crop*) changes nothing and makes no step.
- **Undo while the previous render is still running:** the new request takes over; work for the older one stops between tiles (DEV-01, *Generations*).
- **A photo whose decode failed after edits were made:** this cannot happen in M1. A photo that does not decode is unsupported from the start, and its controls are disabled.

## Acceptance criteria

Each becomes a test in the engine crate (`engine/tests/history.rs`), against the edit and history, without rendering, unless it says otherwise.

1. **One step per finished change.** A gesture of 50 updates makes one step. A typed value, a reset, Reset all, a pick and a quarter turn each make one.
2. **No step for no change:** a gesture that ends at its start value, and a delta clamped to the current value, leave the history as it was.
3. **Undo and redo round trip:** after n steps, n undos give the defaults, and n redos give the last edit, bit for bit.
4. **A new step drops the redo steps.**
5. **Per photo:** steps on photo A, then photo B, then Undo on A, undo A's last step and leave B's edit and history alone.
6. **During a gesture,** Undo and Redo change nothing.
7. **Labels:** each step's label names its control, and `GetEdit` reports the next undo's and redo's labels.
8. **The render follows:** after undoing a white balance step, the settled frame equals a fresh render of the restored edit, bit for bit (GPU test).
9. **Frontend** (Vitest and Playwright against the fake engine): Ctrl Z and Ctrl Shift Z reach the engine, the quick actions and the menu are disabled with nothing to undo or redo, and a focused text field keeps its own undo.
