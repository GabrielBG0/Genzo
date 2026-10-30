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
- **First pixels (since 2026-10-25, LIB-09's preview on a switch):** from the switch call to its embedded preview reaching the engine's preview sink, upright and in the frame's target. On a switch to a photo not prefetched, it is read from the file, beside the photo's decode until 2026-10-30 and before it since. On a switch to a neighbor, it is the preview the prefetch kept.

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

**With the preview prepared on its own pool** (2026-10-30, same setup, three runs at 48 MP, one at 12 MP; p95, p50 in brackets). `prepare` now runs on a rayon pool of its own, one thread per core. It still shares the cores with the decode, but its work no longer waits for the decode's to drain. That holds for a neighbor's decode in prefetch too, which `tests/preview_pool.rs` checks along with the switch's own. Only the switch uses that pool: the prefetch worker prepares the previews it keeps on the global pool, so a switch never waits behind one of them:

| Target | Budget | 12 MP | 48 MP |
|---|---|---|---|
| First pixels, a switch to a photo not prefetched | under 100 ms | 28.0 ms (15.5) | 67.5 to 98.8 ms (54.2 to 63.4) |
| First pixels, a switch to a prefetched neighbor | under 100 ms | 0.0 ms | 0.0 ms |
| Switch to a prefetched neighbor | under 150 ms | 19.6 ms (16.9) | 73.4 to 83.5 ms (55.1 to 60.7) |
| Full fit-view render, not prefetched | under 1 s, under 2 s | 71 ms (56) | 1254 to 1335 ms (1185 to 1198) |

No cold sample of the 27 waited for the decode. One run's p95 of 98.8 ms is within 2 ms of the budget, so the 48 MP cold first pixels are the tightest number here.

**The 48 MP preview in every table above was the wrong image** (found 2026-10-30). The iPhone ProRAW file stores semantic mattes (sky, skin, hair) as grayscale JPEGs beside its preview, and `imgio` took the 2016 × 1512 sky matte for the preview, since it is the smallest stream big enough. So the 48 MP first pixels above timed a matte, as did the 433 ms below. The iPhone 12 Pro file in the corpus had the same fault; no other corpus file did. `imgio` now tries color streams before grayscale ones. The file's real preview is its full-size 8064 × 6048 JPEG; it has no smaller one.

**With the real preview** (2026-10-30, same setup, one run at 48 MP; p95, p50 in brackets):

| Target | Budget | 48 MP |
|---|---|---|
| First pixels, a switch to a photo not prefetched | under 100 ms | **2834 ms** (2794) |
| First pixels, a switch to a prefetched neighbor | under 100 ms | **none shown**: no samples |
| Switch to a prefetched neighbor | under 150 ms | 87.8 ms (82.8) |
| Full fit-view render, not prefetched | under 2 s | **3198 ms** (3142) |

- **The decode of the preview alone takes about 270 ms** (zune-jpeg, 8064 × 6048).
- **`prepare` converts every one of its 48 million pixels:** about 2 s into sRGB, where the conversion compresses the Display P3 gamut, and 550 ms into Display P3. The full-size result is 195 MB of RGBA for a 1920 × 1440 frame.
- **The fit render slows from 1.3 s to 3.2 s** because that work shares the cores with the photo's decode.
- **No neighbor keeps a preview:** its decoded and prepared pixels, about 390 MB, are larger than the 128 MB the prefetch keeps for previews, so a switch to a neighbor shows none until its level 1 does.

**With the preview reduced to the Fit frame** (2026-10-30, same setup, three runs at 48 MP, one at 12 MP; p95, p50 in brackets). `preview::reduce` takes box means of the preview's codes down to the size the Fit frame shows it at (1920 × 1440 here), before the conversion. The conversion keeps the colors it converted per band of rows, so gamut compression runs once per color rather than once per pixel, with the same result. Prefetch keeps the reduced preview, about 22 MB with the prepared one, so neighbors keep theirs again:

| Target | Budget | 12 MP | 48 MP |
|---|---|---|---|
| First pixels, a switch to a photo not prefetched | under 100 ms | 31.1 ms (17.2) | **560 to 605 ms** (528 to 547) |
| First pixels, a switch to a prefetched neighbor | under 100 ms | 0.0 ms | 0.0 ms |
| Switch to a prefetched neighbor | under 150 ms | 19.2 ms (17.5) | 80.0 to 81.8 ms (72.5 to 77.6) |
| Full fit-view render, not prefetched | under 1 s, under 2 s | 73 ms (55) | 1187 to 1198 ms (1114 to 1121) |

That left the 48 MP cold first pixels at about 0.55 s against 100 ms. The floor was the preview's decode, about 270 ms for the 8064 × 6048 JPEG alone, slower beside the photo's decode. The owner chose to decode it in parallel bands (2026-10-30).

**With the preview decoded in bands** (2026-10-30, same setup, three runs at 48 MP, one at 12 MP; p95, p50 in brackets). Four changes, each measured in turn at 48 MP (cold first pixels, p95):

1. **The preview decodes in bands** (`imgio/src/bands.rs`): 207 to 253 ms. The ProRAW preview has a restart marker on every MCU row, so it is cut into one band per thread of the preview pool, each with a row more above and below for chroma upsampling, then trimmed. The result is the same bytes as one decode (tested on every corpus preview that can be cut, and fuzzed). Alone, on an idle CPU, it takes about 40 ms instead of 270. The switch now reads its preview on the preview pool, so the bands never wait behind a neighbor's decode on the global pool.
2. **The conversion is cheaper** (`color::output`): 118 to 136 ms. Each target's matrices are derived once, not six inversions per color, and the search for the gamut's edge stops as soon as the color is known to be kept, which most are. Both give the same values as before: every 8-bit Display P3 and Adobe RGB code is checked against the full search. The 1440 × 1080 conversion went from 52 to 23 ms.
3. **The preview is read before the photo's decode starts, not beside it:** 104 to 108 ms. Beside it, the two shared the cores. The decode now starts once the preview is shown, about 90 ms later at 48 MP, 9 ms at 12 MP, and about 10 ms on a file with no preview (its read and the scan for one).
4. **The full-size decoded preview (195 MB) is freed after the preview is shown**, not before: 90 ms.

| Target | Budget | 12 MP | 48 MP |
|---|---|---|---|
| First pixels, a switch to a photo not prefetched | under 100 ms | 9.2 ms (8.8) | 89.9 to 90.6 ms (84.8 to 88.5) |
| First pixels, a switch to a prefetched neighbor | under 100 ms | 0.0 ms | 0.0 ms |
| Switch to a prefetched neighbor | under 150 ms | 16.9 ms (15.8) | 80.2 to 86.7 ms (71.0 to 76.0) |
| Full fit-view render, not prefetched | under 1 s, under 2 s | 62 ms (59) | 1243 to 1307 ms (1142 to 1168) |

**Result:** every budget holds on the reference desktop. The 48 MP cold first pixels are the closest, at 90 of 100 ms.

**Room left:**
- **The 48 MP switch is the closest:** 99 of 150 ms. Uploading the 98 MB level 1 dominates; DEV-01 measured the GPU part of the switch alone at 22.5 ms.
- **The 48 MP first pixels read the whole 28 MB file** before the preview is found. Reading only the head would cut that, if the budget gets tight.
- **A cold switch reads the file twice**, once for its preview and once for its decode. The second read is about 6 ms from the OS cache.
- **The 48 MP cold first pixels, at 90 ms,** are, on an idle CPU, about 8 ms reading the file, 40 ms decoding the preview in bands, 7 ms reducing it and 23 ms converting it. If the budget gets tight, the next step is to reduce each band as it is decoded, so the 195 MB full-size preview is never held. A disk tier of small previews (plan step 8) helps only photos seen before.

**Still open:**
- **The bench runs by hand.** `scripts/gpu-check.sh` doesn't run it yet, since it needs corpus copies. CI only runs a smoke test on small PNGs (`cli/tests/bench_open.rs`).
- **The other machines** (macOS, the Linux laptop) are not measured.
