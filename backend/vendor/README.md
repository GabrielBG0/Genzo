# Vendored crates

## rawler 0.8.0 (LGPL-2.1-only), patched

`rawler/` is [rawler](https://crates.io/crates/rawler) 0.8.0 from crates.io, from the dnglab project, with three changes: exact curve lookups, size limits before allocating, and the TIFF entry count check below. It replaces the registry crate through `[patch.crates-io]` in `backend/Cargo.toml`, `backend/fuzz/Cargo.toml` and `shell/Cargo.toml`. Only rawler's own test images (`data/testdata`, 18 MB) were left out.

### First change (2026-10-18): exact curve lookups

`src/bits.rs`, `LookupTable::dither`: it now returns the exact table value.

Upstream (inherited from rawspeed) adds a pseudo-random offset within the curve step to every lookup. Every curve-coded format goes through it: Nikon NEF, Kodak DCR, CR2 tables, compressed ARW, some packed formats and the DNG LinearizationTable. Each such sample then lands up to a few codes away from the value the file defines, with a slight dark bias.

Against LibRaw 0.22.1 on the corpus, before the patch:

| File | Samples that differ | Range |
|---|---|---|
| Nikon D850 12-bit NEF | 56 % | −3 to +2 codes |
| Nikon Z 9 lossless NEF | 75 % | −1 to 0 |
| Kodak DCS760C DCR | 100 % | −1 |

After it, all three match LibRaw on every sample (`crates/imgio/tests/cfa_corpus.rs`). The function keeps its name and signature, so every caller is covered by the one change.

### Second change (2026-10-18): size limits before allocating

`src/pixarray.rs`: `Pix2D::new`, `Pix2D::new_uninit`, `Color2D::new` and `Color2D::new_with_default` panic before allocating more than 400 million pixels, or when width × height overflows. A damaged or crafted header can claim any size, and an allocation that large aborts the process. No caller can catch that, while this panic is caught (by rawler's own `catch_unwind` and by Genzo's). Upstream has the same check commented out in its `alloc_image` macros.

`src/formats/tiff/entry.rs` (with `EndianReader::remaining` in `reader.rs`): a TIFF entry's value count, read from the file, is checked against the bytes left in the stream before the values are allocated. The `decode` fuzz target found an entry that claimed 13.7 GB. A failed entry is skipped like rawler's other invalid entries, so the rest of the file still parses. The input is `crates/imgio/tests/fixtures/dng/fuzz-rawler-tag-count-oom.bin`.

`src/formats/tiff/reader.rs` and `ifd.rs` set three more limits, each found by the fuzz target growing memory past 4 GB (`fuzz-rawler-ifd-memory.bin`). Legitimate files are far inside all three:
- The IFD chain stops at an offset seen before, or after 64 IFDs. A cycle looped forever.
- At most 16 SubIFDs are parsed per tag.
- The entries of one IFD may copy at most the file's size in values.

In both workspaces that build release binaries, `[profile.release.package.rawler] overflow-checks = true` turns rawler's integer overflows into caught panics, as under fuzzing, instead of wrapped sizes or indices.

### Known rawler 0.8 problems worked around outside the vendored code

- **Lossless JPEG ignores restart intervals (DRI).** Samples past the first restart are wrong, with no error. `crates/imgio/src/ljpeg.rs` splits such streams for the linear DNG reader. `dng::check_cfa_for_rawler` reports CFA DNGs with restarts as unsupported.
- **The DNG CFA pattern is shifted by the ActiveArea offset in the wrong direction.** It computes `pattern(row + top, col + left)`, not `pattern(row − top, col − left)`. The two agree only for patterns of period 2. Genzo reads the DNG pattern itself (`dng::cfa_pattern`).

### Upgrading rawler

Diff the new release against `rawler/`, carry both changes over (drop one if upstream gains an exact lookup or a size check), update the version in `backend/Cargo.toml`, and rerun `cfa_corpus` and the golden tests. LGPL-2.1 section 2 requires the change notice in the modified file to stay.
