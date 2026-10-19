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
