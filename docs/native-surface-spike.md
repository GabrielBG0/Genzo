# Native surface spike (option D): plan and kill criteria

Decided 2026-11-21 (owner): the photo moves to a native wgpu surface under a transparent webview (`docs/preview-performance-report.md`, option D). This file is step 1 of that report's order: a Windows spike that proves or kills D before any real work. The criteria below were written before anything was run.

Style: M0's 0.E. The spike is an experiment on its own branch, behind a flag, with no contract change. It leaves a results section at the bottom of this file.

## What has to be true

D stacks two things in one window: a wgpu swapchain, and a WebView2 whose transparent areas show it. wry hosts WebView2 in a **child HWND** of the tao window (`WS_CHILD | WS_CLIPCHILDREN`, `CreateCoreWebView2Controller(hwnd)`) with a default background of alpha 0 when `transparent` is set (read in wry 0.57 `webview2/mod.rs`). Whether the parent's swapchain shows through that child is not documented and is what the probe decides.

## Probes, in order

Nothing later starts until the earlier one passes.

1. **Occlusion probe.** A tao + wry window (the same hosting Tauri uses), `transparent`, with a wgpu surface on the window's HWND cleared to a known color, and a page with a transparent hole and opaque HTML around and over it.
   - Pass: the clear color shows through the hole, HTML drawn over it stays on top and is clickable, and the area outside the hole is the page's.
   - If it fails, try in this order: (a) a sibling child HWND created by the shell, z-ordered below the webview; (b) a borderless owned window glued behind the main one; (c) the surface above the webview as a click-through child, with the toolbar and overlays redrawn natively. If none passes, D is dead and the owner decides again between E and the stopgap.
2. **Present path.** The engine's render thread (not the probe's) creates the surface from a `RawWindowHandle` and presents a real stage 9 frame, full size, at Fit.
3. **Resize, pan, DPI.** Measured on the probe and then in the real Tauri shell.
4. **Latency and color.** As below.

## Kill criteria

D is **abandoned** (and the owner chooses again) if any of these holds after the fallbacks of probe 1 have been tried:

| # | Criterion | Why it kills |
| --- | --- | --- |
| K1 | No arrangement shows the surface through the webview's hole with HTML on top, at the display's refresh rate | The whole design |
| K2 | Dragging a panel or resizing the window flickers, shows a gap, or lets the photo's rectangle lag the DOM by more than one display frame | Visible on every layout change |
| K3 | Moving the window between two monitors of different scale leaves the surface mis-sized or blurred for more than one frame | Pros use two monitors |
| K4 | An exposure drag at Fit on the RTX 3060 takes more than 2 display refreshes (33 ms at 60 Hz) from the engine receiving the step to the present, p95 | It would be no better than B |
| K5 | A drag frame is not lossless full view size | It would not fix what the owner saw |
| K6 | The same run under `GENZO_ADAPTER=software` does not at least show the photo correctly | The fallback must stay usable |
| K7 | Teardown (closing the window, or a device loss) hangs or crashes | Unacceptable in a shipped app |

**Color is not a kill criterion; "a viable path exists" is.** The report's open question: how Windows converts to the monitor profile when the app presents. To settle it, read what wgpu 30 sets as the DXGI color space for the surface formats it offers on DX12, and check on this machine whether Windows 11's automatic color management converts an `Rgba16Float` (scRGB) swapchain. If neither works, a monitor-profile transform in the present pass is the path, and it is step 4 of the report's order, not part of the spike.

## Out of scope for the spike

Startup order (the adapter is chosen compatible with the surface: asserted, not reordered), macOS and Linux, 1:1 tiles, the histogram, the real viewport component, and any contract change. The flag is `GENZO_NATIVE_SURFACE=1`; without it nothing changes.

## Results

### Probe 1, occlusion (2026-11-21, RTX 3060, DX12, Windows 11)

Probe: `spikes/surface-probe` (tao 0.37.1, wry 0.57.0, wgpu 30.0.1, the versions in `shell/Cargo.lock`). A surface on the tao window's HWND is cleared to a cycling color; the page is opaque except one hole, with an opaque button and a half-transparent box over the hole. Judged from screenshots.

A first pair of runs proved nothing: the page had a full-window opaque div under the hole, and the first reading of those screenshots was wrong. The page was fixed and the modes rerun:

| tao window | wry webview | Hole shows |
| --- | --- | --- |
| `transparent` | `transparent` | White. The swapchain is not visible |
| opaque | `transparent` | **The swapchain's clear color. The opaque button and the half-transparent box draw over it correctly** |
| opaque | `transparent: false`, background color alpha 0 | White. The alpha-0 color alone does not make the page transparent |

**Result: probe 1 passes in the second arrangement.** The surface on the parent HWND shows through a transparent WebView2 child, with HTML composited over it, as long as the window itself is not transparent (tao's `transparent` also sets DWM blur-behind, which is the likely cause of the white).

**Open, and the next thing to settle:** Tauri's `WebviewWindowBuilder::transparent` sets the window's and the webview's flag together (`webview_window.rs:1146`), which is the failing first row. The two can be set apart only through `WindowBuilder` plus `WebviewBuilder` (`add_child`), behind Tauri's `unstable` feature, or by patching. So the next probe is the same test in the real shell. If that route is not workable, the fallbacks of probe 1 stay in order.

**Superseded by probe 1b below.** Not yet measured in the wry probe: frame rate of the probe (the fps lines did not reach the log), a real click on the button over the hole (it draws on top; input was not driven), resize, DPI. K1 is therefore not yet cleared for Tauri, only for wry.

**Gotcha:** `0x800700AA` ("resource in use") from WebView2 creation appears when a stale profile from a killed run is reused; `WEBVIEW2_USER_DATA_FOLDER` to a fresh folder avoids it.


### Probe 1b, the same through Tauri (2026-11-21)

Probe: `spikes/tauri-surface-probe` (tauri 2.12.3 with its `unstable` feature, the shell's `Cargo.lock`). `WindowBuilder` makes an opaque window; `Window::add_child` adds a `WebviewBuilder` with `transparent(true)`; a thread creates a wgpu surface on the `Window` and clears it every frame.

- **K1: cleared for Tauri on Windows.** The hole shows the surface; the opaque button and the half-transparent box draw over it; a real mouse click on the button over the hole reached the page (`clicked 1`). The route is Tauri's `unstable` window/webview split, not `WebviewWindowBuilder::transparent`.
- **Resize:** at 1500 x 900, 700 x 500 and 1100 x 1000 the surface and the hole followed the window (`auto_resize` on the webview, the surface reconfigured from the window's size each frame).
- **Open, K2:** in the two captures after shrinking, a black window-shaped remnant stayed beside the window. It may be an artifact of capturing with `CopyFromScreen`, or a stale-paint problem on shrink. Settling it needs a better capture (or a frame-by-frame recording during a live drag-resize), not a guess. It decides K2.
- **Frame rate:** the loop ran at about 3000 fps because the default present mode did not wait for vsync. That says nothing about the latency criterion. Present mode (Fifo or Mailbox) is chosen in the engine.
- **Cost:** the `unstable` feature is a dependency on a feature Tauri does not promise to keep. Keep the use small and behind one function in the shell.

**Still to do for the spike:** K2 on a live drag-resize; K3 (two monitors with different scales); the engine's render thread making the surface from a `RawWindowHandle` and presenting a real stage 9 frame (K4, K5, K6); teardown order on close (K7); the color path.

### Probe 2, the real render thread presenting to the window (2026-11-21)

Code: `pipeline::gpu::surface::NativeSurface` (a present pass that unpacks stage 9's `R32Uint` frame into a plain 8-bit swapchain format), `RenderThread::attach_surface`, `detach_surface` and `layout_surface`, `Develop::set_native` (no half-size preview) and `DevelopFrame::presented`. The surface is made through wgpu's safe `SurfaceTarget`, from the Tauri `Window` itself, so the backend has no unsafe code and no new dependency; `Gpu` now keeps its `Instance` and `Adapter`. Harness: `spikes/tauri-surface-probe` (`HARNESS=native|readback`, `VIEW=WxH`, `PHOTO_MP`, `GENZO_PRESENT`). A 24 MP (and a 48 MP) synthetic RAW, exposure dragged for 300 steps as `Phase::Update` requests, RTX 3060. A "step" is the time `RenderThread::develop` takes: render, the histogram's readback, and either the present or the frame's readback. It does **not** include the old path's JPEG encode, fetch or decode, which this harness does not run, so the two columns are not a like-for-like total; the old total is the report's 18 ms p95 in the app.

| Case (view 2560 x 1440, 3:2 photo) | Drag frame | Step p50 / p95 | Present: acquire, submit | Release frame |
| --- | --- | --- | --- | --- |
| Today: readback, no surface | 1080 x 720, **half size** | 1.26 / 1.71 ms | | 3.9 ms |
| Native, vsync (Fifo) | **2160 x 1440, full** | 5.55 / 6.19 ms | 4.9 (the wait for the display), 0.27 ms | 0.43 ms |
| Native, no vsync | 2160 x 1440, full | 1.34 / 2.57 ms | 0.01, 0.25 ms | 0.65 ms |
| Native, no vsync, 48 MP photo | 2160 x 1440, full | 1.30 / 2.49 ms | 0.01, 0.24 ms | 0.84 ms |
| Native on the software adapter (WARP), 940 x 680 | 940 x 627, full | 55.8 / 63.3 ms (GPU 55 ms) | | |

- **K4 cleared on this hardware.** The engine side of a full-size drag step is about 1.3 ms (2.6 ms p95), against the 33 ms allowed; with vsync the step is paced by the display, as it should be. What this does not measure is the display's own scanout, at most one refresh more, and the real app's input path.
- **K5 cleared.** The drag frame is the full Fit frame, never the half-size preview, and nothing compresses it: the assertion in the harness checks it on every run. The frame the swapchain shows is stage 9's own bytes.
- **No sharpen after release:** the settled frame needs 0.4 to 0.8 ms to present, and it is the same lossless frame as the drag's.
- **K6 cleared.** The software adapter (WARP) presents the same frame to the window and the hole shows it. It is 40 times slower there, GPU time, as before; the fallback is usable for the look, not for drags.
- **K7, partly.** `detach_surface`, then dropping the render thread, then closing the window, exits cleanly. Closing the window in the middle of a drag, with no detach, also exits with code 0 and no hang. The shell should still detach first, on the window's close request, as the safe order.
- **The render thread picks Vulkan on this machine** (the probes of 1 and 1b used DX12); both present to a Tauri window. WARP is on DX12.
- **Not yet done, and what is true of the code:** the histogram's blocking readback is still in the step (finding 8 of the report); `DevelopFrame::readback` carries no pixels when presented, which the engine does not handle yet (the engine is not wired); layout origin and size come from the harness's knowledge of the page, not from the frontend; 1:1 tiles were not run; no input pacing.

### Color: the mechanism on Windows (2026-11-21)

Question: does Windows convert a swapchain's colors for the display when the swapchain is tagged with a color space, so the present pass need not? Test: the same settled stage 9 frame, encoded once for sRGB and once for Display P3 (`Target`), is presented to a swapchain asked to be tagged `Auto`, `Srgb` or `DisplayP3`; the screen is captured (`CopyFromScreen`, per-monitor DPI aware) and eight points over the scene's patches are compared with the frame's own bytes read back from the same render. Both monitors, both backends (Vulkan and DX12), auto color management as found (the Samsung explicitly off; the LG has no value set, which I did not change).

Monitors, from their EDID: Samsung LF24T35 about sRGB (78 % of Display P3's area); LG UltraGear (GSM5C8A) a little wider than sRGB (108 % of sRGB, 80 % of Display P3's area). Neither is wide-gamut. No ICC profile is assigned to either display.

| | Result |
| --- | --- |
| Screen pixels against the frame's bytes | **Identical, max diff 0**, for every tag, both frame encodings, both monitors, both backends |
| Tags wgpu 30 offers on Windows (Vulkan and DX12) | `Srgb` for the 8-bit formats; `ExtendedSrgbLinear` for `Rgba16Float`; `Srgb` and `Bt2100Pq` for `Rgb10a2Unorm`. **No `DisplayP3`**: a P3 request falls back to `Auto` |
| Does the tag change what the capture shows | No |

So the OS does not convert a tagged 8-bit swapchain before the capture point, and cannot be asked for P3 at all. **What this does not show:** conversion that happens after composition (the display driver's tables, or Windows auto color management when it is on) would not appear in a screen capture; the monitors are not wide-gamut, so a P3-versus-sRGB difference would be small even if it were applied; and no colorimeter was used. It is the mechanism, not the accuracy.

**Consequence for the design (a recommendation, to be a spec decision):** the present pass converts to the monitor itself, from the display's ICC profile where one is assigned and from its EDID primaries otherwise, and writes the result in the swapchain's plain 8-bit format. That is the "monitor-profile transform in the present pass" of the report, and it is step 4 of the plan, a real piece of work with its own spec section, not something the swapchain tag does for us. It needs a wide-gamut monitor and, ideally, a colorimeter to verify.

**macOS differs:** Metal layers carry a color space and macOS converts from it to the display's profile itself (ColorSync); wgpu offers `DisplayP3` there. The Mac kit below measures that.

### A finding on startup: DX12's first frame

With `WGPU_BACKEND=dx12` the hole stayed white for about 8 s before the first photo frame, against about 1 s on Vulkan (same machine, same code): the first render compiles the shaders. Until the first present the hole shows the webview's white. Two things follow for the shell: the surface should be cleared to the pane's color as soon as it is attached (a present of nothing but the clear), and the first render should not wait for a user action. Which backend wgpu picks varies by machine (Vulkan on this NVIDIA one).

### API shape the macOS work forced (2026-11-21)

wgpu documents that on macOS surface creation panics off the main thread. So the render thread no longer creates the surface: `RenderThread::surface_maker()` returns a `SurfaceMaker` the shell carries to its main thread (setup runs there, or `run_on_main_thread`), where `SurfaceMaker::create(window.into())` makes the surface; `attach_surface` then takes it. That is the same on every OS.

### The macOS kit (2026-11-21)

`scripts/surface-kit/` (run script, README for the tester, `make-kit.sh`) and the `Surface kit` workflow build the probe on macos-15 and publish an unsigned pre-release for a tester with an M1 Mac. The run script records the system, then runs: a visibility case with a screenshot; step timings at 2560 x 1440 for a 24 MP and a 48 MP photo, with Mailbox and Fifo pacing and with readback for comparison; the frame's bytes for both encodings; the screen under every combination of encoding and color-space tag (`Auto`, `Srgb`, `DisplayP3`), each with a screenshot of the test window; and a 20 s moving-layout case. **None of this has run on a Mac.** Open on macOS: whether a Metal surface shows under a transparent webview at all (Tauri needs `macos-private-api` for a transparent webview there, set in the probe), and whether the tag then changes the pixels. The kit is built to report each stage's outcome, so a failure is also a result.

### K2, resize and layout drag (2026-11-21)

The render thread now repaints at once on a layout change (`SurfaceLayout`, from the frame it holds), and the harness follows the page the way the app will: the page reports its hole's rect over IPC, the harness lays the surface out and renders at the new view size. Tools: `RESIZE=1` (follow the window), `ANIM=1` with `PAGE=anim.html` (follow a hole whose left panel swings 200 to 360 px, 1.5 s a cycle, peak about 335 px/s).

- **Window resize, 67 steps** from 700 to 1500 px wide and back, captured 25 ms and 4 ms after each `SetWindowPos`: no uncovered or black pixels inside the window at any step (14 black samples of baseline, at most 30). The surface and the hole followed.
- **The photo trailing the DOM while a panel moves.** Measured as the strip of clear color between the panel's edge and the photo's first column, 500 captures per run:

| Present mode | Samples with a gap | Gap p95 / max | In display frames at peak speed |
| --- | --- | --- | --- |
| Fifo (vsync) | 210 of 500 | 7 / 8 px | about 1.4 |
| Mailbox | 71 of 500 | 2 / 2 px | under 0.5 |

  The gap shows only where the hole grows (the photo is late); where it shrinks the late frame hides under the panel. It includes the whole path: the page's report, the IPC, the render, the present and the compositor. **Under Fifo it exceeds K2's one display frame at the fastest part of the drag; under Mailbox it does not.** So the surface now presents with Mailbox by default (`GENZO_PRESENT=fifo` for vsync; Fifo is the fallback where Mailbox is not offered), and the area a stale frame leaves uncovered should be painted in the pane's color, not a contrasting one.
- **A black window-shaped block outside the window after a shrink** (the earlier unexplained remnant) is **not caused by D.** It appears with the surface not attached, with the child webview opaque, and with the ordinary `WebviewWindowBuilder` the shell uses today, and it is absent for a plain WinForms window under the same capture. Its position varies. It lasts at least 2 s. A Chrome video plays under the test window on this machine, which may be involved (overlay capture), but that is not established. To do before trusting either way: reproduce by hand-dragging the real app, with and without the video, and see if the display shows it, not only a `CopyFromScreen` capture. Until then it is a note about WebView2 windows on this machine, not a finding against D.
- Chromium's `Intermediate D3D Window` child stays at its largest size after a shrink (1792 x 1280 in a 684 x 461 webview); it is clipped by its parent and is the same with or without our surface.

### K3, two monitors at different scales (2026-11-21)

Monitors: DISPLAY1 1920 x 1080 at scale 1, DISPLAY2 1920 x 1080 at scale 1.5 (set by the owner for this test), the render thread on Vulkan, Mailbox presenting. The window was moved with `SetWindowPos` from a per-monitor-DPI-aware script, captured per monitor, and measured on the middle row: the panel's edge (page: 200 CSS px), the right bar (60 CSS px), and strips of the surface's clear color between them and the photo, which would show a mismatch. The harness now follows the window's own size as well as the page's hole reports, as the shell must from the window's Resized and ScaleFactorChanged events: an earlier run that resized the surface only on hole reports left the swapchain at the old size after a DPI change, which showed as uncovered area. That is a rule for the shell, not a finding against D.

| State | Client area | Panel edge | Right bar | Gaps | Black |
| --- | --- | --- | --- | --- | --- |
| On monitor 1 (scale 1), settled | 784 x 521 | 200 px | 60 px | 0 | 0 |
| On monitor 2 (scale 1.5), settled | 1178 x 784 | 300 px (200 x 1.5) | 89 px (60 x 1.5) | 0 | 0 |
| Back on monitor 1 | 517 x 334 | 200 px | 60 px | 0 | 0 |
| Dragging across, 40 px steps, 30 ms after each | the DPI changes between x=1620 and 1660 | | | 0 at every step, including the first after the change | 0 at every step after the change |

- **K3 cleared on this pair.** At both scales the photo is placed and sized in the window's own device pixels, flush with the page's hole, and sharp: patch edges on monitor 2 (scale 1.5) are one pixel, as are the DOM's text and edges beside it. The first sample after the DPI change, 30 ms later, is already correct. What this does not cover is faster crossings than 30 ms steps, or a window dragged by hand (the capture rate is the limit), or a monitor at a scale such as 125% (fractional device pixels per CSS pixel).
- **The black samples that remain while moving** (up to about 1400 of 57,000 at some positions on monitor 1) are the NVIDIA Overlay's toast ("Press Alt+Z to open NVIDIA Overlay"), which appeared over the window at those positions in the capture; a plain WinForms window moved over the same positions shows none. That overlay was running throughout the spike, and it may also be what drew the black window-shaped blocks seen after shrinks earlier (K2), which stayed unexplained and not caused by D. This is a hypothesis for those, not shown; closing the overlay and repeating the shrink would show it.

### Where the spike stands

| Criterion | State |
| --- | --- |
| K1 surface visible through the hole, HTML on top, clickable | Cleared (Tauri, Windows) |
| K2 no flicker or lag on resize and panel drag | Resize: cleared (no gaps in 67 steps). Panel drag: **passes with Mailbox** (under half a frame), **marginal with Fifo** (about 1.4 frames at the fastest part of a drag). The black block after a shrink is WebView2's, present without D |
| K3 two monitors, different scales | Cleared on 100% and 150%: correct size and placement, sharp, correct from the first sample after the change. The shell must follow Resized and ScaleFactorChanged, not only the page's reports |
| K4 engine step to present within 2 refreshes | Cleared on the RTX 3060 (1.3 ms engine side) |
| K5 lossless, full view size | Cleared |
| K6 software adapter shows the photo | Cleared |
| K7 teardown | Cleared for both orders tested |
| Color: a viable path | Windows mechanism measured: the OS does not convert, P3 cannot be tagged; the path is the present pass converting itself (plan step 4). macOS: kit built, not yet run |
