# APP-08 · Panel visibility

Showing and hiding the side panels and the filmstrip from the keyboard, and solo mode in a side panel. This spec covers what M1 builds (plan, M1 frontend item 12: "Tab, Shift Tab, F6, F7, F8, Alt-click for solo mode").

Sources: FEATURES.md (APP-08), the shortcut registry's View commands (`view.sidePanels`, `view.allPanels`, `view.filmstrip`, `view.leftPanel`, `view.rightPanel`), CLAUDE.md's frontend rules (edited state always visible, every command in the registry). Where this spec and the technical spec disagree, the technical spec wins and this file is corrected.

**Decided** (owner, 2026-11-08):
- **Tab and Shift Tab act on the panels only when no control has focus.** With a slider, button, field or menu item focused, they move focus as usual, so keyboard navigation keeps working.
- **"Hide all" hides the side panels and the filmstrip.** The top bar (app menu, quick actions, modules) and the status bar stay, so commands and the edited state stay in view.

T hides and shows the photo toolbar (`view.toolbar`, the viewport spec `DEV-01-viewport.md`), which came after this spec.

**Out of scope for M1:** resizable side panels and their widths (FEATURES.md's open question), screen modes and Lights Out (APP-09), remembering the layout across launches (it needs the settings store).

## Behavior

### Showing and hiding

- **Tab:** if either side panel shows, both hide; if both are hidden, both show.
- **Shift Tab:** if the left panel, the right panel or the filmstrip shows, all three hide; if all three are hidden, all three show.
- **F7** toggles the left panel, **F8** the right panel, **F6** the filmstrip.
- A hidden panel takes no space: the photo grows into it, and the engine renders at the new size, as for any window resize (DEV-01, `SetView`).
- A hidden panel keeps its state (which sections are open, scroll position), and shows again as it was.
- The layout is kept for the session, across photos and modules.
- The commands' labels follow the state: "Hide left panel" when it shows, "Show left panel" when it is hidden. The app menu (APP-02) and the shortcut sheet read them from the registry.

### When Tab is a panel command

- Tab and Shift Tab hide or show panels when focus is on no control: on the page itself, or on the photo (after clicking it).
- When a control has focus (a slider, a button, a select, a text field, a menu item), Tab and Shift Tab move focus, as in any app. Clicking the photo or an empty area takes focus off the control, and Tab goes back to the panels.
- F6, F7 and F8 work whatever has focus, except in a text field.

### Solo mode

- **Alt-click a panel's header** in a side panel turns solo mode on or off for that side. In solo mode, opening a panel closes the side's other panels, so only one is open at a time. Turning solo on keeps the clicked panel open and closes the others.
- **Ctrl-click a header** (Cmd-click on macOS) opens all of the side's panels if the clicked one is closed, and closes them all if it is open.
- In M1 the right side has one collapsible panel (Basic) and the left side none, so solo mode shows its effect as more panels arrive (M3). Its state is kept per side.
- A header in solo mode shows it: its chevron turns accent-soft, with a tooltip naming solo mode.

## Inputs and shortcuts

| Command | Shortcut | Registry id |
|---|---|---|
| Hide side panels / Show side panels | Tab (no control focused) | `view.sidePanels` |
| Hide all panels / Show all panels | Shift Tab (no control focused) | `view.allPanels` |
| Hide filmstrip / Show filmstrip | F6 | `view.filmstrip` |
| Hide left panel / Show left panel | F7 | `view.leftPanel` |
| Hide right panel / Show right panel | F8 | `view.rightPanel` |
| Solo mode for a side | Alt-click a panel header | — |
| Open or close all of a side's panels | Ctrl-click (macOS: Cmd-click) a panel header | — |

## States

| State | What shows |
|---|---|
| Default | Both side panels and the filmstrip |
| Side panels hidden | The photo between the top bar and the filmstrip, full width |
| All hidden | The photo between the top bar and the status bar |
| A dialog open (Export) | The panel commands do nothing, as every command under a dialog |

## Data

Nothing is stored. The layout and the solo flags live in the app for the session.

## Edge cases

- **The right panel hidden while the white balance picker is on:** the picker stays on (W and Esc still work).
- **The filmstrip hidden:** ← and → still switch photos (LIB-09).
- **Tab right after a slider drag:** the slider keeps focus, so Tab moves focus. Click the photo first to hide panels.
- **A very narrow window:** hiding panels gives the photo the room; nothing else changes.

## Acceptance criteria

Vitest and Playwright against the fake engine:

1. Tab with nothing focused hides both side panels, and Tab again shows them; the photo's size follows (a new `setView`).
2. Tab with a slider focused moves focus and leaves the panels as they are.
3. Shift Tab hides the side panels and the filmstrip, and Shift Tab again shows them; the top bar and status bar stay.
4. F6, F7 and F8 toggle the filmstrip, the left and the right panel, even with a slider focused.
5. Labels: `view.leftPanel` reads "Hide left panel" when it shows and "Show left panel" when it is hidden.
6. A hidden right panel shows again with its sections as they were.
7. Solo mode (unit test of the layout state, with several panels on a side): Alt-click turns it on and leaves only the clicked panel open; opening another closes the first; Ctrl-click opens or closes all.
