# DNG SDK probes

Four small programs that print what the Adobe DNG SDK 1.4 computes, as ground truth for Genzo. They are test tools only: Genzo never builds or ships them.

- **`color_probe.cpp`:** a DNG's camera color. The reference for `backend/crates/color/src/camera.rs` (DEV-01, *Camera color: the SDK's details*).
- **`tone_probe.cpp`:** the Genzo Standard base curve: the ACR3 default tone curve, the exposure ramp, the ProfileToneCurve spline, the 4097-entry table `dng_render` builds from a curve, `RefBaselineRGBTone`, and each DNG's ramp and curve parameters. The reference for `backend/crates/color/src/tone.rs` (DEV-02).
- **`render_probe.cpp`:** a DNG rendered by `dng_render` at a given exposure into linear ProPhoto RGB, as block means over the default crop, with OpcodeList3 cleared. The reference for DEV-02's criterion 5, the whole render.
- **`stage2_probe.cpp`:** a DNG's stage 2 image, the raw image after OpcodeList1, the black and white levels and OpcodeList2, over the ActiveArea. The reference for stage 1 (DEV-01, *Stage 1: linearize*).

The SDK copy used is the one in [gopro/gpr](https://github.com/gopro/gpr) (`source/lib/dng_sdk`, DNG SDK 1.4, BSD-3-Clause there). gpr builds it with CMake.

## Build (Windows, Visual Studio 2022)

1. Clone gpr at commit `446c736` into a scratch directory (outside the repo).
2. Copy this directory's `color_probe.cpp`, `stage2_probe.cpp`, `tone_probe.cpp`, `render_probe.cpp` and `CMakeLists.txt` to `source/app/color_probe/` in the clone.
3. Add `add_subdirectory( "source/app/color_probe" )` to the end of the clone's top-level `CMakeLists.txt`.
4. MSVC rejects expat's `__attribute((fallthrough));`. In `source/lib/expat_lib/xmltok.c` and `xmltok_impl.c`, replace it with `FALL_THROUGH;`.
5. Configure and build:

   ```
   cmake -S . -B build
   cmake --build build --config Release --target color_probe stage2_probe tone_probe render_probe --parallel
   ```

   Use `--parallel`, not `/m`: Git Bash rewrites `/m` into a path.

The executables land in `build/source/app/color_probe/Release/`.

## Use

```
uv run --no-project --with tifffile --with numpy python scripts/color-reference.py --probe PATH/color_probe.exe
```

That rewrites `backend/crates/color/tests/fixtures/dng-sdk-color.jsonl` and `dng-sdk-temperature.jsonl`. The corpus files it reads are listed in the script. The Adobe DNG Converter references come from `scripts/make-dng-references.ps1`.

Run directly, `color_probe FILE...` prints one JSON object per file, and `color_probe --temperature` prints the SDK's Temp and Tint to xy conversion for a fixed set of pairs.

`stage2_probe IN.dng OUT.bin` writes the stage 2 image, and `scripts/stage1-reference.py --probe PATH/stage2_probe.exe` turns six corpus DNGs' images into the crops and block means in `backend/crates/pipeline/tests/fixtures/sdk-stage2/`.

`uv run --no-project --with tifffile --with numpy python scripts/tone-reference.py --probe PATH/tone_probe.exe` rewrites `dng-sdk-tone.json`, `dng-sdk-tone-params.jsonl` and `dng-sdk-rgbtone.f32` in `backend/crates/color/tests/fixtures/`. Run directly, `tone_probe --curves FILE...`, `--rgbtone N OUT` and `--params FILE...` print each part; the RGBTone inputs come from a generator that `color/tests/tone_sdk.rs` repeats.

`python scripts/render-reference.py --probe PATH/render_probe.exe` rewrites `backend/crates/pipeline/tests/fixtures/sdk-render/`: five corpus DNGs at +1 EV, and at 0 EV those whose BaselineExposure is 0 or more. `pipeline/tests/render_sdk.rs` compares Genzo's stage 6 with them.
