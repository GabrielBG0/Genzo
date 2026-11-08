# APP-06 · Right-click menus (M1 part: sliders)

The context menu of a Develop slider. This spec covers what M1 builds (plan, M1 frontend item 10: "reset, copy value, paste value"). The grid's menu comes in M2, the Develop photo's in M3.

Sources: FEATURES.md (APP-06, with its Decided header style), the handoff's Menu-4 slider menu, DEV-02 (the sliders, reset by double-click, typed values), APP-10 (one step per change). Where this spec and the technical spec disagree, the technical spec wins and this file is corrected.

**Out of scope for M1**, with their milestone:
- **Reset Basic panel:** it needs its own step label ("Reset Basic"), so it waits for DEV-13's panel resets. Double-clicking a group label already resets that group.
- **Apply to all selected photos:** M2, which brings a selection of several.
- **Show in history:** M3, with the History panel (WF-03).
- **Pin to top bar:** with quick-action customization (APP-03, later).

## Behavior

- **Right-click a slider** in the Basic panel (its label, track or value), or press the context-menu key or Shift F10 while it has focus, to open its menu at the pointer (or at the slider, from the keyboard).
- The **header** names the slider and its value, as FEATURES.md's Decided style has it: 11 px caps, the value in mono accent-soft. For example "EXPOSURE  +0.35 EV", "TEMP  5400 K", "HIGHLIGHTS  −38".
- **Items:**
  1. **Reset *Name***, showing "double-click" as its shortcut, as the handoff does. It does what double-clicking the slider does: one step labelled "Reset *Name*" (APP-10). Temp and Tint reset the white balance to As Shot, so their item reads "Reset White balance".
  2. A separator.
  3. **Copy value**: keeps the slider's value for the session, and also puts its text ("+0.35", "5400") on the system clipboard, so it can be pasted into a value field or elsewhere.
  4. **Paste value**: sets the slider to the kept value, as one step labelled with the slider's name. It is enabled only for the slider the value was copied from, on any photo.
- Choosing an item closes the menu. Esc, a click elsewhere, or a scroll of the panel closes it without acting.
- **Temp and Tint** have two scales: Kelvin and RAW tint for RAW files, a relative −100 to +100 for other files. A Temp copied on a RAW file pastes only into a RAW file's Temp, and the same for the relative scale. Pasting Temp keeps Tint, and the reverse; the white balance becomes Custom, as a drag does.
- The pasted value is clamped by the engine, as any value is (DEV-01).

## Inputs and shortcuts

| Action | Input |
|---|---|
| Open the menu | Right-click a slider; the context-menu key or Shift F10 on a focused slider |
| Move between items | ↑ ↓ (focus starts on the first item) |
| Choose | Enter, Space or a click |
| Close | Esc, a click outside |

The items act on the one slider the menu was opened on. They are not global commands, so they have no shortcuts of their own beyond reset's double-click, which the slider already has.

## States

| State | Menu |
|---|---|
| No photo, or one that does not render | The slider is disabled: no menu |
| Nothing copied, or a value copied from another slider or scale | Paste value disabled |
| A gesture in progress | The menu does not open until the gesture ends |

## Data

The copied value lives in the app for the session: the slider, its scale for Temp and Tint, and the value. Nothing is stored in the catalog.

## Edge cases

- **Copy on one photo, paste on another:** that is the main use. The paste is a step on the photo it is pasted on.
- **Paste a value equal to the current one:** no step is recorded (APP-10).
- **The system clipboard refuses the write** (no permission, no focus): the value is still kept for Paste value; nothing is shown.
- **B&W hides Vibrance and Saturation:** their menus go with them; a value copied from them stays kept.

## Acceptance criteria

Vitest and Playwright against the fake engine:

1. Right-click on a slider opens its menu with the header "NAME  value" and the four items; Esc closes it.
2. Reset sends the same reset as a double-click and makes one step.
3. Copy value, then Paste value on another photo, sets that photo's slider to the value as one step labelled with the slider's name.
4. Paste value is disabled before a copy, and for a different slider or a different Temp/Tint scale.
5. Shift F10 on a focused slider opens the menu with focus on its first item; ↓ and Enter choose an item.
6. A disabled slider opens no menu.
