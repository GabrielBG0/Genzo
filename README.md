# Genzo

A free desktop photo editor modeled on Lightroom Classic. It has a Rust backend, a Svelte frontend, and a hard performance budget. It runs on Windows, Linux and macOS from one codebase, and opens iPhone RAW files (Apple ProRAW, HEIC) as well as RAW files from all major camera makers.

> **Status:** pre-alpha. There is no code yet. The project is in its planning stage; work starts with the M0 feasibility spikes described in the [development plan](docs/development-plan.md).

## Goals

- Cover what photographers use daily in Lightroom Classic: import, catalog, culling, non-destructive development, local adjustments and export.
- Open iPhone photos and the common camera RAW formats.
- Feel instant: sliders update within one frame at fit-to-screen zoom, and the grid scrolls at 60 fps.
- Keep the backend and the frontend as two codebases that meet only at a typed contract.

No generative AI. AI is used only to assist selection and masking and for denoise, and every model runs on your machine.

## Architecture

```
backend/    Cargo workspace (model, imgio, color, pipeline, catalog, cache,
            jobs, ai, export, engine, cli). No UI dependencies.
contract/   API schema and TypeScript types generated from the Rust types
shell/      Thin Tauri 2 adapter that maps the engine API to IPC
frontend/   Svelte 5 + TypeScript app. Never imports Rust code.
docs/       Development plan, UI handoff and feature list
```

- Edits are a parametric graph rendered on the GPU with wgpu, at the resolution being displayed. Original files are never modified.
- Machines without a usable GPU fall back to a software adapter (WARP on Windows, lavapipe on Linux), so every feature still works, only slower.
- The catalog is one SQLite file. Ratings, keywords and develop settings are also written to XMP sidecars.

## Documentation

- [Development plan](docs/development-plan.md): milestones M0 to M5, their exit gates, and every feature mapped to a milestone.
- [UI handoff](docs/UI/fixer-ui-handoff.html): the Carbon Glass design system, every mockup screen and the component list. Open it in a browser.
- [Feature list](docs/UI/FEATURES.md): every UI feature with its priority, screens and open questions.

The technical spec is the source of truth. Where the UI documents disagree with it, the spec wins.

## License

Genzo is licensed under the [GNU General Public License v3.0](LICENSE).
