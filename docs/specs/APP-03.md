# APP-03 · Quick actions in the top bar

One-click buttons next to the app menu. This spec covers what M1 builds (plan, M1 frontend item 3: "Quick actions with fixed Develop defaults: Undo, Redo, Before/After, Reset all and Export. Customization comes later").

Sources: FEATURES.md (APP-03, with its Decided look), the handoff's Top bar component and *Buttons and toolbar groups*, the plan's *Control polish* decision (2026-11-05). Where this spec and the technical spec disagree, the technical spec wins and this file is corrected.

**Out of scope for M1:** Copy settings and Paste settings (M3, with settings sync), Library's defaults (M2), customizing, reordering, suggestions and "Pin to top bar" (Later).

## Behavior

- **Groups**, separated by dividers: Undo, Redo · Before/After, Reset all · Export.
- **Each button is a registered command:** its icon, its label and shortcut in the tooltip ("Undo Exposure  Ctrl Z": APP-10 names the step), disabled when the command is.
- **Flat, not lens:** 28 px tall, radius 6, transparent, rgba(255,255,255,.07) groups, .11 on hover. A toggle that is on (Before/After while the before shows, WF-02) has accent-soft text on rgba(255,255,255,.13).
- **Export** shows the running export's progress as a ring, and a click while one runs opens the dialog with *Cancel export* (OUT-01).

## Inputs and shortcuts

| Button | Command | Shortcut |
|---|---|---|
| Undo | `edit.undo` | Ctrl Z |
| Redo | `edit.redo` | Ctrl Shift Z |
| Before/After | `develop.beforeAfter` | \ |
| Reset all | `develop.resetAll` | Ctrl Shift R |
| Export | `module.export` | Ctrl Shift E |

## States

| State | Undo, Redo | Before/After, Reset all | Export |
|---|---|---|---|
| No photo, or an unsupported one | disabled | disabled | disabled |
| A photo that renders | by its history | enabled; Before/After *on* while the before shows | enabled; the ring while an export runs |

## Data

Nothing is stored.

## Acceptance criteria

1. The buttons run their commands and their tooltips name the shortcut (Playwright: APP-10, WF-02, OUT-01's tests).
2. No quick action has the lens (Playwright).
