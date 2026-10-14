# Genzo

A free desktop photo editor modeled on Lightroom Classic. It has a Rust backend, a Svelte frontend, and a hard performance budget. It runs on Windows, Linux and macOS from one codebase, and opens iPhone RAW files (Apple ProRAW, HEIC) as well as RAW files from all major camera makers.

> **Status:** pre-alpha, milestone M0. The repository scaffolding, the contract pipeline, the Tauri shell and the frontend foundation (tokens, shared components, fake engine) are in place. The feasibility spikes (ProRAW decode, GPU pipeline, pixels through the webview) are under way; see the [development plan](docs/development-plan.md).

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

## Development

Requirements: Rust 1.90 or newer, Node 24 with pnpm, and on Linux the WebKitGTK 4.1 development packages (`libwebkit2gtk-4.1-dev`, `libsoup-3.0-dev`).

```sh
# Backend: tests, the contract snapshot, and the engine-side benchmark
cd backend
cargo test
cargo run -p genzo-cli -- contract write   # after changing a contract type (bump CONTRACT_VERSION first)
cargo run --release -p genzo-cli -- bench test-pattern 2560 1440 60

# Frontend against the fake engine, in a browser
cd frontend
pnpm install
pnpm dev                 # http://localhost:5173, component gallery at /#/gallery
pnpm check && pnpm lint && pnpm test

# The desktop app with the real engine (starts the Vite dev server too)
cd frontend
pnpm app:dev
```

`python3 scripts/check-architecture.py` checks the dependency rules that CI enforces.

## Documentation

- [Development plan](docs/development-plan.md): milestones M0 to M5, their exit gates, and every feature mapped to a milestone.
- [UI handoff](docs/UI/fixer-ui-handoff.html): the Carbon Glass design system, every mockup screen and the component list. Open it in a browser.
- [Feature list](docs/UI/FEATURES.md): every UI feature with its priority, screens and open questions.
- [License review](docs/licenses.md): adopted dependencies and the native libraries still under review.

The technical spec is the source of truth. Where the UI documents disagree with it, the spec wins.

## License

Genzo is licensed under the [GNU General Public License v3.0](LICENSE).
