# M1 results

Measured results of M1 work, against the budget table in [development-plan.md](development-plan.md#m1-develop-core). Each section lists what was measured, on what, and what is still open.

## 48 MP ProRAW decode on the reference desktop (2026-10-18)

Plan, backend step 2: measure the parallel decode of a 48 MP JPEG XL ProRAW before building on it, since the fit-view budget (under 2 s at 48 MP) was not proven.

**Setup:**
- **Machine:** the owner's desktop, the M1 reference class. RTX 3060 12 GB, 16 threads, Windows 11.
- **File:** the iPhone 16 Pro file `IPhone16proRaw.DNG` (8064 × 6048, 12 tiles of 2016 px in a 4 × 3 grid, lossy JPEG XL).
- **Command:** `cargo run --release -p genzo-imgio --example dng_jxl_probe -- FILE`, five runs, with the file read before timing.

| Reader | Decode, 48 MP |
|---|---|
| `dng::decode_linear`, one row of tiles per thread | 911 to 957 ms |

**Result:** the decode fits the budget. It leaves about 1 s for upload, stages 1 to 4 and the first frame. In M0, the settled render at 48 MP took 22 ms of GPU time and 31 ms in the engine on this GPU.

**Room left.** The reader runs one row of tiles per thread, so this file keeps only 3 of the 16 threads busy. Decoding tile by tile is the next step if the budget gets tight. On the owner's 4-core laptop the same reader took 4.8 to 5.4 s (M0), so laptops will miss the 2 s fit-view target at 48 MP. The budget is set for the reference desktop.

**Still open:** the second 16 Pro file (`IMG_6889`, on the owner's Linux machine), and peak memory on Windows (the probe reads it only on Linux).

## Opening a folder in Develop on the reference desktop (2026-10-25)

The photo-open budgets, and LIB-09's criterion 5: switching to a prefetched neighbor renders at Fit without decoding, under 150 ms.

**Setup:**
- **Machine:** the owner's desktop, RTX 3060 12 GB (Vulkan), 16 threads, Windows 11, release build.
- **Folders:** 20 copies each of two corpus files, so every switch has its neighbors. Copies are separate photos to the engine (ids by path), so prefetch treats them as any other.
  - **48 MP:** `IPhone16proRaw.DNG` (8064 × 6048, JPEG XL ProRAW).
  - **12 MP:** `PXL_20201121_100251397.dng` (Pixel 4a, 4032 × 3024 Bayer DNG).
- **Command:** `genzo-cli bench open FOLDER 2560 1440 3`: a 2560 × 1440 view at Fit, three passes, each with a new engine, so nothing is cached. Each pass:
  - waits for the prefetch, then moves one photo right to a prefetched neighbor;
  - then jumps four right to a photo that isn't one, which is decoded, opened and rendered;
  - and repeats.
- The first pixels are timed once the prefetch is idle, so the neighbors' decodes don't share the CPU with them.

**Results** (p95; p50 in brackets):

| Target | Budget | 12 MP | 48 MP |
|---|---|---|---|
| Switch to a prefetched neighbor | under 150 ms | 19.2 ms (18.4) | 99.3 ms (87.3) |
| Full fit-view render, not prefetched | under 1 s (12 MP), under 2 s (48 MP) | 76 ms (71) | 1315 ms (1210) |
| First pixels: the file read and its embedded preview decoded | under 100 ms | 6.7 ms (6.2) | 23.2 ms (17.7) |

Samples: 12 switches, 9 fit renders and 60 previews per folder.

**The 12 MP switch is provisional at this view.** A Fit view reads level 1 only when level 1 covers the whole view on both sides (`sampling::plan_view`). Here the view is 2560 × 1440, and level 1 of the 4032 × 3024 photo is 2016 × 1512: narrower than the view, though wider than the 1920 × 1440 frame. So the switch draws its frame from level 1 in place of level 0. The frame from level 0 follows once the input is decoded and attached: at 60 ms p50, 62 ms p95, after the switch began. At 48 MP, level 1 (4032 × 3024) covers the view, so the switch is final.

**Result:** every budget measured here holds on the reference desktop.

**Room left:**
- **The 48 MP switch is the closest:** 99 of 150 ms. Uploading the 98 MB level 1 dominates; DEV-01 measured the GPU part of the switch alone at 22.5 ms.
- **The 48 MP first pixels read the whole 28 MB file** before the preview is found. Reading only the head would cut that, if the budget gets tight.

**Still open:**
- **The bench runs by hand.** `scripts/gpu-check.sh` doesn't run it yet, since it needs corpus copies. CI only runs a smoke test on small PNGs (`cli/tests/bench_open.rs`).
- **The embedded preview isn't shown on a switch yet** (LIB-09, after this). "First pixels" measures its decode, not its display.
- **The other machines** (macOS, the Linux laptop) are not measured.
