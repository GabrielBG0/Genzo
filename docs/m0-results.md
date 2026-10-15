# M0 spike results

Measured results of the M0 feasibility spikes, against the exit gate in [development-plan.md](development-plan.md#m0-feasibility-spikes). Each section lists what was measured, on what, and what is still open.

## 0.D GPU pipeline

**Status: done on Windows (hardware and WARP). lavapipe runs in CI (`gpu` job); its first result is pending. macOS and Linux hardware are not measured yet.**

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
- macOS (Metal) and Linux hardware: not measured.
- Real raw files instead of the synthetic scene arrive with 0.C (rawler).

## 0.C ProRAW and RAW decode

In progress (step 2 partly done). Steps 3 to 7 are still open.

### Corpus

`python scripts/fetch-raw-corpus.py` downloads one file per camera body from raw.pixls.us into the gitignored `corpus/raw/` and checks it against `corpus/manifest.json` (sha256). 16 raw.pixls.us bodies (the Z 9 twice, lossless and High Efficiency), plus the iPhone 12 Pro ProRAW. The owner's iPhone 16 Pro 48 MP ProRAW is in `corpus/proraw/iPhone 16 Pro/`. A 12 MP file from an iPhone 14 or 15 Pro would still help.

### rawler 0.8.0 gap report (step 2)

`cargo run -p genzo-imgio --features raw --example raw_gap_report -- ../corpus` (debug build, so times are pessimistic).

**Decodes: 16 of 19 files** (the corpus without the owner's iPhone 16 Pro file; with it, 16 of 20), with white and black levels, a camera matrix (D65 and A, or D50 for Leica) and white balance present in all but one. The exception is the Kodak DCS760C, which has no WB coefficients.

| Result | Files |
|---|---|
| Fails: Nikon High Efficiency NEF ("compression HighEfficency is not supported") | Z 8, Z 9 HE |
| Fails: GoPro GPR ("Unsupported DNG compression", probably VC-5) | HERO6 Black |
| Decodes, LinearRaw (cpp 3, no demosaic) | iPhone 12 Pro ProRAW, Galaxy S23 Ultra DNG |
| Decodes, X-Trans (6x6 CFA) | Fujifilm X-T5 |
| Decodes, Bayer | everything else, including Canon R5m2 CR3 (C-RAW), Sony 7RM5 lossless ARW, GFX100S, Hasselblad 3FR, Phase One IIQ (9 s, slowest) |

Gaps that matter for the pipeline:

- `dng_tags` is empty for every DNG, so rawler does not surface OpcodeList1 to 3 or ProfileGainTableMap in a usable form. We need our own TIFF tag reader for those (iPhone and Samsung files carry them).
- ProRAW and Samsung DNG are already linear RGB, so the pipeline needs an input path that skips the demosaic.
- White and black levels are per channel or per CFA position (36 entries for X-Trans). The pipeline must take them as arrays, not scalars.
- The 4th WB coefficient is NaN on every body. Treat only the first three as valid.
- **iPhone 16 Pro ProRAW fails in rawler 0.8.0, and the cause is JPEG XL.** The owner's file `IPhone16proRaw.DNG` (28.6 MB, sha256 `97f76203...b5b378`) is **48 MP** (8064x6048, 3 samples per pixel, LinearRaw). Its main image uses compression 52546 (JPEG XL, DNG 1.7), and it carries a ProfileGainTableMap of 3.1 MB. rawler has no JPEG XL decoder, so it panics ("index out of bounds: the len is 1024 but the index is 1024"), after 8 s. The iPhone 12 Pro file uses an older encoding and decodes. So **every ProRAW from the iPhone 16 Pro on is likely unreadable by rawler**, and Samsung Expert RAW will be the same. Our own DNG path with a pure-Rust JPEG XL decoder (`jxl-oxide`, MIT OR Apache-2.0) is the likely fix. PhotoCraft (github.com/storytold/photocraft, `crates/raw`, MIT OR Apache-2.0) does it this way: its DNG reader decodes JPEG XL tiles with `jxl-oxide` pinned exactly, and reports ProfileGainTableMap as "not applied". We can do the same, or go further and apply the gain table map.
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
| 2 | Phones: Apple ProRAW (all generations), Samsung and other DNG | 3% | iPhone 12 Pro and S23 Ultra decode with rawler. **iPhone 16 Pro (JPEG XL) does not** | Own DNG path with `jxl-oxide`, LinearizationTable with clamping, opcode lists. Gain table map off by default |
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

The tile protocol, the `genzo-frame` URI scheme, the WebGL2 viewport and the in-app latency probe exist from the M0 foundation. The app's Exposure and Temp sliders now drive the real GPU pipeline (`renderSpike`), sending interactive renders while dragging and one settled render on release. **Windows is measured (below). macOS and Linux are not.**

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
- **macOS and Linux are still unmeasured**, and the custom-scheme throughput there is unknown. Compression makes that less of a risk, not zero.
- **Display P3 previews** are drawn as untagged bitmaps, so their colors are approximate during a drag. The spike only sends sRGB; the color check (step 5) is still open.
- **At 1:1 zoom** the preview is not used (it would show at the wrong scale), so a 1:1 drag still sends full frames. That case needs its own answer in M1 (send only the visible region).
- **The settled frame** still moves 11 MB raw, about 90 ms. Nobody waits on it during a drag, but it is the next thing to compress if the pause after release feels slow.
- The CI bench gate has no baseline for the new `... preview` cases yet; they are reported as new until the baseline is refreshed.

**Not measured:** macOS (WKWebView) and Linux (WebKitGTK 4.1 on two distros) have no DevTools attach, so the script does not work there. On those the numbers have to be read off the "Input → pixels" rows in the app's right panel while dragging the slider by hand, on a release build (`pnpm app:build --no-bundle`). Color handling (sRGB and Display P3 tagged frames, step 5) is also open.
