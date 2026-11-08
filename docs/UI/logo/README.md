# Genzo logo

The mark is one frame of 35 mm film, with 現像 (genzō, "photo development") and the frame number 12A printed in the edge, the way film carries its brand along the margin. Two versions:

- **color**: the image window in Genzo blue (#5b9dff, the UI accent, 90 % opacity).
- **bw**: the window left dark. For monochrome places, print, and anywhere the blue would clash.

All text is converted to outlines, so the SVGs need no fonts. The kanji come from Zen Kaku Gothic New Black and the frame number from JetBrains Mono Medium, both under the SIL Open Font License.

## Small sizes

At 32 px and below, the edge text cannot be read, so these sizes use the **small** mark: the same frame, symmetrical, without the edge marking. The PNGs at 16, 24 and 32 px, the .ico files and the favicons already use it.

## Files

| File | Use |
|---|---|
| `svg/genzo-icon-{color,bw}.svg` | App icon master, full tile |
| `svg/genzo-icon-{color,bw}-small.svg` | App icon for 32 px and below |
| `svg/genzo-icon-{color,bw}-macos.svg` | App icon on Apple's grid (tile 824 of 1024) |
| `svg/genzo-mark-{color,bw}-on-{dark,light}.svg` | The mark without the tile, for docs, the website, the splash and about screens |
| `svg/genzo-mark-{color,bw}-small-on-{dark,light}.svg` | The small mark without the tile |
| `svg/genzo-glyph-currentcolor.svg` | Top bar glyph; takes the text color (`currentColor`) |
| `png/{color,bw}/icon-N.png` | 16 to 1024 px |
| `png/{color,bw}/icon-1024-macos-source.png` | Source for `tauri icon` |
| `png/{color,bw}/genzo-*.ico` | Windows icon, 16 to 256 px |
| `png/{color,bw}/genzo-*.icns` | macOS icon |
| `png/{color,bw}/favicon-*.ico` | 16, 32, 48 px |

## In the repo

1. In `frontend/`: `pnpm tauri icon ../docs/UI/logo/png/color/icon-1024-macos-source.png -o ../shell/icons` regenerates every file in `shell/icons/`.
2. Then replace `shell/icons/32x32.png` with `png/color/icon-32.png` and `shell/icons/icon.ico` with `png/color/genzo-color.ico`, so the small sizes use the small mark instead of a scaled-down large one.
3. The top bar's logo button uses `genzo-glyph-currentcolor.svg`, drawn as one path in `frontend/src/lib/components/Logo.svelte`.
