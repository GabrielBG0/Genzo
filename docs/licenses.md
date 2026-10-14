# License review (M0, step 0.B)

**Status: draft.** The adopted dependencies below are checked against `cargo metadata`, the package files or the upstream sources. The candidates in the second table are **not adopted yet**. Their licenses are from memory and each one is marked *to confirm* until someone reads the upstream license file and signs it off.

cargo-deny (`deny.toml`) checks Rust crates only. The C libraries built from source (libheif, libde265) are invisible to it, so this file is what guards them, and their security releases must be tracked by hand. Watch [libheif releases](https://github.com/strukturag/libheif/releases) and [libde265 releases](https://github.com/strukturag/libde265/releases).

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
| libheif 1.23.6 | backend (`genzo-heif`), bundled | LGPL-3.0-or-later | Read from `COPYING` and the source headers in the pinned submodule. Built from source and linked statically, with only the libde265 decoder enabled. Compatible with GPL-3.0: the combined work is distributed under GPL-3.0 with full source, which meets the LGPL's relinking terms |
| libde265 1.1.3 | backend (`genzo-heif`), bundled | LGPL-3.0-or-later | Same as libheif. HEVC decoder; see the HEVC position below |
| cc, cmake (build tools) | backend build only | MIT OR Apache-2.0 | Not distributed |
| @tauri-apps/api | frontend | Apache-2.0 OR MIT | |
| Public Sans (`@fontsource/public-sans`) | frontend, bundled | OFL-1.1 | Checked in `node_modules/@fontsource/public-sans/LICENSE`. OFL allows bundling with any software; the font may not be sold on its own |
| JetBrains Mono (`@fontsource/jetbrains-mono`) | frontend, bundled | OFL-1.1 | Same as Public Sans |

The other 400-odd transitive Rust crates use the licenses allowed in `deny.toml`. Frontend dev tooling (Vite, Svelte, ESLint, Vitest, Playwright) is not distributed with the app.

## Candidates for the spikes (to confirm before adoption)

| Dependency | Use | License (to confirm) | Question to settle |
|---|---|---|---|
| rawler | RAW and DNG decode (0.C) | LGPL-2.1 | Compatible with GPL-3.0 (LGPL-2.1 allows relicensing to GPL). Static linking is fine for a GPL app. Confirm the exact version and whether "or later" applies |
| LibRaw | RAW fallback through FFI | LGPL-2.1 or CDDL-1.0 | Use under LGPL-2.1. Confirm the build links it in a way we are comfortable shipping on all three OSes |
| lcms2 (Little CMS) | ICC transforms | MIT | Alternative: moxcms, a pure-Rust option (license to confirm, believed BSD-3-Clause or Apache-2.0) |
| wgpu | GPU pipeline (0.D) | MIT OR Apache-2.0 | |
| ort | ONNX Runtime bindings (M4) | MIT OR Apache-2.0 | |
| ONNX Runtime | Inference (M4) | MIT | Downloaded binaries; check what the provider builds pull in |
| lensfun library | Lens profiles (M3) | LGPL-3.0 | Or read the database only and implement the models in Rust |
| lensfun database | Lens profiles (M3) | CC BY-SA 3.0 | Data, not code. Shipping it needs attribution and share-alike for the data files; it does not affect the code license |
| zune-jpeg | JPEG decode | MIT OR Apache-2.0 OR Zlib | |
| jpeg-encoder or mozjpeg | JPEG encode (M1) | jpeg-encoder: MIT OR Apache-2.0. mozjpeg: IJG, BSD-3-Clause and Zlib | |
| rusqlite and SQLite | Catalog (M2) | MIT and public domain | |

## HEVC patent position (decided)

HEIC images are HEVC-coded. HEVC is covered by patent pools (Access Advance, Via LA) whose licensing terms cover decoders shipped in products. Software licenses like the LGPL do not grant patent rights.

**Decision (project owner, 2026-10-14):** ship libde265 inside Genzo on every platform (option 1 of three considered), as many open-source viewers do, and accept the patent risk. Revisit before the first public release, ideally with advice from someone who knows patent law where the project is published. The rejected options were the OS decoders (ImageIO, Windows HEVC Video Extensions, a system libheif on Linux) and an optional download.

### Cutting HEIC support

The decoder is isolated so it can be removed in one change:

1. Build without it: `genzo-engine` and `genzo-imgio` have a `heic` feature, on by default. With it off, HEIC files are still recognized and fail with "HEIC support is not included in this build". CI checks that this build compiles.
2. To remove it completely, also delete `backend/crates/heif` and its two submodules (`git submodule deinit` and `git rm`), the `genzo-heif` entries in `backend/Cargo.toml`, `scripts/check-architecture.py` and `backend/fuzz`, and the libheif and libde265 rows in this file.

## Release obligations

- [ ] Ship the LGPL-3.0 and GPL-3.0 texts and the libheif and libde265 copyright notices with every bundle (MSI or NSIS, deb, rpm, AppImage, dmg), for example in an About dialog and a `licenses/` folder.
- [ ] Ship the OFL-1.1 text with the bundled fonts.

## Sign-off

- [ ] Each candidate's license read from its upstream repository, version pinned
- [x] HEVC position decided (2026-10-14)
- [ ] `deny.toml` updated for each adopted candidate
