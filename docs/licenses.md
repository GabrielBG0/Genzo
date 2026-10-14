# License review (M0, step 0.B)

**Status: draft.** The adopted dependencies below are checked against `cargo metadata` and the package files. The candidates for the M0 spikes are **not adopted yet**. Their licenses are from memory and each one is marked *to confirm* until someone reads the upstream license file and signs it off. `deny.toml` only allows what is in the first table.

Genzo is GPL-3.0-only. A dependency is acceptable when its license is compatible with distributing the combined work under GPL-3.0.

## Adopted

| Dependency | Where | License | Notes |
|---|---|---|---|
| serde, serde_json | backend | MIT OR Apache-2.0 | |
| ts-rs | backend (`model`) | MIT | Contract generator |
| thiserror | backend | MIT OR Apache-2.0 | |
| tauri, tauri-build, wry, tao | shell | Apache-2.0 OR MIT | |
| cssparser, selectors, dtoa-short, option-ext | shell (through Tauri) | MPL-2.0 | File-level copyleft, compatible with GPL-3.0 |
| WebKitGTK 4.1 | Linux system library | LGPL-2.1 | Linked dynamically from the system, not bundled |
| @tauri-apps/api | frontend | Apache-2.0 OR MIT | |
| Public Sans (`@fontsource/public-sans`) | frontend, bundled | OFL-1.1 | Checked in `node_modules/@fontsource/public-sans/LICENSE`. OFL allows bundling with any software; the font may not be sold on its own |
| JetBrains Mono (`@fontsource/jetbrains-mono`) | frontend, bundled | OFL-1.1 | Same as Public Sans |

The other 400-odd transitive Rust crates use the licenses allowed in `deny.toml`. Frontend dev tooling (Vite, Svelte, ESLint, Vitest, Playwright) is not distributed with the app.

## Candidates for the spikes (to confirm before adoption)

| Dependency | Use | License (to confirm) | Question to settle |
|---|---|---|---|
| rawler | RAW and DNG decode (0.C) | LGPL-2.1 | Compatible with GPL-3.0 (LGPL-2.1 allows relicensing to GPL). Static linking is fine for a GPL app. Confirm the exact version and whether "or later" applies |
| LibRaw | RAW fallback through FFI | LGPL-2.1 or CDDL-1.0 | Use under LGPL-2.1. Confirm the build links it in a way we are comfortable shipping on all three OSes |
| libheif | HEIC decode | LGPL-3.0 | Compatible with GPL-3.0. Check the plugin layout (libde265 as a plugin or built in) |
| libde265 | HEVC decode inside libheif | LGPL-3.0 | See the HEVC patent position below |
| lcms2 (Little CMS) | ICC transforms | MIT | Alternative: moxcms, a pure-Rust option (license to confirm, believed BSD-3-Clause or Apache-2.0) |
| wgpu | GPU pipeline (0.D) | MIT OR Apache-2.0 | |
| ort | ONNX Runtime bindings (M4) | MIT OR Apache-2.0 | |
| ONNX Runtime | Inference (M4) | MIT | Downloaded binaries; check what the provider builds pull in |
| lensfun library | Lens profiles (M3) | LGPL-3.0 | Or read the database only and implement the models in Rust |
| lensfun database | Lens profiles (M3) | CC BY-SA 3.0 | Data, not code. Shipping it needs attribution and share-alike for the data files; it does not affect the code license |
| zune-jpeg | JPEG decode | MIT OR Apache-2.0 OR Zlib | |
| jpeg-encoder or mozjpeg | JPEG encode (M1) | jpeg-encoder: MIT OR Apache-2.0. mozjpeg: IJG, BSD-3-Clause and Zlib | |
| rusqlite and SQLite | Catalog (M2) | MIT and public domain | |

## HEVC patent position (open)

HEIC images are HEVC-coded. HEVC is covered by patent pools (Access Advance, Via LA) whose licensing terms cover decoders shipped in products. Software licenses like LGPL do not grant patent rights. Options, to decide with the project owner:

1. Ship libde265 and accept the patent risk, as many open-source viewers do.
2. Use the OS decoder where one exists: ImageIO on macOS, and on Windows the HEVC Video Extensions the user installs. On Linux, use libheif if the user has it installed.
3. Ship HEIC support as an optional download, kept out of the installer.

This is a legal and distribution decision, not a technical one. It must be settled before HEIC support ships in M1.

## Sign-off

- [ ] Each candidate's license read from its upstream repository, version pinned
- [ ] HEVC position decided
- [ ] `deny.toml` updated for each adopted candidate
