# License review (M0, step 0.B)

**Status: signed off for M0 (2026-10-14).** The adopted dependencies below are checked against `cargo metadata`, the package files or the upstream sources. The dependencies cleared for the M0 spikes were read from upstream and pinned, and their full trees were checked against `deny.toml`. Later candidates (M1 onward) are listed with the license their upstream declares. Each one gets the same check, and is moved up, at the milestone that adopts it.

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
| wgpu 30.0.1 | backend (`pipeline`) | MIT OR Apache-2.0 | GPU pipeline (0.D). Built with the Vulkan, Metal and D3D12 backends only. Its tree adds Apache-2.0-only crates (spirv, codespan-reporting) |
| pollster 1.0.1 | backend (`pipeline`) | Apache-2.0 OR MIT | |
| bytemuck 1 | backend (`pipeline`) | Zlib OR Apache-2.0 OR MIT | Already in wgpu's tree |
| jpeg-encoder 0.7.1 | backend (`imgio`) | (MIT OR Apache-2.0) AND IJG | Drag-preview frames (0.E). Read from the registry metadata. The IJG license is on the FSF's list of GPL-compatible licenses. It requires that documentation shipped with binaries states that the software is based in part on the work of the Independent JPEG Group: add that line to the About box and the README when the first release is built. `deny.toml` allows `IJG` for this crate only |
| rayon 1.12 | backend (`engine`) | MIT OR Apache-2.0 | CPU parallelism, as the architecture rules prescribe. Already in the tree through jxl-oxide and rawler |
| criterion 0.8.2 | backend (`pipeline`), benchmarks only | Apache-2.0 OR MIT | Not distributed |
| png 0.17.16 | backend (`imgio`; also `golden` for golden-image storage) | MIT OR Apache-2.0 | PNG decode (M1, 2026-10-18), so now distributed. Read from the registry metadata; the version rawler's tree already used. Its tree adds fdeflate, crc32fast (MIT OR Apache-2.0) and miniz_oxide (MIT OR Zlib OR Apache-2.0) |
| half 2.7.1 | backend (`golden`, and `imgio` through tiff) | MIT OR Apache-2.0 | |
| zune-jpeg 0.5.15, zune-core 0.5.3 | backend (`imgio`) | MIT OR Apache-2.0 OR Zlib | JPEG decode (M1, 2026-10-18), the spec's choice. Already in rawler's tree, at the same version |
| tiff 0.11.3 | backend (`imgio`) | MIT | TIFF decode (M1, 2026-10-18), image-rs. Built with the deflate, lzw and jpeg features (not fax). Its tree adds weezl 0.1.12 and flate2 1.1.10 (MIT OR Apache-2.0), quick-error 2.0.1 (MIT/Apache-2.0), simd-adler32 0.3.10 (MIT), adler2 (0BSD OR MIT OR Apache-2.0), half and zune-jpeg |
| memchr 2.8.3 | backend (`imgio`) | Unlicense OR MIT | Byte search for embedded previews (M1, 2026-10-18). Already in the tree through rawler, tiff and others |
| @tauri-apps/api | frontend | Apache-2.0 OR MIT | |
| Public Sans (`@fontsource/public-sans`) | frontend, bundled | OFL-1.1 | Checked in `node_modules/@fontsource/public-sans/LICENSE`. OFL allows bundling with any software; the font may not be sold on its own |
| JetBrains Mono (`@fontsource/jetbrains-mono`) | frontend, bundled | OFL-1.1 | Same as Public Sans |

The other 400-odd transitive Rust crates use the licenses allowed in `deny.toml`. Frontend dev tooling (Vite, Svelte, ESLint, Vitest, Playwright) is not distributed with the app.

## Cleared for the M0 spikes

Signed off 2026-10-14. Each crate is added to `Cargo.toml` when its spike starts, at the version below, and then moves to the Adopted table (wgpu, pollster and criterion did with 0.D). Their combined dependency tree (about 270 crates, normal and build dependencies) was resolved and every license in it checked against `deny.toml`. The only license outside the allow list is rawler's, which has a per-crate exception.

| Dependency | Version | Use | License (read upstream) | Notes |
|---|---|---|---|---|
| rawler | 0.8.0 | RAW and DNG decode (0.C) | LGPL-2.1-only | `LICENSE` in dnglab is the LGPL-2.1 text, and `rawler/Cargo.toml` declares `LGPL-2.1`. The sources carry no "or later" header, so we treat it as 2.1 only. That is still compatible: LGPL-2.1 section 3 lets the recipient apply the GPL instead, and the FSF lists LGPL-2.1 as compatible with GPL-3.0. Pure Rust, with no `-sys` crates and no C build. Static linking into a GPL-3.0 app with full source shipped meets the LGPL. Allowed in `deny.toml` for this crate only. Vendored with patches since 2026-10-18 (`backend/vendor/README.md`): exact curve lookups instead of dithered ones, and limits checked before allocating (image buffers, TIFF entry counts, the IFD chain and SubIFDs). The modified files are `src/bits.rs`, `src/pixarray.rs` and `src/formats/tiff/{entry,ifd,reader}.rs`. Each carries the change notice LGPL-2.1 section 2 requires, and the full source ships with the repository |
| jxl-oxide | 0.12.6 | JPEG XL decode for DNG 1.7 tiles (0.C) | MIT OR Apache-2.0 | Registry metadata. Pure Rust. Pinned exactly because it parses untrusted files. `cargo deny check licenses bans` passes with it added (2026-10-15) |
| moxcms | 0.9.1 | ICC transforms (0.D, M1) | BSD-3-Clause OR Apache-2.0 | Chosen over lcms2 (2026-10-14): pure Rust, with no C build or FFI. Its one non-dev dependency, pxfm, has the same license |

## Candidates for later milestones

These licenses come from each project's upstream license file or its registry metadata, as noted. The exact version is pinned, and the tree checked, when the milestone adopts the dependency.

| Dependency | Use | License | Source | Notes |
|---|---|---|---|---|
| LibRaw | RAW fallback through FFI (0.C prototype, M1) | LGPL-2.1 or CDDL-1.0, at our choice | Upstream `README.md` | Use under LGPL-2.1. The README warns that licensing may change in future versions, so re-read it at the pinned version. A C++ build on all three OSes, like `genzo-heif`, so it would be the second crate allowed unsafe code. The only Rust bindings (libraw-rs 0.0.4, MIT OR Apache-2.0) are immature; write our own shim as `genzo-heif` does |
| lcms2 (Little CMS) | ICC transforms | MIT | Registry | Not chosen; moxcms is used instead. Kept as the fallback if moxcms falls short |
| rusqlite and SQLite | Catalog (M2) | MIT; SQLite is public domain | Registry | |
| lensfun library | Lens profiles (M3) | LGPL-3.0 | Upstream `README.md` | Or read the database only and implement the models in Rust |
| lensfun database | Lens profiles (M3) | CC BY-SA 3.0 | Upstream `README.md` | Data, not code. Shipping it needs attribution and share-alike for the data files; it does not affect the code license |
| ort | ONNX Runtime bindings (M4) | MIT OR Apache-2.0 | Registry | |
| ONNX Runtime | Inference (M4) | MIT | Upstream `LICENSE` | Downloaded binaries; check what the provider builds pull in |

## Derived code

Source code ported from other projects into Genzo's own files. Each file keeps the original copyright notice and says what was changed.

| Origin | Files | License | Notes |
|---|---|---|---|
| Adobe DNG SDK 1.4 (`dng_color_spec.cpp`, `dng_camera_profile.cpp`, `dng_temperature.cpp`, `dng_xy_coord.cpp`; Copyright 1999-2014 Adobe Systems Incorporated), the copy in gopro/gpr | `backend/crates/color/src/camera.rs` | BSD-3-Clause | Compatible with GPL-3.0. The file carries the notice; a binary release lists it with the other notices (Release obligations). `scripts/dng-sdk-probe/` links the SDK as a test tool only and is not distributed |
| darktable `data/kernels/demosaic_rcd.cl` and `src/iop/demosaicing/rcd.c` (Copyright 2010-2026 darktable developers); the RCD algorithm by Luis Sanz Rodríguez | `backend/crates/pipeline/src/gpu/shaders/rcd_*.wgsl`, `backend/crates/pipeline/tests/rcd_reference/` (test-only) | GPL-3.0-or-later (darktable); RCD itself GPL-3.0 | Compatible: Genzo is GPL-3.0. Ported to WGSL with full-resolution buffers |

## HEVC patent position (decided)

HEIC images are HEVC-coded. HEVC is covered by patent pools (Access Advance, Via LA) whose licensing terms cover decoders shipped in products. Software licenses like the LGPL do not grant patent rights.

**Decision (project owner, 2026-10-14):** ship libde265 inside Genzo on every platform (option 1 of three considered), as many open-source viewers do, and accept the patent risk. Revisit before the first public release, ideally with advice from someone who knows patent law where the project is published. The rejected options were the OS decoders (ImageIO, Windows HEVC Video Extensions, a system libheif on Linux) and an optional download.

### Cutting HEIC support

The decoder is isolated so it can be removed in one change:

1. Build without it: `genzo-engine` and `genzo-imgio` have a `heic` feature, on by default. With it off, HEIC files are still recognized and fail with "HEIC support is not included in this build". CI checks that this build compiles.
2. To remove it completely, also delete `backend/crates/heif` and its two submodules (`git submodule deinit` and `git rm`), the `genzo-heif` entries in `backend/Cargo.toml`, `scripts/check-architecture.py` and `backend/fuzz`, and the libheif and libde265 rows in this file.

## JPEG XS patent position (decided)

Nikon High Efficiency RAW (Z 6III, Z 8, Z 9, Z f in HE mode) is JPEG XS coded (ISO/IEC 21122). Read 2026-10-18 from the pool's own documents:

- **There is an active pool.** The JPEG XS patent pool, run by Vectis IP, licenses the essential patents of intoPIX and Fraunhofer IIS. Its July 2026 [patent list](https://www.jpegxspool.com/patent-list) has 101 patents, granted from 2018 on: 14 in the US, much of Europe (EP, the Unitary Patent, DE, FR, GB and others), Japan, China, Korea, Brazil, India and more. So most run into the late 2030s.
- **Decoding is covered.** In the [overview of terms](https://www.jpegxspool.com/s/JPEG-XS-PPL-Overview-of-Terms-v50-01-Apr-2025.pdf) (v5.0, April 2025):
  - an *Instance* is "a single operation which consists of either encoding, decoding, editing, or analyzing one JPEG XS Stream";
  - a *Product* is "a ready-to-use End-User solution (e.g., hardware device or software application) capable of encoding, decoding, editing, and/or analyzing one or more JPEG XS streams".
- **There is no free or open-source exemption.** Royalties start at 3.00 USD per HD instance, or 0.60 USD per product at volume, reported quarterly. Licensed products must be marked with the patent numbers.
- **A pool license does not fit GPL-3.0.** A license that charges royalties for copies would stop Genzo from being distributed under GPL-3.0 at all (section 12). So licensing is not an option for this project, and the choice is the same as for HEVC.

**Decision (project owner, 2026-10-18):**
- The JPEG XS decoder is built in M1, in its own crate, behind a `jpeg-xs` cargo feature that **release builds leave off**.
- Without it, HE files show their embedded preview and say why. Self-builders can turn it on.
- Revisit before the first public release, together with HEVC.
- The other options were shipping it on by default and accepting the risk, as with HEVC, or not building it.

## Release obligations

- [ ] Ship the LGPL-3.0 and GPL-3.0 texts and the libheif and libde265 copyright notices with every bundle (MSI or NSIS, deb, rpm, AppImage, dmg), for example in an About dialog and a `licenses/` folder.
- [ ] Ship the OFL-1.1 text with the bundled fonts.
- [ ] Ship the BSD-3-Clause notice of the Adobe DNG SDK, which `color::camera` is ported from.

## Sign-off

- [x] Each M0 candidate's license read from its upstream repository, version pinned (2026-10-14)
- [x] HEVC position decided (2026-10-14)
- [x] JPEG XS position decided: decoder behind a feature flag, off in release builds (2026-10-18)
- [x] `deny.toml` updated for each adopted candidate (rawler exception; everything else was already allowed)
- [x] ICC library chosen: moxcms (2026-10-14)

Later candidates are signed off at the milestone that adopts them, with the same three steps.
