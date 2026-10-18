# CLAUDE.md

Guidance for Claude Code when working in this repository.

## Project

Genzo is a free desktop photo editor modeled on Lightroom Classic: Rust backend, Tauri 2 shell, Svelte 5 + TypeScript frontend. License: GPL-3.0.

The repo is in milestone M0. Before starting work, read [docs/development-plan.md](docs/development-plan.md) to find the current milestone and its steps.

## Sources of truth, in order

1. **Technical spec** (*Genzo: Technical Spec*, a Claude Docs document). It decides architecture, scope and milestones.
2. **Development plan** (`docs/development-plan.md`). It turns the spec into steps per milestone, and maps every feature ID to a milestone.
3. **UI handoff** (`docs/UI/fixer-ui-handoff.html`) and **feature list** (`docs/UI/FEATURES.md`). They decide the look, layout, tokens and interactions. "Fixer" was the project's earlier name.

Resolution rule: where the spec and the UI documents conflict, the spec wins. Where only one has something, it is in scope. Features that appear in none of them need a spec change before they are built.

## Repository layout

```
backend/    Cargo workspace, zero UI dependencies
  crates/   model, imgio, color, pipeline, catalog, cache, jobs, ai, export, engine, cli,
            heif (bundled libheif + libde265, the only crate with unsafe code; git submodules)
contract/   generated API schema and TypeScript types (generated, checked in)
shell/      thin Tauri adapter in Rust, maps the engine API to IPC
frontend/   Svelte 5 + TypeScript (strict) app, built with Vite
docs/       plan, UI handoff, feature list, license review
scripts/    CI checks (architecture rules)
```

Contract workflow: change the Rust types in `backend/crates/model`, bump `CONTRACT_VERSION`, then run `cargo run -p genzo-cli -- contract write` in `backend/`. The frontend imports the result as `$contract/genzo`.

## Architecture rules (enforced in CI)

- `backend/` depends on nothing under `shell/` or `frontend/`. No backend crate depends on Tauri, a webview or any UI toolkit.
- Backend dependencies point down only. `engine` is the only public crate; everything below it is private to the workspace.
- `frontend/` imports no Rust code and touches the Tauri API in exactly one transport module.
- `shell/` has no business logic. If it needs an if-statement about photos, that code belongs in `engine`.
- The only shared artifact is `contract/`. Rust types are the source; TypeScript types are generated (specta or ts-rs). Never hand-edit `contract/`.
- Every contract change bumps the contract version. The schema snapshot test fails on breaking changes otherwise.
- The `cli` crate must be able to do everything the UI does except draw.
- The backend owns the edit document and its history. The frontend sends edit deltas, receives frames and never computes pixels.
- Pixels (tiles, thumbnails) never travel as JSON; they use the binary tile protocol.
- One render thread owns the GPU device. CPU work runs on rayon, file IO on tokio, SQLite writes on a dedicated thread. Nothing blocks the render thread.
- Original files are never written to, including DNG and JPEG. Metadata goes to XMP sidecars.

## Frontend rules

- Tokens come from the handoff's design system as CSS variables. No component hard-codes a color.
- Backdrop blur only on slider handles and selection indicators. Everything else, including overlays on the photo, is flat.
- Edited state is always visible; every command shows its shortcut; every command is registered in the shortcut registry.
- Build each screen against the fake engine first (Vitest, Playwright), then wire it to the real engine.
- Fonts (Public Sans, JetBrains Mono) are bundled, never loaded from a CDN.

## Quality rules

- Every new pipeline stage gets golden-image tests (ΔE2000 and PSNR tolerances, lavapipe as the reference device) and a criterion benchmark.
- Every new decoder gets a `cargo-fuzz` target.
- Performance budgets are enforced in CI: a p95 regression above 10 percent fails the build. On the shared lavapipe runner the baseline uses a 25 percent threshold against the slowest run seen, because that hardware varies by about 2x between runs; see `benchmarks/README.md`. The reference GPU (the owner's RTX 3060) keeps the 10 percent rule through a manual check, `scripts/gpu-check.sh`, run before merging pipeline, engine or benchmark changes; its summary goes in the PR.
- Check the license of every native dependency before adopting it (`cargo-deny`).
- Before building a feature, its spec must be written using the template in `docs/UI/FEATURES.md`, as `docs/specs/<ID>.md`, and linked from the feature's entry there.

## Out of scope

Generative AI (generative fill, remove or expand) is out by decision. Nothing in the app creates new pixels.
