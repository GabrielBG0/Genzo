#!/usr/bin/env python3
"""Enforces the dependency rules from CLAUDE.md and the spec.

1. No backend crate depends (even transitively) on Tauri, a webview or a UI toolkit.
2. Backend crates depend only on crates in lower layers.
3. The cli and the shell reach the backend through genzo-engine alone.
   Only genzo-imgio may use genzo-heif, the crate with the native code.
4. The shell stays thin: a line budget on its Rust sources.

Run from anywhere: python3 scripts/check-architecture.py
"""

import json
import pathlib
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent

# Crate names that mean a UI dependency leaked into the backend.
FORBIDDEN = {
    "tauri", "tauri-runtime", "tauri-runtime-wry", "wry", "tao", "webkit2gtk", "webview2-com",
    "gtk", "gdk", "egui", "eframe", "slint", "iced", "winit", "druid", "fltk",
}

# Layers, bottom to top. A crate may depend only on crates in lower layers.
LAYERS = [
    # genzo-heif wraps the bundled libheif; it is the only crate with unsafe code.
    {"genzo-model", "genzo-heif"},
    {"genzo-imgio", "genzo-color", "genzo-cache", "genzo-jobs", "genzo-catalog"},
    {"genzo-pipeline", "genzo-ai", "genzo-golden"},
    {"genzo-export"},
    {"genzo-engine"},
    {"genzo-cli"},
]
LAYER_OF = {name: i for i, layer in enumerate(LAYERS) for name in layer}

SHELL_LINE_BUDGET = 200


def metadata(manifest_dir: pathlib.Path) -> dict:
    out = subprocess.run(
        ["cargo", "metadata", "--format-version", "1"],
        cwd=manifest_dir, check=True, capture_output=True, text=True,
    ).stdout
    return json.loads(out)


def main() -> int:
    errors: list[str] = []

    meta = metadata(ROOT / "backend")
    names = {p["name"] for p in meta["packages"]}
    for bad in sorted(names & FORBIDDEN):
        errors.append(f"backend depends on UI crate '{bad}'")

    members = {p["id"]: p for p in meta["packages"] if p["id"] in meta["workspace_members"]}
    for pkg in members.values():
        name = pkg["name"]
        if name not in LAYER_OF:
            errors.append(f"{name} is not assigned to a layer in scripts/check-architecture.py")
            continue
        for dep in pkg["dependencies"]:
            if not dep["name"].startswith("genzo-") or dep["kind"] == "dev":
                continue
            if LAYER_OF.get(dep["name"], 99) >= LAYER_OF[name]:
                errors.append(f"{name} depends on {dep['name']}, which is not in a lower layer")
        # Test support only: no crate may ship it.
        for dep in pkg["dependencies"]:
            if dep["name"] == "genzo-golden" and dep["kind"] != "dev":
                errors.append(f"{name} depends on genzo-golden outside [dev-dependencies]")
        uses_heif = any(d["name"] == "genzo-heif" for d in pkg["dependencies"])
        if uses_heif and name != "genzo-imgio":
            errors.append(f"{name} uses genzo-heif directly; only genzo-imgio may")
        if name == "genzo-cli":
            internal = {d["name"] for d in pkg["dependencies"] if d["name"].startswith("genzo-")}
            if internal != {"genzo-engine"}:
                errors.append(f"genzo-cli must depend on genzo-engine only, found {sorted(internal)}")

    shell = metadata(ROOT / "shell")
    shell_pkg = next(p for p in shell["packages"] if p["name"] == "genzo-shell")
    internal = {d["name"] for d in shell_pkg["dependencies"] if d["name"].startswith("genzo-")}
    if internal != {"genzo-engine"}:
        errors.append(f"the shell must depend on genzo-engine only, found {sorted(internal)}")

    lines = sum(len(f.read_text().splitlines()) for f in (ROOT / "shell" / "src").rglob("*.rs"))
    if lines > SHELL_LINE_BUDGET:
        errors.append(
            f"shell/src has {lines} lines, over the {SHELL_LINE_BUDGET}-line budget; "
            "move logic into genzo-engine"
        )

    for e in errors:
        print(f"architecture: {e}", file=sys.stderr)
    if not errors:
        print(f"architecture: ok ({len(members)} backend crates, shell {lines} lines)")
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
