#!/usr/bin/env python3
"""Decides which CI jobs a run needs, from the files it changes.

Prints two flags, for $GITHUB_OUTPUT:

- `code`: anything outside the documentation changed. A change that touches only
  documentation skips the build, test, GPU, license and architecture jobs: nothing
  they check can differ. Documentation means files under docs/, any Markdown file,
  and license files. Everything else, the workflows included, counts as code.
- `bench`: something the performance gate measures or judges by changed: the
  backend workspace (crates, manifests, lockfile, submodules), the baselines, the
  comparison script or this workflow. A frontend or shell change cannot move the
  numbers, so the benchmark steps skip it.

When the changed files cannot be listed (a new branch, a force push, a missing
commit), both flags are true, so the full run happens.

Usage: python3 scripts/ci-changes.py BASE HEAD
"""

import fnmatch
import subprocess
import sys

DOCS = ["docs/*", "*.md", "LICENSE", "LICENSE.*", "COPYING"]

BENCH = [
    "backend/Cargo.toml",
    "backend/Cargo.lock",
    "backend/crates/*",
    "benchmarks/*",
    "scripts/check-bench.py",
    "scripts/ci-changes.py",
    ".github/workflows/ci.yml",
    ".gitmodules",
]


def is_docs(path: str) -> bool:
    name = path.rsplit("/", 1)[-1]
    return any(fnmatch.fnmatch(path, p) or fnmatch.fnmatch(name, p) for p in DOCS)


def is_bench(path: str) -> bool:
    return not is_docs(path) and any(fnmatch.fnmatch(path, p) for p in BENCH)


def changed_files(base: str, head: str) -> list[str] | None:
    if not base or set(base) == {"0"}:
        return None
    result = subprocess.run(
        ["git", "diff", "--name-only", base, head], capture_output=True, text=True
    )
    if result.returncode != 0:
        print(result.stderr, file=sys.stderr)
        return None
    return [line for line in result.stdout.splitlines() if line]


def flag(name: str, value: bool) -> None:
    print(f"{name}={'true' if value else 'false'}")


def main() -> None:
    base, head = sys.argv[1], sys.argv[2]
    files = changed_files(base, head)
    # An empty diff is odd enough to run everything.
    if not files:
        print("No changed files listed; running everything.", file=sys.stderr)
        flag("code", True)
        flag("bench", True)
        return
    outside = [f for f in files if not is_docs(f)]
    bench = [f for f in files if is_bench(f)]
    print(f"{len(files)} files changed, {len(outside)} outside the docs:", file=sys.stderr)
    for f in outside:
        print(f"  {f}{'  (benchmarked)' if f in bench else ''}", file=sys.stderr)
    flag("code", bool(outside))
    flag("bench", bool(bench))


if __name__ == "__main__":
    main()
