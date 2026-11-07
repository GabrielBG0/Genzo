# Genzo · brand

This file records the logo decisions of 2026-11-05. The [UI handoff](fixer-ui-handoff.html) defines the look of the app itself.

## The mark

The mark is one 35 mm film frame: a 3:2 image window with 8 perforations per side. 現像 and the frame number 12A are printed in the bottom edge, like film edge markings.

現像 (genzō) means photo development, the origin of the name.

The type is outlined, so the mark needs no font. The kanji is Zen Kaku Gothic New Black and the number is JetBrains Mono Medium. Both fonts are SIL OFL, as the [license review](../licenses.md) records.

Explored and rejected, for the record: kanji seal and shop-sign marks (unreadable at 16 px), abstract geometric marks, film can, instant print.

## Versions

| Version | Image window | Use it |
|---|---|---|
| Color | the accent `#5b9dff` at 90 % | by default, wherever the mark stands for the app |
| B&W | dark | where the accent is not available or would compete with what is around it |

## Small sizes

Sizes here are the mark's width, its longer side. At 32 px wide and below, use the small mark: the same frame, symmetrical, with no edge marking.

| Mark | Minimum width |
|---|---|
| Full mark, with edge marking | 48 px |
| Small mark | 16 px |

Edge text is not allowed below 48 px, so use the small mark from 33 to 47 px wide too.

## Clear space

Keep at least the height of the perforation band clear around the mark.

## Do and don't

- Do use the color version by default and the small mark at small sizes.
- Don't recolor the frame.
- Don't put blue on the B&W version.
- Don't use edge text below 48 px.

The top bar's logo button uses `logo/svg/genzo-glyph-currentcolor.svg`, which takes the color of the text around it by design.

## Files

The files go in `logo/`, and its README describes them.

| File | Use |
|---|---|
| `logo/README.md` | what each file is |
| `logo/svg/genzo-glyph-currentcolor.svg` | the top bar's logo button |

> The `logo/` folder is not in the repository yet. When it lands, link these paths and complete this table from its README.
