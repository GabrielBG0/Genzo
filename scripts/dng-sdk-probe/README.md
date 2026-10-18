# DNG SDK color probe

`color_probe.cpp` prints what the Adobe DNG SDK 1.4 computes for a DNG's camera color. It is the ground truth for `backend/crates/color/src/camera.rs` (DEV-01, *Camera color: the SDK's details*). It is a test tool only: Genzo never builds or ships it.

The SDK copy used is the one in [gopro/gpr](https://github.com/gopro/gpr) (`source/lib/dng_sdk`, DNG SDK 1.4, BSD-3-Clause there). gpr builds it with CMake.

## Build (Windows, Visual Studio 2022)

1. Clone gpr at commit `446c736` into a scratch directory (outside the repo).
2. Copy this directory's `color_probe.cpp` and `CMakeLists.txt` to `source/app/color_probe/` in the clone.
3. Add `add_subdirectory( "source/app/color_probe" )` to the end of the clone's top-level `CMakeLists.txt`.
4. MSVC rejects expat's `__attribute((fallthrough));`. In `source/lib/expat_lib/xmltok.c` and `xmltok_impl.c`, replace it with `FALL_THROUGH;`.
5. Configure and build:

   ```
   cmake -S . -B build
   cmake --build build --config Release --target color_probe --parallel
   ```

   Use `--parallel`, not `/m`: Git Bash rewrites `/m` into a path.

The executable lands in `build/source/app/color_probe/Release/color_probe.exe`.

## Use

```
uv run --no-project --with tifffile --with numpy python scripts/color-reference.py --probe PATH/color_probe.exe
```

That rewrites `backend/crates/color/tests/fixtures/dng-sdk-color.jsonl` and `dng-sdk-temperature.jsonl`. The corpus files it reads are listed in the script. The Adobe DNG Converter references come from `scripts/make-dng-references.ps1`.

Run directly, `color_probe FILE...` prints one JSON object per file, and `color_probe --temperature` prints the SDK's Temp and Tint to xy conversion for a fixed set of pairs.
