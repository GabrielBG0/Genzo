# APP-02 · App menu behind the logo (M1 part)

The compact app menu that replaces a menu bar. This spec covers what M1 builds (plan, M1 frontend item 2: "Start with the File, Edit, Photo, Develop, View and Help items that exist in M1. It grows every milestone").

Sources: FEATURES.md (APP-02 and its Decided menu style), the handoff's screen M2 (compact app menu, with its View submenu open), the shortcut registry, APP-08 (the View commands), APP-10, WF-02, DEV-02, DEV-03, OUT-01, LIB-09. Where this spec and the technical spec disagree, the technical spec wins and this file is corrected.

**Out of scope for M1**, with their milestone: Import, Open recent catalog and Catalog settings (M2, with the catalog), the Library and Window submenus (M2), Copy and Paste settings (M2), Preferences (with the settings store), Keyboard shortcuts (APP-05), Customize quick actions (APP-03's customization), and on macOS mirroring the menu in the system menu bar (FEATURES.md's open question). Each item arrives with its feature; the menu shows only commands that work.

## Behavior

### Opening and closing

- **Click the logo** (the Genzo button at the top left), or **press and release Alt** with nothing else pressed or clicked meanwhile, to open the menu below the logo.
- Alt counts only on its own: Alt held while dragging a slider (DEV-03's clipping preview) or clicking a panel header (APP-08's solo mode) does not open the menu.
- It closes on Esc, Alt again, a click outside, a chosen item, or the logo again.
- While it is open, keys go to the menu; no other command runs.

### Structure

The menu has a header naming the open folder ("No folder open" before one), then:

1. **Open folder…** (Ctrl O) and **Export…** (Ctrl Shift E), the two most used.
2. A separator.
3. **File ›**, **Edit ›**, **Photo ›**, **Develop ›**, **View ›**, **Help ›**, each opening a submenu to its right.
4. A separator.
5. **Quit** (Ctrl Q).

| Submenu | Items (shortcut) |
|---|---|
| File | Open folder… (Ctrl O) · Export… (Ctrl Shift E) · Cancel export (only while an export runs) |
| Edit | Undo *step* (Ctrl Z) · Redo *step* (Ctrl Shift Z) |
| Photo | Previous photo (←) · Next photo (→) |
| Develop | Before / after (\\) · Reset all (Ctrl Shift R) · separator · Color / B&W (V) · White balance picker (W) · Show clipping (J) |
| View | ✓ Side panels (Tab) · ✓ All panels (Shift Tab) · ✓ Filmstrip (F6) · ✓ Left panel (F7) · ✓ Right panel (F8) |
| Help | About Genzo |

- Every item is a registered command: its label, shortcut and enabled state come from the registry, so they read the same as in tooltips. A disabled command shows dimmed.
- The View items show a check when the part shows, as in the handoff, and read as the part's name ("Side panels"), not "Hide side panels". Before / after and Show clipping show a check when on.
- Undo and Redo name their step ("Undo Exposure").
- Shortcuts show in the platform's form: "Ctrl Shift E" on Windows and Linux, "⌘⇧E" on macOS.

### Quit

- **Quit** (Ctrl Q, ⌘Q on macOS) closes the window, so it asks first during an export, as closing the window does (OUT-01).

### About Genzo

- A small dialog: "Genzo", the app version, the engine contract version, the GPU adapter in use (and the software-adapter notice when it applies), and "Free software under the GNU GPL v3". Esc, Enter or Close dismisses it.

### Keyboard

| Key | In the menu |
|---|---|
| ↓ ↑ | The next or previous enabled item |
| → or Enter on a submenu item | Opens it, focus on its first item |
| ← or Esc in a submenu | Closes it, focus back on its item |
| Enter or Space | Chooses the item |
| Esc or Alt | Closes the menu |

The first item has focus when the menu opens. Pointing at a submenu item opens it too.

## Inputs and shortcuts

| Command | Shortcut | Registry id |
|---|---|---|
| App menu | Alt (alone) | `app.menu` |
| Quit | Ctrl Q (macOS: ⌘Q) | `app.quit` |
| Reset all | Ctrl Shift R | `develop.resetAll` (bound here; the quick action had no action until now) |
| About Genzo | — | `help.about` |

## States

| State | Menu |
|---|---|
| No folder open | Header "No folder open"; Open folder enabled; Export, Edit, Photo and Develop items disabled |
| A photo that does not render | Its edit items disabled, as their commands are |
| An export running | File › Cancel export shows |
| The export dialog open | The menu does not open (the dialog takes the keys) |

## Data

Nothing is stored.

## Edge cases

- **Alt then a letter** (Alt F, say): the letter cancels the menu's opening. M1 has no Alt shortcuts.
- **Alt pressed, then the window loses focus:** nothing opens.
- **A command that becomes disabled while the menu is open** (an export ends): the menu follows it.
- **Quit in the browser build** (the fake engine): it closes the fake's window, which tests observe; the page stays.

## Acceptance criteria

Vitest and Playwright against the fake engine:

1. The logo and Alt alone open the menu, with focus on Open folder; Esc, Alt and a click outside close it.
2. Alt held during a slider drag, and Alt-click on a panel header, do not open it.
3. Each submenu lists its items with the registry's labels and shortcuts; disabled commands show disabled.
4. → opens a submenu with focus on its first item, ← returns; Enter runs an item and closes the menu.
5. View's checks follow the layout; choosing Filmstrip hides it.
6. Edit › Undo names the step and undoes it.
7. Quit closes the window, after the question during an export.
8. About Genzo shows the contract version and the adapter, and Esc closes it.
