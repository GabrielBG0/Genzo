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
- **First pixels (since 2026-10-25, LIB-09's preview on a switch):** from the switch call to its embedded preview reaching the engine's preview sink, upright and in the frame's target. On a switch to a photo not prefetched, it is read from the file while the photo decodes beside it. On a switch to a neighbor, it is the preview the prefetch kept.

**Results** (p95; p50 in brackets):

| Target | Budget | 12 MP | 48 MP |
|---|---|---|---|
| Switch to a prefetched neighbor | under 150 ms | 19.2 ms (18.4) | 99.3 ms (87.3) |
| Full fit-view render, not prefetched | under 1 s (12 MP), under 2 s (48 MP) | 76 ms (71) | 1315 ms (1210) |
| First pixels: the file read and its embedded preview decoded | under 100 ms | 6.7 ms (6.2) | 23.2 ms (17.7) |

Samples: 12 switches, 9 fit renders and 60 previews per folder.

**With the preview shown on a switch** (2026-10-25, same setup; p95, p50 in brackets):

| Target | Budget | 12 MP | 48 MP |
|---|---|---|---|
| First pixels, a switch to a photo not prefetched | under 100 ms | 30.1 ms (25.9) | 71.8 ms (56.4) |
| First pixels, a switch to a prefetched neighbor | under 100 ms | 0.0 ms | 0.0 ms |
| Switch to a prefetched neighbor | under 150 ms | 20.1 ms (18.5) | 84.0 ms (67.7) |
| Full fit-view render, not prefetched | under 1 s, under 2 s | 81 ms (76) | 1404 ms (1165) |

Samples: 9 cold switches and 12 neighbor switches per folder. The cold first pixels take longer than the preview's decode alone (the table above), because the preview read and the photo's decode now run at the same time and each reads the whole file.

**These frames were stretched.** Until 2026-10-26 a Fit frame had the viewport's size, so these 4:3 photos rendered at 2560 × 1440. That made the 12 MP switch provisional: level 1 (2016 × 1512) was narrower than the 2560 px frame, and the frame from level 0 followed at 62 ms p95.

**With the frame fitted into the viewport** (2026-10-26, same setup, three runs at 48 MP; p95, p50 in brackets). The frame is now 1920 × 1440, which level 1 covers at 12 MP, so no switch is provisional:

| Target | Budget | 12 MP | 48 MP |
|---|---|---|---|
| Switch to a prefetched neighbor | under 150 ms | 26.8 ms (23.3) | 58.1 to 84.8 ms (49.3 to 73.4) |
| Full fit-view render, not prefetched | under 1 s, under 2 s | 77 ms (60) | 1305 to 1495 ms (1271 to 1344) |
| First pixels, a switch to a photo not prefetched | under 100 ms | 41.7 ms (27.6) | **1206 to 1210 ms** (65.1 to 73.4) |
| First pixels, a switch to a prefetched neighbor | under 100 ms | 0.0 ms | 0.0 ms |

**The 48 MP cold first pixels miss the budget once in 9 samples, in every run.** The cause is not the Fit change:
- `preview::prepare` converts the preview with rayon. On a cold switch it runs on the switching thread while the photo's decode fills rayon's pool, so its work can queue behind the decode's, for about 1.2 s.
- Run sequentially, `prepare` takes 433 ms for this file's preview. Run in parallel, it normally takes about 60 ms.
- The 2026-10-25 run above had no such sample, but the code was the same, so that was luck.
- The fix comes next, in its own change.

**Result:** every budget measured here holds on the reference desktop, except the 48 MP cold first pixels' outlier above.

**Room left:**
- **The 48 MP switch is the closest:** 99 of 150 ms. Uploading the 98 MB level 1 dominates; DEV-01 measured the GPU part of the switch alone at 22.5 ms.
- **The 48 MP first pixels read the whole 28 MB file** before the preview is found. Reading only the head would cut that, if the budget gets tight.
- **A cold switch reads the file twice**, once for its preview and once for its decode, side by side: 72 ms of 100 at 48 MP. Reading it once for both, or the preview first, would cut that.

**Still open:**
- **The bench runs by hand.** `scripts/gpu-check.sh` doesn't run it yet, since it needs corpus copies. CI only runs a smoke test on small PNGs (`cli/tests/bench_open.rs`).
- **The other machines** (macOS, the Linux laptop) are not measured.
