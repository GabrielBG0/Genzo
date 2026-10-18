# Vendored crates

## rawler 0.8.0 (LGPL-2.1-only), patched

`rawler/` is [rawler](https://crates.io/crates/rawler) 0.8.0 from crates.io, from the dnglab project, with one change. It replaces the registry crate through `[patch.crates-io]` in `backend/Cargo.toml`, `backend/fuzz/Cargo.toml` and `shell/Cargo.toml`. Only rawler's own test images (`data/testdata`, 18 MB) were left out.

### The change (2026-10-18)

`src/bits.rs`, `LookupTable::dither`: it now returns the exact table value.

Upstream (inherited from rawspeed) adds a pseudo-random offset within the curve step to every lookup. Every curve-coded format goes through it: Nikon NEF, Kodak DCR, CR2 tables, compressed ARW, some packed formats and the DNG LinearizationTable. Each such sample then lands up to a few codes away from the value the file defines, with a slight dark bias.

Against LibRaw 0.22.1 on the corpus, before the patch:

| File | Samples that differ | Range |
|---|---|---|
| Nikon D850 12-bit NEF | 56 % | −3 to +2 codes |
| Nikon Z 9 lossless NEF | 75 % | −1 to 0 |
| Kodak DCS760C DCR | 100 % | −1 |

After it, all three match LibRaw on every sample (`crates/imgio/tests/cfa_corpus.rs`). The function keeps its name and signature, so every caller is covered by the one change.

### Known rawler 0.8 problems worked around outside the vendored code

- **Lossless JPEG ignores restart intervals (DRI).** Samples past the first restart are wrong, with no error. `crates/imgio/src/ljpeg.rs` splits such streams for the linear DNG reader. `dng::check_cfa_for_rawler` reports CFA DNGs with restarts as unsupported.
- **The DNG CFA pattern is shifted by the ActiveArea offset in the wrong direction.** It computes `pattern(row + top, col + left)`, not `pattern(row − top, col − left)`. The two agree only for patterns of period 2. Genzo reads the DNG pattern itself (`dng::cfa_pattern`).

### Upgrading rawler

Diff the new release against `rawler/`, carry the `dither` change over (or drop it if upstream gains an exact lookup), update the version in `backend/Cargo.toml`, and rerun `cfa_corpus` and the golden tests. LGPL-2.1 section 2 requires the change notice in the modified file to stay.
