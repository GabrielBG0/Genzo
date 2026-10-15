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

**Decodes: 15 of 18 files**, with white and black levels, a camera matrix (D65 and A, or D50 for Leica) and white balance present in all but one. The exception is the Kodak DCS760C, which has no WB coefficients.

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
- Nikon HE is the one mainstream gap. Whether LibRaw can read it is step 3 and is not checked yet.


## 0.E Pixels through the webview

The tile protocol, the `genzo-frame` URI scheme, the WebGL2 viewport and the in-app latency probe exist from the M0 foundation. The app's Exposure and Temp sliders now drive the real GPU pipeline (`renderSpike`), sending interactive renders while dragging and one settled render on release. **End-to-end numbers per OS are not measured yet.**
