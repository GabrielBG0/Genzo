# APP-01 · Module switcher

The pill tabs at the right of the top bar. This spec covers what M1 builds (plan, M1 frontend item 1: "pill tabs. Develop is active (D). Library is disabled until M2. The Export tab opens the basic export view. Map and Print tabs appear only when those modules ship").

Sources: FEATURES.md (APP-01, with its Decided motion), the handoff's Top bar component ("module tabs (one sliding indicator)") and *Selection indicators*, the plan's *Control polish* decision (2026-11-05). Where this spec and the technical spec disagree, the technical spec wins and this file is corrected.

**Out of scope for M1:** Library (M2: the tab shows, disabled, and G does nothing), Map and Print (Later: no tab until they ship), remembering module state per photo or per module (FEATURES.md's open question, M2 with Library).

## Behavior

- **Tabs:** Library, Develop, Export, left to right, each a registered command (`module.library` G, `module.develop` D, `module.export` Ctrl Shift E) whose tooltip names its shortcut.
- **Develop** is the module in M1. Its tab shows the indicator and `aria-current="page"`.
- **Library** is disabled until M2, its text dim.
- **Export** opens the export dialog (OUT-01) for a photo that renders, and the module stays Develop. Without one the tab does nothing, as Ctrl Shift E.
- **One indicator:** a single lens sits under the current tab and slides to a new one: left and width over 180 ms, cubic-bezier(.22, 1, .36, 1), with no bounce or overshoot. While a tab is pressed it squeezes to scale (.95, .9). Its first placement does not slide. With reduced motion it moves at once.

The same indicator serves every segmented group and tab bar (the handoff's Segmented control: "one sliding indicator per group, on the Lens surface"): Color / B&W in the Basic panel, the histogram's tool strip, and the Library toolbar's views. The menus' highlight is the same lens, faster (APP-02).

## Inputs and shortcuts

| Input | Effect |
|---|---|
| Click a tab | Its module (Export: the export dialog) |
| D | Develop |
| G | Nothing in M1 (Library is disabled) |
| Ctrl Shift E | The export dialog |

## States

| State | Library | Develop | Export |
|---|---|---|---|
| M1 | disabled | current, the indicator | opens the dialog when a photo renders |

## Data

Nothing is stored.

## Edge cases

- **A dialog is open:** D does nothing until it closes.
- **The window narrows:** the indicator follows its tab (it is re-placed when the bar's size changes).

## Acceptance criteria

1. One indicator in the tab bar, over Develop; Library disabled; D keeps Develop (Playwright).
2. In Color / B&W, V moves the indicator to B&W (Playwright).
3. The Export tab opens the export dialog (Playwright, OUT-01's test).
