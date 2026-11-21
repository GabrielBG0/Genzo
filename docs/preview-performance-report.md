# Real-time full-resolution preview: performance report

2026-11-14. A review of the engine, the shell and the frontend's drag path, written after the owner found the lower-resolution preview during slider drags unacceptable. The same report is a Claude Docs document, *Genzo: Real-Time Full-Resolution Preview — Performance Report*.

Nothing in this report was newly measured. Numbers come from `docs/m0-results.md`, `docs/specs/DEV-01.md` and `benchmarks/baseline-rtx3060.json`; estimates are marked as estimates.

## Summary

The soft preview while dragging is a transport workaround, not a rendering limit. The GPU already renders every drag frame at full viewport resolution in 0.7 to 5 ms on the RTX 3060. The engine then discards three quarters of those pixels and JPEG-compresses the rest, because moving a full frame into the webview is slow.

- **The bottleneck is the pixel path into the webview.** WebView2 reads the `genzo-frame` scheme at about 120 MB/s. A lossless 2560 px frame is 11 MB, so the fetch alone took 83 to 91 ms and the whole drag step 197 to 236 ms in M0. The half-size JPEG preview brought that to 18 ms, at the cost the owner saw.
- **The webview is fine for the panels.** That same 18 ms figure includes the slider, the IPC call, the event and the draw. Svelte is not what is slow, so a full frontend rewrite would not buy anything.
- **Recommended fix: a native wgpu surface for the photo, under a transparent webview.** The render thread presents the frame it already has on the GPU. No readback, no JPEG, no fetch, no decode. Drag frames become lossless and full resolution, at the display's refresh rate, at Fit and at 1:1. This is the plan's own "ladder step 3".
- **It needs a spec change.** The spec routes all pixels through the tile protocol and leaves monitor color conversion to the webview. A native surface changes both.
- **A stopgap exists** for sharper drags before that lands: full-size JPEG bands instead of half-size. It is lossy and runs at roughly 25 to 30 fps on Windows.

## How a drag frame travels today

A slider move becomes pixels in nine steps, and the GPU work is the cheapest of them. At Fit, "full resolution" means the viewport's resolution (for example 1920 x 1440 for a 4:3 photo in a 2560 x 1440 view), not the sensor's.

| Step | Today, through the webview | With a native surface |
| --- | --- | --- |
| 1 | Slider moves; waits for an animation frame to send | Slider moves; sends on pointer move |
| 2 | `invoke` to the engine; answers only after render, readback and encode | `invoke` to the engine; answers at once |
| 3 | GPU renders the edit at full view size, 0.7 to 5 ms | The same |
| 4 | **Stage 9 at half size**: three quarters of the pixels dropped | Present to the native surface: full size, lossless, next display refresh |
| 5 | **Readback to the CPU**: blocking; 12 ms if the frame were full size | |
| 6 | **JPEG encode, quality 80**: 2.4 to 3.6 ms at half size | |
| 7 | **Event, then fetch over `genzo-frame`**: about 120 MB/s on WebView2 | |
| 8 | **Decode and WebGL upload**: 3.5 to 5.3 ms at half size | |
| 9 | Canvas draw; waits for the next animation frame | |

The GPU renders at full view size in both paths. Today the five steps in bold shrink, compress and carry that frame into the page; with a surface the frame never leaves the GPU.

## Findings

Nine places stand between a slider move and a full-resolution frame. Items 1 and 2 are what the user sees; items 3 to 6 are why they exist; items 7 to 9 are smaller costs that become the floor once the transport is gone. Paths are under `backend/crates/` or `frontend/src/lib/` unless they start with `shell/`.

| # | Where | What happens | Cost |
| --- | --- | --- | --- |
| 1 | `pipeline/src/gpu/develop.rs:626`, `view.rs:854` | During a gesture at Fit, stage 9 writes a half-size frame (`half = interactive && counting`). Sampling, tone and color still run at full view size | Three quarters of the rendered pixels are discarded |
| 2 | `engine/src/frames.rs:54` and `:62`, `engine/src/lib.rs:60` | Drag frames are JPEG, quality 80: bands at Fit, per tile at 1:1 | Lossy at both zooms. Encode 2.4 to 3.6 ms at half size, 9 to 14 ms at full size (4 bands) |
| 3 | `pipeline/src/gpu/readback.rs:185`, `view.rs:1002` | The frame is copied GPU to CPU with a blocking `poll(wait_indefinitely)`, then copied twice more into `Vec`s | Stage 9 plus readback: 12.0 ms for a full 2560 x 1920 frame, 3.2 ms at half size |
| 4 | `engine/src/frames.rs:92`, `engine/src/lib.rs:421` | A settled frame is copied again into 256 px tiles, and the stored message is cloned for every fetch | Two more copies of 11 to 20 MB per frame |
| 5 | `shell/src/main.rs:145`, `transport/tauri.ts:49` | The frontend fetches the bytes over the `genzo-frame` URI scheme | About 120 MB/s on WebView2: 83 to 91 ms for an 11.1 MB frame. The largest single cost |
| 6 | `viewport/frame.ts:14`, `viewport/webgl.ts:144` | Each JPEG band is copied, decoded with `createImageBitmap` and uploaded with `texSubImage2D` | 3.5 to 5.3 ms at half size; 16.7 ms for one full 2560 x 1080 JPEG |
| 7 | `engine/src/develop.rs:561` and `:574`, `components/PhotoViewport.svelte:73`, `develop/session.svelte.ts:301` and `:464` | One frame takes three webview hops: `invoke` (answers only after render, readback and encode), the `renderFinished` event, then the fetch. Sends wait for an animation frame, draws wait for another, and one command is in flight at a time | Frame rate is one over the round trip. Up to two display frames of added wait |
| 8 | `pipeline/src/gpu/view.rs:339`, `timer.rs:91`, `thread.rs:1060` | Every drag frame also does a blocking histogram readback, a blocking timer readback and three error-scope pops | Engine time is 5.2 ms per exposure drag frame against 0.73 ms of GPU time |
| 9 | `engine/src/frames.rs:59` | On release the full lossless frame takes the same route as item 5 | DEV-01 says it is swapped in about 150 ms after the gesture ends: the visible sharpen |

One thing is not a resolution problem: a white balance drag applies a ratio to the cached demosaic and re-runs the demosaic on release (`pipeline/src/gpu/develop.rs:585`). At Fit it matches the settled render within a mean ΔE2000 of 0.1, so it can stay.

## The webview question

The webview is the bottleneck for photo pixels and only for them. Keep it for the interface, and stop sending the photo through it.

| Option | Drag frame at Fit | Expected input to pixels at 2560 px | Work and risk |
| --- | --- | --- | --- |
| A. Today: half-size JPEG through the webview | Half resolution, lossy | 18 ms p95 measured on a synthetic scene; 19 to 26 ms estimated for photos | None |
| B. Full-size JPEG bands through the webview | Full resolution, lossy | About 39 ms per frame, estimated in M0 from measured parts | Small: a constant and a branch. No spec change beyond DEV-01's Interactive section |
| C. A faster byte channel into the webview (shared memory on WebView2, or a local socket) | Full resolution, lossless | Not measured. Readback (12 ms) and the WebGL upload remain, so 60 fps is unlikely | Medium, different per OS. What Tauri exposes here is not verified |
| D. Native wgpu surface under a transparent webview | Full resolution, lossless | GPU time plus one display refresh | Medium to large. Spec change. Risks below |
| E. Replace the frontend with a native toolkit | Same as D | Same as D | Rewrite of every screen, component and frontend test. No gain over D |

**Recommendation: D**, with B as an optional stopgap. E solves the same problem as D at many times the cost, because the panels were never slow.

### How D works

1. The shell passes the window's handle to the engine. The render thread creates a wgpu surface on it and presents there. It already owns the device, so the one-render-thread rule holds.
2. Stage 9 always runs at full size. A small present pass draws its frame texture into the swapchain, placed by zoom and pan uniforms.
3. The webview becomes transparent where the photo is. `PhotoViewport.svelte` keeps its toolbar, crop overlay and badges as HTML on top, and reports its rectangle in device pixels, which it already does for `SetView`.
4. The histogram keeps its small readback, made non-blocking.
5. The tile protocol stays for the CLI, export, thumbnails, the fake engine, the Playwright tests, and as the fallback where a surface cannot be used.

### Risks of D

- **Color management.** Today the webview converts each frame to the monitor profile. With a native surface the app must do it: probably a monitor-profile transform in the present pass on Windows, and a tagged layer on macOS. This is the main design work and needs its own spec section and a color probe run.
- **Device creation order.** The device opens before any window exists (`Engine::new` in the shell's `setup`). The adapter must be chosen compatible with the surface, so startup order changes.
- **Transparency per OS.** WebView2 supports a transparent background. macOS needs Tauri's private-API flag for it, as far as known. WebKitGTK is unverified; Linux can stay on the tile path, since the plan ranks it lower.
- **Architecture rules.** The backend would depend on `raw-window-handle`, a handle type and not a UI toolkit, but `scripts/` may need to allow it. The "pixels use the binary tile protocol" rule needs a second sanctioned route.
- **Layout sync.** While a panel is resized, the photo's rectangle can trail the DOM by a frame.
- **Testing.** Playwright cannot see native pixels. End-to-end tests stay on the fake engine and WebGL; the native path is checked by reading back the presented texture against the stage 9 goldens.

A one to two day spike on Windows should come first, in the style of M0's 0.E: a transparent webview over a wgpu surface in the shell, driven by the existing autobench. Tauri has had a wgpu example for this pattern; check its current state before starting.

## Recommended fixes

Do the spike first, then build the native surface on Windows, then clean up what it exposes. Effort is a rough size, not an estimate in days.

| Order | Change | Removes findings | Expected gain | Effort | Risk |
| --- | --- | --- | --- | --- | --- |
| 1 | Spec change and a Windows spike: transparent webview over a wgpu surface, measured with the autobench | none yet | Proves or kills option D before any real work | Small | Low |
| 2 | Optional stopgap: full-size JPEG at Fit drags. Drop the `half` branch, raise `PROXY_MAX_BANDS` to the core count, raise quality | 1 | Full resolution now, still lossy, about 25 to 30 fps on Windows | Small | Lower frame rate than today |
| 3 | Native surface on Windows: surface owned by the render thread, present pass, transparent viewport, rectangle sent with `SetView` | 1, 2, 3, 4, 5, 6, 9 | Lossless full-resolution drags at the refresh rate, at Fit and 1:1. No sharpen after release | Large | Startup order, layout sync |
| 4 | Monitor color conversion for the surface | none | Keeps the spec's color policy true without the webview | Medium to large | The main unknown. Needs a color probe run |
| 5 | Take blocking readbacks off the drag path: histogram mapped without waiting and throttled, GPU timer read only when benchmarking, error scopes measured and trimmed | 8 | Engine time per drag frame from 5.2 ms toward the 0.73 ms of GPU time | Small to medium | Low |
| 6 | Loosen the drag loop: send updates on pointer move, and answer an interactive delta before its render finishes | 7 | Removes up to two display frames of waiting. Frame rate no longer tied to the IPC round trip | Medium | Contract bump. The session's ordering rules rely on replies today |
| 7 | macOS surface. Decide Linux: surface, or stay on tiles | same as 3 | Same as 3 on those systems | Medium | WebKitGTK transparency unverified |
| 8 | Keep the JPEG path only as the fallback (software adapter, no surface) | none | Less code on the hot path | Small | Low |

Three notes on the order:

- **Step 6 only pays off after step 3.** Today the render thread already drops stale requests (`pipeline/src/gpu/thread.rs:806`), but the webview fetch paces everything, so sending faster gains nothing yet.
- **Step 2 is the owner's call.** It gives full resolution within a day and costs smoothness. If the spike in step 1 goes well, skipping it is reasonable.
- **Step 3 changes a budget.** "Slider input to pixels under 50 ms p95" was set for the webview path. With a surface, a budget of two display refreshes is realistic and worth gating.

## Other snappiness issues

Five smaller things outside the Fit drag, in rough order of how much a user would feel them. These were read less closely than the drag path.

- **Panning at 1:1.** Each pan step is a `SetView` round trip that renders settled and sends lossless tiles through the same slow scheme (`engine/src/develop.rs:249`). With a surface, a pan is a present.
- **The sharpen after release.** About 150 ms after every gesture the soft frame is replaced by the lossless one. It goes away with the surface.
- **The histogram during a drag** rides on each frame's blocking readback. Decoupled (fix 5), it can update at its own rate without slowing frames.
- **Queries on the main thread.** `query` in `shell/src/main.rs:36` is not `async`, so Tauri runs it on the main thread, where it takes the session lock. Worth checking that no query can wait behind a long engine operation.
- **First open of ProRAW folders.** `docs/m1-results.md` records about 3 s per 20 thumbnails for 48 MP ProRAW, whose only preview is full size, and a cold 48 MP first-pixels p95 of 98.8 ms against a 100 ms budget. Neither is on the slider path.

## How to measure

The existing tools cover most of this; one gap must be closed before a surface can be gated.

| Metric | Today | Target with the surface | Tool |
| --- | --- | --- | --- |
| Drag frame at Fit | Half size, JPEG quality 80 | Full size, lossless | Read back the presented texture against the stage 9 goldens |
| Slider input to pixels, p95 at 2560 px | 18.4 ms (synthetic scene, Windows) | No worse than today, within two display refreshes | In-app autobench (`GENZO_AUTOBENCH`) |
| Engine time per exposure drag frame | 5.2 ms p50 | Near the GPU's 0.73 ms | `cli bench gpu`, `scripts/gpu-check.sh` |
| Release to settled frame | About 150 ms | One display refresh | In-app autobench, new case |

- **The gap:** the autobench times a frame at the webview's draw callback (`components/PhotoViewport.svelte:73`). A native present never reaches that callback, so the engine has to report the present time itself.
- **Still unmeasured from M0:** drag latency on real photographs inside the app. The in-app bench opens a synthetic RAW. Add a real-file case before comparing options.

### Not verified in this review

- No benchmarks were run and the app was not launched. Every number is from the repository's own records.
- The 39 ms figure for full-size JPEG is M0's estimate from measured parts, not an end-to-end run.
- Option D's behavior on WebKitGTK, the macOS transparency flag, and the Tauri wgpu example are from memory, not checked against current documentation.
- How Windows should convert to the monitor profile on a native surface is an open design question.
- The split of the 4.5 ms between engine and GPU time (finding 8) across readback, encode, histogram and error scopes is not measured.
