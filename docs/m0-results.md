# M0 spike results

Measured results of the M0 feasibility spikes, against the exit gate in [development-plan.md](development-plan.md#m0-feasibility-spikes). Each section lists what was measured, on what, and what is still open.

## 0.D GPU pipeline

**Status: done on Windows (hardware and WARP). lavapipe runs in CI (`gpu` job); its first result is pending. macOS (M1 Max, base M1) and Linux (RTX 3050 laptop) are measured with the bench kit, see 0.E.**

### What was built

`genzo-pipeline::gpu`, offscreen wgpu 30 with no surface and no unsafe code:

- **Render thread** (`gpu::thread`). One thread owns the device, and callers send it requests over a channel. Renders are coalesced latest-wins: when several are queued, only the newest runs.
- **Adapter selection** (`gpu::Gpu::open`). A hardware adapter is preferred. Genzo falls back to the software adapter (WARP, lavapipe or llvmpipe) when there is no hardware adapter, when the driver is on the blocklist, or when the startup self-test fails. `GENZO_ADAPTER=software` forces the software adapter. The self-test renders flat gray and checks every pixel. It runs inside wgpu error scopes, so a shader that fails to compile becomes a fallback reason instead of a crash. The driver blocklist exists and is empty.
- **Minimal raw path** (`gpu::spike`), as WGSL compute passes on RGBA16F textures:
  1. Linearize and white balance.
  2. RCD demosaic, in seven passes.
  3. The mip pyramid.
  4. Resample to the viewport and apply the camera matrix.
  5. Exposure and the output transform to 8-bit sRGB or Display P3.
- **Caching.** Each step's result is cached, and a render re-runs from the earliest step whose inputs changed:
  - An exposure change re-runs step 5 only.
  - A resized viewport re-runs from step 4.
  - The image itself re-runs everything.
  - **While a white-balance slider is dragged** (`interactive`), the new white balance is applied at step 4, as a ratio to the white balance the cached demosaic was made with. The settled render at the end of the drag re-runs the demosaic. This is the M1 plan's "white balance applied after the cached demosaic while dragging".
- **Contract 0.2.0:** `loadSpikeImage` (a synthetic Bayer scene for now), `renderSpike` (frames as binary tiles), and `gpuMs` on `renderFinished`.
- **cli:** `genzo-cli bench gpu [W H N]`.

### Working space

The plan asks for a working space that "fully contains Display P3". **Linear Rec. 2020 does not:** Display P3's red primary is just outside it, at −0.0012 in Rec. 2020's blue (checked in `genzo-color`, test `rec2020_does_not_contain_display_p3`). The working space is therefore linear **ACES AP1 primaries with a D65 white**. It contains both Display P3 and Rec. 2020, and keeps the D65 white of the display spaces, so no chromatic adaptation is needed. Recorded in the plan's Decided table.

### RCD port

The WGSL RCD is a port of darktable's `demosaic_rcd.cl` (GPL-3.0-or-later). The test `rcd_matches_the_darktable_reference` checks it against a literal CPU port of darktable's `rcd.c`, which is test-only (`crates/pipeline/tests/rcd_reference`). Measured on a 768 × 512 synthetic scene, 8-bit output:

| Region | GPU vs `rcd.c`, mean ΔE2000 | Max ΔE2000 | `rcd.c` vs the true scene, mean ΔE2000 |
|---|---|---|---|
| Gradients | 0.056 (RTX 3060), 0.028 (WARP) | 0.91, 1.05 | 0.18 |
| Flat patches with hard edges | 0.003 | 0.97 | 0.32 |
| Fine anti-diagonal lines | 0.048, 0.001 | 0.70 | **18.6** |

- **The one deliberate change.** The port evaluates the P/Q diagonal high-pass at every pixel. darktable keeps it in half-width buffers, which shifts one of the three terms per pixel by a column. The two variants differ by ΔE 0.004 on average.
- **Tolerance:** max ΔE2000 below 2, and at most 0.01 percent of pixels above 1. That is one 8-bit step after f16 storage.
- **Fine lines.** The lines band is a Bayer worst case: each anti-diagonal holds only green, or only red and blue samples. RCD itself renders it green-tinted, ΔE 18.6 from the truth in darktable's own code. It is therefore checked against the reference, not against the truth.
- **Against the true scene** (`reproduces_the_scene`): smooth regions measure 51 dB PSNR with mean ΔE 0.05. With every patch edge included, 41 dB with mean ΔE 0.13.

### Golden-image tests: a deviation to confirm

CLAUDE.md asks for golden-image tests, with ΔE2000 and PSNR tolerances and lavapipe as the reference device. **No golden images are stored yet.** The tests compare instead against:

- the synthetic scene's ground truth (PSNR and ΔE2000)
- the literal CPU port of darktable's `rcd.c` (ΔE2000)

Both hold on any adapter, and neither depends on a render someone stored. Stored goldens rendered on lavapipe can be added once the CI `gpu` job has run. The project owner should confirm this approach or ask for stored goldens.

### GPU time per slider change at 2560 px (target: under 16 ms)

Synthetic scene, 2560 × 2560 viewport bounds (2560 × 1920 frames), `genzo-cli bench gpu`, release build, 50 steps per row. GPU time comes from timestamp queries. "Engine" is the time inside `Engine::execute`: the render, the readback of the frame, and splitting it into tiles.

**NVIDIA GeForce RTX 3060, Vulkan, Windows 11** (the reference GPU class from the M1 gate):

| Slider | Raw size | GPU p50 | GPU p95 | Engine p50 | Engine p95 |
|---|---|---|---|---|---|
| Exposure drag | 48 MP | 0.11 ms | 0.11 ms | 8.9 ms | 9.9 ms |
| White balance drag | 48 MP | 0.30 ms | 0.31 ms | 9.0 ms | 10.0 ms |
| White balance settled (once per release) | 48 MP | 21.7 ms | 22.6 ms | 30.6 ms | 31.8 ms |
| Exposure drag | 12 MP | 0.11 ms | 0.11 ms | 10.0 ms | 11.3 ms |
| White balance drag | 12 MP | 0.30 ms | 0.31 ms | 9.9 ms | 13.0 ms |
| White balance settled (once per release) | 12 MP | 5.6 ms | 6.3 ms | 15.5 ms | 16.6 ms |

**Result: both drags are far under 16 ms of GPU time.** The settled render at 48 MP takes 22 ms. It runs once, after the drag ends.

**Microsoft Basic Render Driver (WARP), D3D12, Windows 11**, 12 MP, 10 steps:

| Slider | GPU p50 | GPU p95 |
|---|---|---|
| Exposure drag | 35.6 ms | 57.3 ms |
| White balance drag | 468 ms | 482 ms |
| White balance settled | 1289 ms | 1402 ms |

WARP runs the pipeline correctly, and all GPU tests pass on it. It is too slow to drag white balance smoothly, which is what the status bar's software-fallback notice is for. The resample in step 4 is the costly part on WARP.

The criterion benchmark (`cargo bench -p genzo-pipeline --bench spike`, 12 MP, RTX 3060) measures a whole render as the render thread sees it, readback included: output 5.1 ms, viewport 5.6 ms, linearize 9.8 ms.

### Findings for M1

1. **Readback dominates the drag cost**, not the GPU. About 9 ms of the engine time is reading back a 2560 × 1920 RGBA8 frame (19.7 MB) and splitting it into tiles. The 0.E end-to-end measurement will show whether this matters against the 50 ms gate. If it does, the options are reading back only the visible tiles, or staging buffers kept mapped across frames.
2. **GPU memory.** The spike keeps every full-resolution intermediate: about 49 bytes per pixel, or **2.4 GB for a 48 MP image**. The M1 budget is under 1 GB, with full-resolution intermediates per tile only. M1 must tile steps 1 and 2 and keep only the mip pyramid image-wide. The R32F and RG32F intermediates can probably drop to 16-bit.
3. **The FXC shader compiler** (D3D12 on WARP, and possibly on older drivers) rejects writes to a vector through a runtime index (`v[i] += x`). The shaders avoid runtime vector indexing (`pick3`, `onehot` in `common.wgsl`). Keep it that way.
4. **WARP crashes** (access violation) when several devices are alive in one process. The app opens one device, so this only affects tests, which take turns on the GPU.
5. **Settled white balance at 48 MP takes 22 ms of GPU time.** It is not on the drag path, but tiling (finding 2) and a cheaper first pass for the visible area would bring it down.

### Open

- lavapipe: the CI `gpu` job runs the tests on Ubuntu 22.04's mesa. Its first result has not been seen yet.
- macOS (Metal, M1 Max): measured with the bench kit, see 0.E. Exposure and white balance drags take 0.25 to 0.71 ms of GPU time at 12 and 48 MP; the settled white balance render takes 7 ms at 12 MP and 26 ms at 48 MP.
- macOS (Metal, base M1, 7 GPU cores, 8 GB): measured with the bench kit, see 0.E. Drags take 1.2 to 4.7 ms of GPU time (p50) at 12 and 48 MP, about 4 to 5 times the M1 Max, in line with its 7 GPU cores against 32. The settled white balance render takes 34 ms at 12 MP and 124 ms (p95 137 ms) at 48 MP; at 48 MP that pause after release is long enough to notice, which adds weight to finding 5.
- Linux (Vulkan, GeForce RTX 3050 Laptop, 4 GB): measured with the bench kit, see 0.E. Drags take 0.33 to 1.24 ms of GPU time at 12 and 48 MP; the settled white balance render takes 11 ms at 12 MP and 42 ms at 48 MP.
- Real raw files instead of the synthetic scene arrive with 0.C (rawler).

## 0.C ProRAW and RAW decode

In progress (step 2 partly done). Steps 3 to 7 are still open.

### Corpus

`python scripts/fetch-raw-corpus.py` downloads one file per camera body from raw.pixls.us into the gitignored `corpus/raw/` and checks it against `corpus/manifest.json` (sha256). 16 raw.pixls.us bodies (the Z 9 twice, lossless and High Efficiency), plus the iPhone 12 Pro ProRAW. The owner's iPhone 16 Pro 48 MP ProRAW is in `corpus/proraw/iPhone 16 Pro/`, and a friend's iPhone 13 Pro Max 12 MP ProRAW in `corpus/proraw/iPhone 13 Pro Max/`. A file from an iPhone 14 or 15 Pro would still help.

### rawler 0.8.0 gap report (step 2)

`cargo run -p genzo-imgio --features raw --example raw_gap_report -- ../corpus` (debug build, so times are pessimistic).

**Decodes: 16 of 19 files** (the corpus without the owner's iPhone 16 Pro file; with it, 16 of 20), with white and black levels, a camera matrix (D65 and A, or D50 for Leica) and white balance present in all but one. The exception is the Kodak DCS760C, which has no WB coefficients.

Added since this run, both decoding: the iPhone 16 Pro Bayer DNG (Halide) and the iPhone 13 Pro Max ProRAW (below). With every file now in the corpus that is **18 of 22**.

| Result | Files |
|---|---|
| Fails: Nikon High Efficiency NEF ("compression HighEfficency is not supported") | Z 8, Z 9 HE |
| Fails: GoPro GPR ("Unsupported DNG compression", probably VC-5) | HERO6 Black |
| Decodes, LinearRaw (cpp 3, no demosaic) | iPhone 12 Pro ProRAW, Galaxy S23 Ultra DNG; since then iPhone 13 Pro Max ProRAW |
| Decodes, X-Trans (6x6 CFA) | Fujifilm X-T5 |
| Decodes, Bayer | everything else, including Canon R5m2 CR3 (C-RAW), Sony 7RM5 lossless ARW, GFX100S, Hasselblad 3FR, Phase One IIQ (9 s, slowest) |

Gaps that matter for the pipeline:

- `dng_tags` is empty for every DNG, so rawler does not surface OpcodeList1 to 3 or ProfileGainTableMap in a usable form. We need our own TIFF tag reader for those (iPhone and Samsung files carry them).
- ProRAW and Samsung DNG are already linear RGB, so the pipeline needs an input path that skips the demosaic.
- White and black levels are per channel or per CFA position (36 entries for X-Trans). The pipeline must take them as arrays, not scalars.
- The 4th WB coefficient is NaN on every body. Treat only the first three as valid.
- **iPhone 16 Pro ProRAW fails in rawler 0.8.0.** The owner's file `IPhone16proRaw.DNG` (28.6 MB, sha256 `97f76203...b5b378`) is **48 MP** (8064x6048, 3 samples per pixel, LinearRaw). Its main image uses compression 52546 (JPEG XL, DNG 1.7), and it carries a ProfileGainTableMap of 3.1 MB. rawler panics on it ("index out of bounds: the len is 1024 but the index is 1024"), after 8 s. *Corrected 2026-10-16: this was first put down to JPEG XL, but rawler 0.8 does decode JPEG XL tiles (with jxl-oxide too). The panic is in its LinearizationTable lookup, which does not clamp the few samples past the end of the table; see "rawler's LinearizationTable" below.* The iPhone 12 Pro file uses an older encoding and decodes. Our own DNG path with a pure-Rust JPEG XL decoder (`jxl-oxide`, MIT OR Apache-2.0) is the likely fix. PhotoCraft (github.com/storytold/photocraft, `crates/raw`, MIT OR Apache-2.0) does it this way: its DNG reader decodes JPEG XL tiles with `jxl-oxide` pinned exactly, and reports ProfileGainTableMap as "not applied". We can do the same, or go further and apply the gain table map.
- **iPhone 13 Pro Max ProRAW decodes in rawler 0.8.0** (added 2026-10-16). `IMG_8104.dng` (36.1 MB, sha256 `197623dd...f0d62f`), shot on iOS 26.3.1: 12 MP (4032x3024) LinearRaw, 3 samples per pixel, 12-bit, **lossless JPEG** (compression 7, DNG 1.6, 504x504 tiles), so not JPEG XL. rawler reads it in 250 ms with a warm cache (3 s on the first, cold read), with both colour matrices (illuminants 17 and 21) and none of the fields the gap report checks missing. As for every DNG, rawler does not surface its ProfileGainTableMap (see the gaps above); the tags below come from our own TIFF tag reading. Like the 16 Pro file it has a 4096-entry LinearizationTable, AsShotNeutral 1,1,1 (white balanced in the camera), DefaultBlackRender 1, no ForwardMatrix and no opcode lists. It also carries a **ProfileGainTableMap** (3.1 MB), so the gain table map is not limited to the 16 Pro: a current iOS writes it on older phones too. Its **BaselineExposure is +2.50 EV**, against +0.51 on the 16 Pro file, so the default look depends on honoring BaselineExposure. With the 12 Pro, that makes three iPhone generations, of which the two older ones decode in rawler and the 16 Pro only in the JPEG XL prototype.
- **iPhone Bayer DNG decodes in rawler 0.8.0.** The owner's `Halide_iPhone16Pro_bayer.dng` (16.0 MB, sha256 `a6c97126...38b547`) is a Halide RAW capture from the **ultra wide** camera (4224x3024, 12 MP, RGGB Bayer, lossless JPEG compression, white level 4095, black level 528). rawler decodes it in 349 ms with no missing fields (WB, two colour matrices, crop). So the Bayer DNG path is covered for iPhone, with one device. It is the same phone as the ProRAW sample, so it adds a second sensor, not a second generation. The file is kept under `corpus/bayer-dng/` (gitignored).
- Nikon HE is the one mainstream gap. Whether LibRaw can read it is step 3 and is not checked yet.


### JPEG XL ProRAW prototype (iPhone 16 Pro, 48 MP)

`cargo run --release -p genzo-imgio --features raw --example dng_jxl_probe -- FILE [full|mip]`. A small TIFF reader finds the raw SubIFD and its 12 tiles of 2016x2016 (4x3 grid), and `jxl-oxide` 0.12.6 decodes each tile. Release build, tiles decoded one after another.

| | Decode time, 48 MP | Peak memory |
|---|---|---|
| `full`: assemble the whole image as u16 (279 MiB) | 1.84 s | 445 MiB |
| `mip`: box-filter each tile to 1/8 size and drop it | 1.77 s | 164 MiB |

- The format is no obstacle. Tiles decode in about 145 ms each; running them in parallel on rayon should cut the 1.8 s a lot. Not measured yet.
- Streaming tile by tile saves 280 MiB at 48 MP. The 164 MiB peak is the whole file read into memory (28.6 MB) plus decoder working space, and could be lower with a memory-mapped or streamed read.
- **LinearizationTable.** Stored sample values are indices into a 1024-entry table (tag 50712) that maps to linear 16-bit (0 to 65535, WhiteLevel 65535, BlackLevel 0). The decoded values run from 8 to 1029, so **3 samples in the whole 48 MP image fall past the end of the table**. The DNG spec has no rule for that, and clamping to the last entry is the safe choice. This explains the rawler panic: it indexes the table without clamping. With the table applied the mean linear value is about 3900 to 4700 of 65535 per channel (a dark, not yet exposure-corrected image).
- **Tags this file carries:** ColorMatrix1 and 2, AsShotNeutral 1,1,1 (ProRAW is already white balanced in the camera, so no WB gain is needed), BaselineExposure +0.51 EV, NoiseProfile, DefaultBlackRender. No ForwardMatrix and **no opcode lists** in this file, so the iPhone 16 Pro needs no GainMap or warp opcode handling. Opcode lists still matter for other DNGs (Samsung, Pixel), not tested.
- Still to do for the real DNG path: parse opcode lists for the files that have them, and the ProfileGainTableMap reader (kept but off by default, see below).

### The linear DNG reader (2026-10-16)

The prototype became a library reader, `genzo_imgio::dng::decode_linear` (`backend/crates/imgio/src/dng.rs`, default `dng` feature). It reads the full-size LinearRaw image of a DNG, applies the LinearizationTable exactly with clamping, and returns the levels, matrices, neutral, BaselineExposure and flags for the gain table map and opcode lists. Tiles may be JPEG XL, uncompressed, or (with the `raw` feature) lossless JPEG, decoded by rawler's lossless JPEG decompressor. **Every ProRAW, of every iPhone generation, goes through this reader**; CFA (Bayer) DNGs and camera RAW stay with rawler. It is not part of `imgio::decode` yet; the normalized RAW output is M1 step 2.

- **Hardened for untrusted files.** Every read is bounds-checked, sizes are capped before allocating, and each JPEG XL header is checked against the tile grid before decoding. Output is capped at 400M samples and allocated fallibly, so a tiny file claiming a huge image gets an error, not an abort. Unit tests cover truncated and byte-flipped files. The `dng_linear` fuzz target (seeds in `backend/fuzz/seeds/`) ran for 10 minutes on 2026-10-16: 708,050 inputs, no crash, hang or out-of-memory, peak RSS 594 MB. With lossless JPEG added it found a panic inside rawler's decompressor (now caught, see below, and kept as a regression test); a second 10-minute run then found nothing in 581,486 inputs (peak RSS 630 MB).
- **Bit depth.** jxl-oxide rescales samples to 0..65535 unless a stream declares 16 bits, which would break the table lookup. The reader scales back from the declared depth; a synthetic test checks that 16- and 12-bit tiles give the same integers. The iPhone streams declare 16 bits (the DNG says 10), so the iPhone files did not hit this.
- **Second iPhone 16 Pro file:** `IMG_6889.DNG` (20.3 MB, sha256 `8e93f79c...7f50d1`, iOS 27.2, big-endian). 48 MP, 12 tiles of 2016x2016, **lossy** JPEG XL (VarDCT, XYB), a 1024-entry table, gain table map, BaselineExposure +0.13 EV. 7 samples past the table, mean linear value 6789, 6669, 6715.
- **Checked against libjxl**, the reference decoder (imagecodecs, table applied in NumPy): the same clamp count and means, and 99.982 % of samples identical. The other 0.018 % differ by exactly one table step, up or down: lossy tiles decode to fractions, and the two decoders round the last step differently. The prototype gives the same clamp count and means.
- **The prototype's own file**, `IMG_6884.DNG` (the `97f76203...b5b378` above, also big-endian lossy JPEG XL): the reader reproduces the recorded values, 3 samples past the table and means 3901, 4229, 4705. Against libjxl, 0.017 % of samples differ, almost all by one table step. 15 samples in 8 strongly saturated pixels (one channel near zero) differ by 2 to 5 steps, at most 80 of 65535: lossy colors are stored as XYB, and converting back to RGB amplifies small float differences at the edge of the gamut. Decoder variance, not a reader error.
- **Speed and memory** on the owner's laptop (i7-10510U, 4 cores), release build, file read first:

  | Decode time and peak memory, 48 MP | `IMG_6889` (20.3 MB) | `IMG_6884` (28.6 MB) |
  |---|---|---|
  | Prototype (`full`, tiles one after another) | 5.35 to 5.83 s, 443 to 455 MiB | 8.82 to 9.79 s, 451 to 466 MiB |
  | Linear DNG reader (rows of tiles in parallel) | 2.60 to 2.75 s, 602 to 638 MiB | 4.82 to 5.35 s, 628 to 661 MiB |

  1.8 to 2 times as fast, for about 170 MiB more at peak while three rows of tiles decode at once. (The prototype's 1.84 s in the table above was on the faster Windows machine.) Streaming tiles into the mip pyramid, as the `mip` mode showed, would cut the 279 MiB full-size buffer when M1 wires this into the pipeline.

### rawler's LinearizationTable (checked 2026-10-16)

The question was whether one decoder, rawler, could read every ProRAW, so the linear DNG reader could go. With a one-line clamp patched in, rawler 0.8 reads both iPhone 16 Pro files (1.5 s, about 1 GB peak, faster than our reader). But comparing every sample against libjxl turned up three faults in how it applies the table (`LookupTable::dither`):

| | Linear DNG reader | rawler 0.8 |
|---|---|---|
| Samples past the end of the table | clamped and counted | **panic**: the original 16 Pro failure |
| Samples at the top of the table | 65535 | **wrap to 3 to 38**: blown highlights turn black. 4 and 8 samples in the two 16 Pro files, which barely clip; a frame with the sun in it would have many |
| Every other sample | exact lookup | **dithered**: 93 % of samples get noise of up to one table step (mean absolute difference 6 to 8 of 65535) |

The wrap happens because `dither` adds its offset in `u32` and returns `pixel as u16`; it affects any DNG whose table ends at 65535, not only ProRAW. Fuzzing later found a fourth fault, in rawler's lossless JPEG decompressor: a damaged Huffman table makes `huff_len` index out of bounds (`huffman.rs:196`).

**Decision:** every ProRAW goes through the linear DNG reader, which applies the table exactly. Lossless JPEG tiles (iPhone 12 to 15 Pro) are decoded with rawler's public `LjpegDecompressor` inside `catch_unwind`, so a panic there is an error. rawler keeps CFA DNGs and camera RAW. The table and Huffman faults are drafted as an upstream issue for rawler.

- **iPhone 13 Pro Max through the reader:** `IMG_8104.dng` (12 MP, 12-bit lossless JPEG, 48 tiles of 504x504, 4096-entry table) decodes in about 140 ms (rawler: 250 ms), peak 120 MiB. **Every one of its 36,578,304 samples is identical** to an independent decode with libjpeg-turbo plus the table. Means 1288, 1274, 1268; no sample past the table.
- **iPhone 12 Pro through the reader** (2026-10-17): `IMG_1361.DNG` from raw.pixls.us (29.2 MB, sha256 `e91e77a4...fdbdfc2`): 12 MP, 12-bit lossless JPEG, 504x378 tiles, 4096-entry table, decodes in 54 ms. **Every one of its 36,578,304 samples is identical** to an independent decode with libjpeg (imagecodecs and tifffile) plus the table. Means 4988, 12571, 7970; no sample past the table. Unlike the 13 Pro Max and 16 Pro files it is **not white balanced in the camera**: AsShotNeutral is 0.449, 1, 0.541, so the M1 RAW path must apply the neutral rather than assume 1,1,1. BaselineExposure is −0.30 EV, and it carries the gain table map flag too. Checked by `iphone_12_pro_lossless_jpeg` in `backend/crates/imgio/tests/dng_corpus.rs`.

### LibRaw check (step 3) and what it means for the FFI fallback (step 4)

Read from the LibRaw 0.22.2 source (the newest release, 2026), not run:

- **Nikon High Efficiency: not supported.** `src/tables/cameralist.cpp` lists the Z 6III, Z 8, Z 9 and Z f as "HE/HE* formats are not supported yet". LibRaw reads the compression tag and spots the JPEG XS signature, but has no decoder. So neither rawler nor LibRaw reads Nikon HE.
- **JPEG XL DNG (iPhone 16 Pro ProRAW): not without Adobe's DNG SDK.** LibRaw only decodes DNG 1.7 JPEG XL when built with `-DUSE_DNGSDK`, and it just stops refusing such files at open time otherwise (`jxl_dng_load_raw_placeholder`).
- **GoPro GPR: not without the GPR SDK**, which also needs a patched Adobe DNG SDK (`README.GoPro.txt`).

So **LibRaw would not fix any of the three files rawler fails on.** The LibRaw FFI prototype from step 4 has no test file in the corpus: nothing exists that rawler can't read and LibRaw can. Two ways forward: (a) skip the FFI crate for now and revisit if a real file turns up that only LibRaw reads, which saves a second unsafe crate and a C++ build; (b) hunt for such a file, first across the rest of raw.pixls.us. Recommendation: (a). The gaps that matter are covered by our own code (JPEG XL DNG with `jxl-oxide`) or by nobody yet (Nikon HE, JPEG XS).

### Adobe DNG Converter references (step 5)

Adobe DNG Converter 18.6 runs headless: `"Adobe DNG Converter.exe" -c -l -u -d <outdir> <file>` writes a **linear, uncompressed, demosaiced DNG** in 2 to 6 s per file. `scripts/make-dng-references.ps1` wraps it and writes `corpus/reference/<stem>/<stem>.dng` (gitignored, 58 to 350 MB each). Made for 10 files: D850, Z 9 (lossless), Z 8 (HE), Sony 7RM5, X-T5, Canon R5m2 (C-RAW), Panasonic S5M2, iPhone 12 Pro, iPhone 16 Pro and Galaxy S23 Ultra.

- **Adobe reads Nikon High Efficiency**: the Z 8 HE file converted fine. So we have an Adobe reference for a file neither rawler nor LibRaw can decode, which is also proof that a correct decoder exists.
- **Adobe also reads the iPhone 16 Pro file** and writes it as plain 16-bit LinearRaw. rawler reads these outputs in 17 to 72 ms (`raw_gap_report ../corpus/reference`), so they can be loaded as pixel references without any special decoder. Black levels stay in the file (for example 2048 for the Sony), so apply them when comparing.
- **Not covered: Apple Photos renders.** Those need a Mac. The owner would export the same iPhone files as TIFF or JPEG from Photos into `corpus/reference/<stem>/`.

### Proposed camera priority order (step 7)

Weights are LightCraft's estimate of Lightroom raw users by maker (Canon 33%, Sony 27%, Nikon 16%, Fujifilm 10%, Panasonic 4%, OM 3%, phones 3%, Pentax/Ricoh 1.5%, Leica 1%); they are not our data. "Decodes" means sensor data and levels came out of one corpus file per body, not that colors are right.

| Order | What | Share | Status in our corpus | Work for M1 |
|---|---|---|---|---|
| 1 | Canon CR3, Sony ARW, Nikon NEF (lossless and lossy compressed), Fujifilm RAF (Bayer and X-Trans) | ~86% | All decode with rawler 0.8 on the files tested | Verify more bodies per maker; only one Canon CR3 (an APS-C C-RAW) is tested. X-Trans needs its own demosaic |
| 2 | Phones: Apple ProRAW (all generations), Samsung and other DNG | 3% | 12 Pro, 13 Pro Max and 16 Pro decode with the linear DNG reader; S23 Ultra with rawler | Wire the linear DNG reader into `imgio::decode`, opcode lists. Gain table map off by default |
| 3 | Panasonic RW2, OM/Olympus ORF, Leica/Pentax DNG | ~9% | All decode with rawler | Per-body checks only |
| 4 | Nikon High Efficiency NEF (Z 6III, Z 8, Z 9, Z f in HE mode) | part of Nikon's 16%, only shots taken in HE | Fails in rawler and LibRaw; Adobe reads it | Needs a JPEG XS decoder (est. 20 to 40 h). Until then, show the embedded preview and say why. Decide in M1 whether it is worth the time |
| 5 | Medium format and legacy: Hasselblad 3FR, Phase One IIQ, Kodak DCR | <1% | Decode with rawler (IIQ is slow: 9 s debug) | Nothing planned |
| 6 | GoPro GPR | <1% | Fails; needs a wavelet codec (VC-5) | Out for now |

Consequence for the plan: **no LibRaw fallback is wired in M1** (see the LibRaw section). The spec's parked decision "camera priority order" can be closed with this table.

### LightCraft's raw crate on our corpus (run 2026-10-15)

Built `lightcraft-raw` from github.com/storytold/lightcraft (shallow clone of main) and ran its `rawinfo` example on all 20 files. **It decodes 13 of 20. rawler decodes 16 of 20.**

| File | rawler 0.8 | LightCraft raw |
|---|---|---|
| iPhone 16 Pro ProRAW (JPEG XL) | fails | **decodes** (0.89 s, 55 MP/s) |
| Olympus E-M1 II and OM-3 compressed ORF | decodes | fails (preview only) |
| Phase One IIQ | decodes | fails |
| Kodak DCR | decodes | fails |
| Nikon Z 8 and Z 9 High Efficiency | fails | fails (preview only) |
| GoPro GPR | fails | fails |
| The other 12 (Canon CR3, Sony ARW, Nikon lossless, Fujifilm RAF both, Panasonic, Leica, Pentax, Hasselblad 3FR, iPhone 12 Pro, S23 Ultra) | decodes | decodes |

So LightCraft handles exactly **one** of our problem files better, the JPEG XL one, which `jxl-oxide` also gives us. It handles none of the others and loses four formats rawler has (compressed ORF, IIQ, DCR). It reads the opcode lists (Samsung has a 4-entry OpcodeList2, Fuji and Leica have some) and DNG metadata, which rawler leaves empty. Its decode is also faster on the 48 MP file (0.89 s against our sequential 1.8 s), probably from parallel tiles.

Conclusion: don't replace rawler with it. Use rawler for the formats it covers, and take the DNG pieces from LightCraft (JPEG XL tile decode, LinearizationTable, opcode lists, gain table map reader), with attribution, since its license (MIT OR Apache-2.0) allows it.

### Prior art: LightCraft and PhotoCraft (read 2026-10-15)

Two MIT OR Apache-2.0 projects from the same authors (github.com/storytold/lightcraft, a Lightroom reimplementation, and github.com/storytold/photocraft). Both use a clean-room pure-Rust raw crate. Their license lets us read and reuse code with attribution, and is compatible with GPL-3.0. Nothing is copied yet. What they tell us:

- **ProRAW gain table map: leave it off by default.** LightCraft reads and keeps `ProfileGainTableMap` (tags 52525 and 52544) but does not apply it, because Lightroom Classic renders ProRAW without it: with the map on, their ΔE00 against Lightroom was 19. It offers "camera local tone mapping" as an opt-in. Their `crates/raw/src/gaintable.rs` documents the interpolation. For Genzo, which is modeled on Lightroom, this likely settles the default.
- **JPEG XL DNG works with `jxl-oxide`** (pinned exactly), including real lossless and lossy files, so the iPhone 16 Pro failure is solvable in pure Rust. They also read iPhone semantic mattes from the DNG for subject and sky masks.
- **Nikon High Efficiency is JPEG XS, not decodable in pure Rust today.** They treat it as preview-only and estimate 20 to 40 hours for a decoder, worth about 1.1 points of coverage (only shots taken in HE; lossless works on the same bodies). This supports treating HE as a LibRaw question, not a blocker.
- **Camera priority input** (`docs/raw/usage-weights.tsv` there, their estimate from 2025 market data and their issue tracker): Canon 33%, Sony 27%, Nikon 16%, Fujifilm 10%, Panasonic 4%, OM/Olympus 3%, phones 3%, Pentax/Ricoh 1.5%, Leica 1%. Their biggest raw gaps by gain per hour were compressed ORF and CR3 variants, then Nikon HE. Treat as a starting point for step 7, not data of ours.
- **Docs worth mining:** `docs/raw-parity.md` (per-format support and how they verify it), `docs/raw/coverage.tsv`, `docs/gaps.md`, `docs/cr3.md`, `docs/raf-compression.md`, `docs/arw6-compression.md`.

## 0.E Pixels through the webview

The tile protocol, the `genzo-frame` URI scheme, the WebGL2 viewport and the in-app latency probe exist from the M0 foundation. The app's Exposure and Temp sliders now drive the real GPU pipeline (`renderSpike`), sending interactive renders while dragging and one settled render on release. **Windows, macOS and Linux are measured (below).**

### Windows 11, WebView2 155 (RTX 3060), release build

Measured with `frontend/scripts/measure-latency.mjs`, which attaches to the real app's webview over the DevTools protocol (WebView2 started with `--remote-debugging-port`) and drags the Exposure slider with real mouse events at about 120 Hz, 4 to 6 drags of 120 steps. The numbers are the app's own probe: input to pixels on screen, "latest input wins", as the user feels it during a drag. 12 MP synthetic raw.

| Frame | Probe p50 | Probe p95 | Frame fetch p50 | Frame size |
|---|---|---|---|---|
| 2560 px wide (gate case) | 197 to 218 ms | 220 to 236 ms | 83 to 91 ms | 11.1 MB |
| 1280 px wide | 53 ms | 70 ms | 24 ms | 2.8 MB |

**The gate (under 50 ms p95 at 2560 px) fails on Windows by about 4x.** The GPU is not the cost: 1.3 ms per frame. The time goes into moving the pixels to the page.

- Reading the frame through the `genzo-frame` custom URI scheme runs at about 120 to 130 MB/s on WebView2, so cost is linear in bytes: 24 ms for 2.8 MB, 85 ms for 11 MB. That is about 40 percent of the total.
- The rest is the IPC call (render, readback, tile split: about 10 ms engine side in the 0.D numbers), the WebGL upload and draw, and queueing. While dragging, the probe measures from the oldest input that has not been served, so a request that takes T ms shows up as between T and 2 T. A single isolated input would measure nearer 100 to 115 ms, which still fails.
- Halving the frame (ladder step 1, a proxy while dragging) lands at 53 ms p50 and 70 ms p95. That is close but still over at p95.

**Consequence:** ladder step 1 alone is not enough on Windows. Next experiments, in order: (1) a proxy of about 1280 px while dragging with the full frame on release, combined with (2) a compressed frame (JPEG or QOI, decoded in the page) so a 2560 px frame is around 1 MB instead of 11 MB. Not built yet. The native wgpu surface (step 3) stays the last resort.

**Measured parts of the fix (Windows, 2026-10-15).** A JPEG frame instead of raw RGBA8:

| Frame | Encode (jpeg-encoder 0.7, 1 thread, q80) | JPEG size | Decode + WebGL upload in the webview (p50) |
|---|---|---|---|
| 2560 x 1080 | 19 ms (35 ms at q90) | 405 KB (821 KB at q90), vs 10.8 MB raw | 13.6 + 3.1 = 16.7 ms |
| 1280 x 540 | 4.8 ms (8.6 ms at q90) | 103 KB (209 KB at q90), vs 2.7 MB raw | 3.8 + 0.7 = 4.5 ms |

The test image is synthetic (gradients, texture, noise), so real photos will be larger and slower to encode. Estimated cost of one drag frame (render and readback, encode, fetch at about 120 MB/s, decode, upload): about 39 ms for a full 2560 px JPEG, about 15 ms for a 1280 px JPEG. The probe reads between 1x and 2x of that under continuous input (observed 1.8x today), so only the **1280 px proxy as JPEG** is expected to pass 50 ms p95 with margin (about 25 to 30 ms); a full-size JPEG is not (about 70 ms). Encoding the tiles in parallel on rayon should cut the 2560 px encode to about 5 ms. These are model numbers built from measured parts, not an end-to-end measurement.

### After the fix: half-size JPEG preview while dragging (Windows, 2026-10-15)

Implemented as planned. While a slider is dragged at Fit zoom, the engine renders as before, a small GPU pass (`shrink`) halves the finished frame, only that is read back, and up to four horizontal bands are JPEG-encoded in parallel on rayon (quality 80, `jpeg-encoder`). The webview decodes the bands with `createImageBitmap` and draws them scaled up. On release a full lossless frame is sent as before. The contract moved to 0.3.0 (new `JpegSrgb` and `JpegDisplayP3` pixel formats, and `proxy` on `renderSpike`).

| Windows 11, WebView2 155, RTX 3060, release build, 12 MP synthetic raw, 2560 px frame | Before | After |
|---|---|---|
| Input to pixels p50 (real app, probe) | 197 to 218 ms | **17.8 to 17.9 ms** |
| Input to pixels p95 (real app, probe) | 220 to 236 ms | **18.4 ms** (two runs) |
| Frame fetch p50 | 83 to 91 ms (11.1 MB) | 1.6 to 1.7 ms (28 to 42 KB per frame: all four bands arrive as one message) |
| Engine time per exposure drag frame (`genzo-cli bench gpu`, 12 MP) | 11.8 ms p50 | 4.1 ms p50, p95 4.6 ms, JPEG encode included |

**The 50 ms gate passes on Windows with a wide margin.** The model estimate of 25 to 30 ms p95 was pessimistic.

Caveats, to settle before this counts as final:
- **Real photographs, checked separately.** The spike's own scene is synthetic and compresses far better than a photo, so I took the ten Adobe linear references of the corpus (iPhone 12 and 16 Pro, Galaxy S23 Ultra, Sony 7RM5, Nikon D850, Z 8, Z 9, Canon R5m2, Fujifilm X-T5, Panasonic S5M2), made 1280 px wide previews (the half-size frame a 2560 px viewport sends) and encoded them exactly as the engine does (q80, 4 bands in parallel):

  | | Range over the 10 photos |
  |---|---|
  | Preview JPEG size, whole frame | 78 to 254 KB (the app's synthetic frame was 28 to 42 KB) |
  | Encode, 4 bands on rayon | 2.4 to 3.6 ms |
  | Decode all bands and upload to WebGL in the real WebView2 | 3.5 to 5.3 ms p50, 3.9 to 5.3 ms p95 |
  | Fetch at about 120 MB/s | 0.7 to 2.1 ms (derived from the size) |
  | For comparison, a full 2560 px JPEG | 220 to 754 KB, encode 9 to 14 ms |

  The 18.4 ms p95 measured in the app is for the synthetic scene, whose frame was only 28 to 42 KB. Real photos cost more in three places, compared with that frame:
  - Fetch: 36 to 226 KB more at about 120 MB/s, so 0.3 to 1.9 ms more.
  - Encode: 2.4 to 3.6 ms on photos. The synthetic frame's encode was not timed on its own, but the whole engine step (render, readback, encode) took 4.1 ms, so the extra is at most about 3 ms.
  - Decode and upload: 3.5 to 5.3 ms on photos, against an unmeasured but smaller figure for the smooth frame, so at most about 2 ms more.

  That adds at most about 7 ms, so the expected figure is about 19 to 26 ms p95. It is an estimate from measured parts, not an end-to-end measurement: the spike cannot load a real image, so real-photo latency in the app has not been measured. The conclusion is unchanged: the preview stays well under 50 ms p95 on Windows, and the full-size JPEG alternative would not have (about 40 ms before the probe's queueing).
- **macOS passes** (M1 Max and base M1, below). **Linux passes in WebKitGTK's default mode** on an RTX 3050 laptop, in the three gated cases it could measure (31 to 43 ms p95); the fourth ran out of GPU memory. The dry run on a weak laptop is below too.
- **Display P3 frames** are checked by the color probe (step 5, below). It found that WebView2 desaturated P3 drag previews, now fixed. The app itself still requests sRGB frames.
- **At 1:1 zoom** the preview is not used (it would show at the wrong scale), so a 1:1 drag still sends full frames. That case needs its own answer in M1 (send only the visible region).
- **The settled frame** still moves 11 MB raw, about 90 ms. Nobody waits on it during a drag, but it is the next thing to compress if the pause after release feels slow.
- The CI bench gate has no baseline for the new `... preview` cases yet; they are reported as new until the baseline is refreshed.

**macOS and Linux have no DevTools attach**, so the script above does not work there. Instead the app benchmarks itself: started with `GENZO_AUTOBENCH=FILE` (contract 0.4.0), it runs the same drag protocol with synthetic input and writes a JSON report (`frontend/src/lib/spike/autobench.ts`). The Linux bench kit (`scripts/bench-kit/`, built by the "Bench kit" workflow on ubuntu-22.04) wraps it for testers: it records the hardware, runs `cli bench gpu`, then runs the app in each WebKitGTK drawing mode. After the cases, the app runs the color probe (step 5, below).

### Color handling: sRGB and Display P3 frames (step 5)

The app tags each frame with its encoding (`rgba8Srgb`, `rgba8DisplayP3`, and the JPEG previews `jpegSrgb`, `jpegDisplayP3`), and the WebGL viewport sets the canvas's `drawingBufferColorSpace` to match, so the webview converts to the monitor (the spec's v1 color policy). Two paths can go wrong in different ways:

- **Raw RGBA8 tiles** upload as typed arrays, which no color setting touches. They are right as long as the webview honors `drawingBufferColorSpace`. If it ignores it, P3 values show as sRGB, too saturated. Reading the canvas back cannot see that, so the probe records whether the setting sticks.
- **JPEG previews** are decoded by the webview into bitmaps and uploaded as images, which a webview may color-convert on the way in. A wrong conversion changes the values in the canvas, which reading back does see.

**The color probe** (`frontend/src/lib/spike/colorProbe.ts`) runs at the end of every autobench, after the gated cases, so it does not change their numbers. It loads a small synthetic scene (1200 × 800), renders it four ways (raw and JPEG, sRGB and P3), draws each through the viewport's own upload path into a detached canvas, and reads back the centers of the scene's 12 flat patches. The engine's raw bytes at the same points are the truth. A path passes within 2 codes (raw) or 6 codes (JPEG). It also records whether `drawingBufferColorSpace` keeps 'display-p3' and the display's gamut (CSS `color-gamut`). The bench kit summaries print one line per path. Read-back happens before the compositor, so no wide-gamut monitor is needed for this part.

**Windows 11, WebView2, RTX 3060, sRGB monitor (2026-10-17):**

| Path | Before the fix | After the fix |
|---|---|---|
| raw sRGB | 0 | 0 |
| JPEG sRGB | 1 | 1 |
| raw P3 | 0 | 0 |
| JPEG P3 | **15, wrong** | 1 |

Largest error per channel, in 8-bit codes. For scale, the same patches differ by up to 25 codes between their sRGB and P3 encodings. `drawingBufferColorSpace` keeps 'display-p3'.

- **The bug: WebView2 converted P3 previews a second time.** The viewport set `unpackColorSpace` to the frame's space as well. A decoded JPEG bitmap counts as sRGB, so with 'display-p3' there WebView2 converted the already-P3 values from sRGB to P3 again, and the preview came out desaturated (a red patch at 159 read back as 151). Setting `UNPACK_COLORSPACE_CONVERSION_WEBGL` to `NONE` did not stop it. **The fix:** `unpackColorSpace` stays sRGB for every frame, so nothing converts at upload. Raw tiles ignore it either way.
- The gated latency cases ran unchanged in the same runs (25 to 31 ms p95 in the autobench on this machine, as before).
**macOS Tahoe 26.6.2, WKWebView, base M1, kit bfab0cd (2026-10-17):** all four paths read back within 1 code (raw 0, JPEG 1), so WKWebView uploads P3 previews unchanged. The gated latency cases ran in the same run at 32 ms p95 each, as in the earlier base M1 run.

- **WKWebView has no `unpackColorSpace`** (it reads back as `undefined`), yet that kit reported it as supported: assigning an unknown property just adds a plain JavaScript property, which reads back fine. The probe now checks that the context implements the property before trusting the read-back, and reports 'unsupported' per path otherwise. The same blind spot applied to `drawingBufferColorSpace`, so this run does not prove WKWebView honors it, though Safari has supported it since 16.4. The Linux run needs the fixed probe for its answer.
- The page reported an **sRGB display gamut** (`color-gamut`), on a MacBook whose built-in display covers P3. Not explained yet; the visual check below would show whether P3 reaches the screen.

- **Still open:** the probe on Linux (WebKitGTK), where whether `drawingBufferColorSpace` is implemented is the open question; and one look by a person at a P3 display (the Mac): an sRGB red and a P3 red side by side should differ there, and match on an sRGB display.

### macOS 26.3, M1 Max, WKWebView (WebKit 21623.2.7), bench kit 7a40671 (2026-10-16)

A friend's MacBook Pro (M1 Max, 32 GPU cores, 64 GB, on AC power, built-in Liquid Retina XDR display), release build from the bench kit, synthetic input (autobench), 2560 × 1440 frame. The tester saw the image change.

| Case | Input → pixels p50 / p95 | One request alone, p50 / p95 | of which: send to event (engine) | event to draw (fetch, size) |
|---|---|---|---|---|
| 12 MP, Exposure, Fit | 32 / 33 ms | 17 / 18 ms | 3 / 3 ms (2 / 3) | 14 / 15 ms (0 / 1 ms, 39 KB) |
| 12 MP, Temp, Fit | 31 / 32 ms | 17 / 18 ms | 3 / 3 ms (2 / 3) | 14 / 15 ms (0 / 1 ms, 27 KB) |
| 12 MP, Exposure, 1:1 (full frames) | 32 / 33 ms | 17 / 18 ms | 4 / 3 ms (3 / 3) | 13 / 15 ms (4 / 5 ms, 10.5 MB) |
| 48 MP, Exposure, Fit | 32 / 33 ms | 17 / 18 ms | 3 / 3 ms (2 / 2) | 14 / 15 ms (0 / 1 ms, 20 KB) |
| 48 MP, Temp, Fit | 32 / 33 ms | 17 / 18 ms | 3 / 3 ms (2 / 6) | 14 / 15 ms (0 / 1 ms, 17 KB) |

- **The gate passes on macOS with room to spare: 33 ms p95 against 50 ms**, at 12 and 48 MP, and even at 1:1 with full frames. No ladder step is needed beyond the JPEG preview the app already sends.
- **The display sets the pace, not the pipeline.** Every case lands on the same numbers: one request takes about 17 ms, one 60 Hz frame, and a probe sample (about two requests, see the Linux section) about 33 ms. The engine and IPC take about 3 ms, and the fetch is near zero. The wait for the next animation frame is most of the time; that it matches 60 Hz on a 120 Hz display suggests WebKit caps animation frames at 60 Hz there (not checked).
- **The custom URI scheme is fast in WKWebView**: 10.5 MB in 4 to 5 ms (about 2 GB/s), against about 200 MB/s in WebKitGTK on the Linux laptop. Full frames at 1:1 are no problem here.
- 48 MP fits easily in 64 GB of unified memory; GPU time per drag step is under 1 ms (0.D above).
- The input is synthetic. Running the autobench once on the Windows machine of the 18.4 ms result would check that it agrees with the DevTools measurement.

### macOS 26.6, base M1, WKWebView (WebKit 21624.5.1), bench kit 5755b0b (2026-10-17)

A MacBook with the base M1 (8 CPU cores, 7 GPU cores, 8 GB), on battery with low power mode off, built-in 2560 × 1600 Retina display. Release build from the bench kit, synthetic input (autobench), 2560 × 1440 frame. The tester saw the image change. This is the weakest Apple silicon machine the app is likely to meet.

| Case | Input → pixels p50 / p95 | One request alone, p50 / p95 | of which: send to event (engine) | event to draw (fetch, size) |
|---|---|---|---|---|
| 12 MP, Exposure, Fit | 23 / 32 ms | 17 / 18 ms | 5 / 5 ms (3 / 4) | 12 / 13 ms (0 / 1 ms, 39 KB) |
| 12 MP, Temp, Fit | 22 / 32 ms | 17 / 18 ms | 6 / 6 ms (5 / 6) | 11 / 12 ms (0 / 1 ms, 27 KB) |
| 12 MP, Exposure, 1:1 (full frames) | 27 / 32 ms | 17 / 18 ms | 5 / 5 ms (4 / 5) | 12 / 13 ms (3 / 4 ms, 10.5 MB) |
| 48 MP, Exposure, Fit | 23 / 32 ms | 17 / 17 ms | 5 / 4 ms (3 / 4) | 12 / 13 ms (0 / 1 ms, 20 KB) |
| 48 MP, Temp, Fit | 26 / 32 ms | 17 / 18 ms | 6 / 5 ms (5 / 5) | 11 / 13 ms (0 / 1 ms, 17 KB) |

`genzo-cli bench gpu` on the same machine (2560 × 1440 frames, 30 steps per row):

| Slider | Raw size | GPU p50 | GPU p95 | Engine p50 | Engine p95 |
|---|---|---|---|---|---|
| Exposure drag | 48 MP | 1.18 ms | 1.35 ms | 6.5 ms | 16.0 ms |
| Exposure drag, preview | 48 MP | 1.59 ms | 2.12 ms | 5.0 ms | 8.6 ms |
| White balance drag | 48 MP | 4.03 ms | 4.49 ms | 15.3 ms | 17.8 ms |
| White balance drag, preview | 48 MP | 3.97 ms | 4.48 ms | 7.2 ms | 8.2 ms |
| White balance settled (once per release) | 48 MP | 124.4 ms | 136.5 ms | 136.1 ms | 147.0 ms |
| Exposure drag | 12 MP | 1.17 ms | 1.35 ms | 6.1 ms | 8.6 ms |
| Exposure drag, preview | 12 MP | 1.63 ms | 2.01 ms | 5.1 ms | 5.5 ms |
| White balance drag | 12 MP | 3.53 ms | 3.98 ms | 9.3 ms | 15.3 ms |
| White balance drag, preview | 12 MP | 4.69 ms | 7.06 ms | 8.6 ms | 12.9 ms |
| White balance settled (once per release) | 12 MP | 33.9 ms | 37.5 ms | 43.4 ms | 46.3 ms |

Loading the synthetic raw took 47 ms at 12 MP and 181 ms at 48 MP.

- **The gate passes on the base M1 too: 32 ms p95 against 50 ms** in all four gated cases (Fit, 12 and 48 MP), the same as the M1 Max (33 ms). macOS needs no ladder step on any Apple silicon Mac. The 1:1 case is reported, not gated; with full frames it also measures 32 ms p95.
- **The display still sets the pace.** One request takes 17 to 18 ms, one 60 Hz frame, as on the M1 Max. The engine and IPC take 5 to 6 ms here against about 3 ms there, which still fits inside the frame. The lower p50 (22 to 27 ms against 32) comes from the samples landing at different points of the 60 Hz cycle (they alternate between about 21, 28 and 32 ms); with 1 ms timer resolution and a p95 that matches, it is not a real speedup.
- **GPU time is 4 to 5 times the M1 Max**, about the ratio of GPU cores (7 against 32), and still far under the 16 ms target for every drag. The settled white balance render is the cost that shows: 124 ms at 48 MP, so the image would take about an eighth of a second to sharpen after release. Tiling and a cheaper first pass for the visible area (finding 5 in 0.D) are the fix; this machine is the one to check it on.
- **48 MP fits in 8 GB of unified memory.** The spike keeps about 2.4 GB of intermediates at 48 MP (finding 2 in 0.D); the run finished without errors. The M1 budget of under 1 GB matters most on machines like this one.
- **The custom URI scheme stays fast**: 10.5 MB in 3 to 4 ms at 1:1.
- The run was on battery. macOS does not throttle the M1 much on battery without low power mode, but numbers on AC could be slightly better.

### Linux: Ubuntu 22.04, WebKitGTK 2.50.4, X11, GeForce RTX 3050 Laptop 4 GB, bench kit 5755b0b (2026-10-17)

A friend's laptop: Ryzen 5 5600H (12 threads), 14 GB, hybrid graphics (AMD Cezanne integrated plus RTX 3050 Laptop, 4 GB, driver 550.163.01), built-in 1920 × 1080 display at 120 Hz, GNOME on X11. Release build from the bench kit, synthetic input (autobench), 2560 × 1440 frame. The engine picked the RTX 3050 (Vulkan) in every mode. The tester saw the image change. How to read the columns: see the dry run below.

| WebKitGTK mode, case | Input → pixels p50 / p95 | One request alone, p50 / p95 | of which: send to event (engine) | event to draw (fetch, size) |
|---|---|---|---|---|
| default (DMABUF renderer), 12 MP, Exposure, Fit | 27 / 40 ms | 16 / 26 ms | 7 / 9 ms (7 / 11) | 9 / 17 ms (1 / 2 ms, 39 KB) |
| default, 12 MP, Temp, Fit | 27 / 31 ms | 16 / 17 ms | 8 / 3 ms (8 / 18) | 8 / 14 ms (1 / 2 ms, 27 KB) |
| default, 12 MP, Exposure, 1:1 (full frames, reported only) | 111 / 125 ms | 56 / 70 ms | 16 / 17 ms (16 / 17) | 40 / 53 ms (36 / 43 ms, 10.5 MB) |
| default, 48 MP, Exposure, Fit | 29 / 43 ms | 16 / 28 ms | 7 / 12 ms (7 / 11) | 9 / 16 ms (1 / 1 ms, 20 KB) |
| default, 48 MP, Temp, Fit | not measured: GPU out of memory | | | |
| `WEBKIT_DISABLE_DMABUF_RENDERER=1`, 12 MP, Exposure, Fit | 107 / 126 ms | 60 / 76 ms | 24 / 23 ms (16 / 17) | 36 / 53 ms (15 / 16 ms, 39 KB) |
| same, 12 MP, Temp, Fit | 93 / 124 ms | 57 / 74 ms | 22 / 23 ms (10 / 23) | 35 / 51 ms (15 / 17 ms, 27 KB) |
| same, 12 MP, Exposure, 1:1 (reported only) | 144 / 155 ms | 88 / 94 ms | 33 / 32 ms (28 / 29) | 55 / 62 ms (51 / 56 ms, 10.5 MB) |
| same, 48 MP, Exposure, Fit | 94 / 109 ms | 59 / 73 ms | 24 / 24 ms (16 / 17) | 35 / 49 ms (15 / 16 ms, 20 KB) |
| same, 48 MP, Temp, Fit | not measured: GPU out of memory | | | |
| PRIME offload to the NVIDIA GPU | did not start: `Could not create GBM EGL display: EGL_SUCCESS. Aborting...` (exit 134) | | | |

`genzo-cli bench gpu` on the same machine (2560 × 1440 frames, 30 steps per row):

| Slider | Raw size | GPU p50 | GPU p95 | Engine p50 | Engine p95 |
|---|---|---|---|---|---|
| Exposure drag | 48 MP | 0.33 ms | 0.34 ms | 11.2 ms | 18.9 ms |
| Exposure drag, preview | 48 MP | 0.47 ms | 0.47 ms | 4.0 ms | 5.6 ms |
| White balance drag | 48 MP | 1.10 ms | 1.10 ms | 11.8 ms | 18.0 ms |
| White balance drag, preview | 48 MP | 1.24 ms | 1.24 ms | 5.4 ms | 6.5 ms |
| White balance settled (once per release) | 48 MP | 42.3 ms | 42.5 ms | 54.1 ms | 63.2 ms |
| Exposure drag | 12 MP | 0.33 ms | 0.33 ms | 11.2 ms | 17.8 ms |
| Exposure drag, preview | 12 MP | 0.47 ms | 0.47 ms | 4.5 ms | 5.8 ms |
| White balance drag | 12 MP | 1.10 ms | 1.10 ms | 11.9 ms | 12.2 ms |
| White balance drag, preview | 12 MP | 1.24 ms | 1.24 ms | 6.0 ms | 6.6 ms |
| White balance settled (once per release) | 12 MP | 11.1 ms | 11.6 ms | 23.3 ms | 23.7 ms |

Loading the synthetic raw took 126 ms at 12 MP and 516 ms at 48 MP.

- **In the default mode the gate passes in the three gated cases that ran: 31 to 43 ms p95 against 50 ms.** The fourth gated case (48 MP, Temp) was not measured, because the GPU ran out of memory (below). That leaves less room than macOS (32 ms) or Windows (18.4 ms): the worst samples reach 44 to 45 ms. The case should be run again once a failed allocation no longer kills the render thread.
- **The default mode is the one to ship; turning DMABUF off makes everything slower here** (109 to 126 ms p95, a fail). That is the opposite of the MX110 dry run, where the no-DMABUF runs were faster than the watched default run. The split shows the cost is not only in the webview: the engine itself takes 16 ms instead of 7, the IPC about 8 ms instead of under 1, and a 39 KB fetch 15 ms instead of 1. One possible cause, not checked: with DMABUF off, WebKit's web process draws on the NVIDIA GPU too (`nvidia-smi` lists `WebKitWebProcess` there only in that mode), so it competes with the render thread.
- **The custom URI scheme is not the problem it looked like in the dry run.** In the default mode a 20 to 39 KB preview fetch takes 1 to 2 ms, and 10.5 MB at 1:1 takes 36 to 43 ms (about 270 MB/s). The 11 to 15 ms small fetches seen on the MX110 appear only with DMABUF off, here as there. Full frames at 1:1 stay slow on Linux (111 / 125 ms), which is the 1:1 item already open for M1.
- **48 MP runs out of GPU memory on 4 GB in the app**, as it did on 2 GB: `wgpu error: Out of Memory` on the render thread, in both modes, during the 48 MP Temp case after the 48 MP Exposure drags had worked. With the 12 MP image loaded, the app already held 1.0 to 1.2 GB (`nvidia-smi`). The cause was found afterwards (see "GPU memory before and after the fix" below): loading an image allocated the new textures while the old ones were still held, so the 48 MP reload at the start of the Temp case briefly needed two 48 MP images, about 5.3 GB. Fixed in PR #22, which also turns a failed allocation into an error instead of a dead render thread. Tiling the full-resolution intermediates (finding 2 in 0.D) is still the M1 item.
- **One request alone takes about 16 ms at p50** on a 120 Hz display, two display frames. Whether WebKitGTK paces animation frames at 60 Hz here was not checked.
- **The kit's PRIME offload variant does not work on this setup**: WebKit aborts at start, unable to create a GBM EGL display with the NVIDIA EGL vendor forced. It is not needed (the engine already renders on the NVIDIA GPU), but the kit should either drop the variant or detect the failure and say it was skipped.
- GPU time per drag step is 0.33 to 1.24 ms, three to four times the desktop RTX 3060, and the settled white balance render takes 42 ms at 48 MP, about twice the RTX 3060.

### GPU memory before and after the fix (Windows, RTX 3060, 2026-10-17)

Both Linux laptops ran out of GPU memory in the 48 MP Temp case. PR #22 frees the old image's textures before allocating the new ones, and catches device errors on the render thread. To check whether 48 MP now fits in 4 GB before the next Linux run, the Windows kits from before (5755b0b) and after the fix (aa6a785) ran the same autobench on the owner's RTX 3060 (12 GB), with `genzo.exe`'s dedicated GPU memory (the Windows `GPU Process Memory` counter) sampled every 0.3 s:

| Stage | Before the fix | After the fix |
|---|---|---|
| 12 MP cases | 1.22 GB | 0.71 GB |
| 48 MP cases | 2.99 GB | 2.74 GB |
| Peak | **5.32 GB**, a short spike at the start of the 48 MP Temp case | **2.74 GB**, no spike |

- **The spike was one extra 48 MP image** (about 2.3 GB): the autobench loads the image again for each case, and the old `set_image` held both copies for a moment. That is the case that died on both Linux laptops. The 12 MP stage shows the same doubling, smaller.
- **The Windows figures carry over to Linux**: with the 12 MP image loaded, the RTX 3050 laptop reported 1,222 MiB before the fix and this machine 1,221 MiB. The new peak of 2.74 GB, plus about 0.1 GB for WebKit and the desktop, leaves about 1.2 GB of a 4 GB card free. The 0.3 s sampling could miss a shorter spike; none showed after the fix.
- All five cases ran on both kits (12 GB is enough either way), with the same latency as before, about 22 ms p95 at Fit.
- This is an estimate for the 4 GB card, not a run on it; the rerun of the Linux kit (aa6a785) on the RTX 3050 laptop decides the 48 MP Temp case.

### Linux dry run: Ubuntu 24.04, WebKitGTK 2.52.6, X11, GeForce MX110 2 GB (2026-10-16)

The owner's laptop, a weak hybrid GPU (Intel UHD plus MX110), used to test the kit. These numbers do not decide the gate, but they point at where Linux loses time. Release build, 2560 × 1440 frame, synthetic 12 MP raw, half-size JPEG preview while dragging:

| WebKitGTK mode, case | Input → pixels p50 / p95 | One request alone, p50 / p95 | of which: send to event (engine) | event to draw (fetch, size) |
|---|---|---|---|---|
| default (DMABUF renderer), unattended | 0 frames drawn of 12 rendered per case: the display was most likely blanked (see below) | | | |
| default, watched, Exposure, Fit | 129 / 215 ms | 69 / 114 ms | 26 / 9 ms (23 / 31) | 43 / 105 ms (4 / 9 ms, 39 KB) |
| default, watched, Temp, Fit | 87 / 181 ms | 47 / 98 ms | 13 / 14 ms (12 / 20) | 34 / 84 ms (1 / 8 ms, 27 KB) |
| default, watched, Exposure, 1:1 | 326 / 387 ms | 171 / 212 ms | 25 / 36 ms (17 / 32) | 146 / 176 ms (139 / 168 ms, 10.5 MB) |
| `WEBKIT_DISABLE_DMABUF_RENDERER=1`, Exposure, Fit | 72 / 89 ms | 45 / 57 ms | 18 / 16 ms (11 / 12) | 27 / 41 ms (11 / 15 ms, 39 KB) |
| same, Temp, Fit | 70 / 82 ms | 42 / 52 ms | 15 / 14 ms (8 / 9) | 27 / 38 ms (12 / 14 ms, 27 KB) |
| same, Exposure, 1:1 (full frames) | 124 / 143 ms | 72 / 81 ms | 17 / 18 ms (10 / 11) | 55 / 63 ms (51 / 58 ms, 10.5 MB) |
| PRIME offload to the NVIDIA GPU, Fit | 73 / 84 ms | (first run, no split) | | |

How to read it: the gate number (input to pixels) is timed like on Windows, from the oldest input waiting for a frame. With input every 8 ms there is always one waiting, so a sample covers the request it waited behind as well as its own: about two requests. "One request alone" times a single request from send to draw; it splits into send to `renderFinished` (IPC and the engine) and `renderFinished` to draw (fetch, JPEG decode, WebGL upload, the next animation frame). Send to event minus the engine is the IPC. Percentiles of the parts do not add up exactly.

- **This laptop fails the gate in the modes that draw: about 84 to 89 ms p95, against 18.4 ms on Windows.** It is a weak GPU and an old CPU, so the friend's run decides the gate, but the split shows where Linux loses time compared with Windows:
  - the IPC round trip costs about 5 to 7 ms on top of the engine;
  - fetching a 39 KB preview over the custom URI scheme takes 11 to 15 ms (on Windows a 78 to 254 KB preview was estimated at 0.7 to 2.1 ms), and 10.5 MB takes about 55 ms, about 200 MB/s;
  - decode, upload and the animation frame take the remaining 15 to 25 ms.
  The custom scheme is the clearest Linux-specific cost. A Tauri binary channel or fewer round trips (the frame bytes with the event) are the next things to try if the friend's machine shows the same pattern.
- **The default mode does draw** (corrected 2026-10-16). The first default-mode runs drew no frame at all, but they ran with nobody at the laptop. Run again with the owner watching, the image changed and every case drew its 164 to 200 frames. The likely cause is a blanked or locked display: WebKitGTK then stops drawing in the DMABUF mode, while the no-DMABUF runs kept drawing. That is not confirmed, but a tester walking away would lose the run, so the kit now keeps the screen awake (`gnome-session-inhibit` or `systemd-inhibit`, `caffeinate`, `SetThreadExecutionState`) and the report flags a case whose page was hidden.
- **The watched default-mode run was slower** than the no-DMABUF runs (181 to 215 ms p95 at Fit, against 82 to 89), mostly between `renderFinished` and the draw, while the frame fetch was faster (4 to 9 ms). The conditions differed (a person at the machine, other apps open, a different hour), so this needs a repeat on an idle machine before it says anything about the DMABUF renderer. (The RTX 3050 run above points the other way: there the default mode is much faster.)
- **48 MP runs out of GPU memory** on 2 GB (`wgpu error: Out of Memory`), and the panic stops the render thread for good. With only the 12 MP image loaded the app already holds about 1.5 GB of GPU memory (`nvidia-smi`). That matches the 48 MP GPU memory item flagged for M1. A failed allocation should become an error, not a dead render thread.
- `performance.now()` has 1 ms resolution in WebKitGTK, so these samples are whole milliseconds.
- WebKit masks the WebGL renderer ("Apple GPU"), so the kit records the page's GPU from `glxinfo` and `eglinfo` instead.
