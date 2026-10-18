#!/usr/bin/env python3
"""Decides which CI jobs a run needs, from the files it changes.

Prints one flag per job, for $GITHUB_OUTPUT: `backend`, `gpu`, `bench`,
`architecture`, `deny`, `frontend` and `shell`. Each is true when a changed file
can affect what that job checks (see RULES).

- Documentation never runs anything: files under docs/, any Markdown file, and
  license files.
- A file no rule names runs every job, so a new kind of file can never slip past
  CI unchecked. Add a rule when it should not.
- When the changed files cannot be listed (a new branch, a force push, a missing
  commit), every flag is true.

Usage: python3 scripts/ci-changes.py BASE HEAD
"""

import fnmatch
import subprocess
import sys

JOBS = ["backend", "gpu", "bench", "architecture", "deny", "frontend", "shell"]

DOCS = ["docs/*", "*.md", "LICENSE", "LICENSE.*", "COPYING"]

# Pattern (fnmatch, `*` crosses `/`) and the jobs a change there needs. A path can
# match several rules; it needs the union.
RULES = [
    # The workspace is built and tested by the backend jobs, tested and benchmarked
    # on lavapipe, checked for layering and licenses, and compiled into the shell.
    ("backend/Cargo.toml", {"backend", "gpu", "bench", "architecture", "deny", "shell"}),
    ("backend/Cargo.lock", {"backend", "gpu", "bench", "architecture", "deny", "shell"}),
    ("backend/crates/*", {"backend", "gpu", "bench", "architecture", "shell"}),
    # Vendored crates (rawler, patched): built into backend and shell, and
    # their licenses checked.
    ("backend/vendor/*", {"backend", "gpu", "bench", "architecture", "deny", "shell"}),
    (".gitmodules", {"backend", "gpu", "bench", "shell"}),
    # The fuzz targets are their own workspace, and no CI job builds them.
    ("backend/fuzz/*", set()),
    # The backend jobs regenerate the contract and compare; the frontend imports it.
    ("contract/*", {"backend", "frontend"}),
    ("frontend/*", {"frontend"}),
    ("shell/Cargo.toml", {"architecture", "deny", "shell"}),
    ("shell/Cargo.lock", {"architecture", "deny", "shell"}),
    ("shell/*", {"architecture", "shell"}),
    ("benchmarks/*", {"gpu", "bench"}),
    ("scripts/check-bench.py", {"gpu", "bench"}),
    ("scripts/check-architecture.py", {"architecture"}),
    ("deny.toml", {"deny"}),
    # Used only by the hand-run Bench kit workflow and by local runs; no job
    # here reads them.
    ("scripts/bench-kit/*", set()),
    (".github/workflows/bench-kit.yml", set()),
    ("scripts/make-dng-references.ps1", set()),
    ("scripts/cfa-reference.py", set()),
    ("scripts/gpu-check.sh", set()),
    ("scripts/accept-goldens.py", set()),
    ("corpus/*", set()),
    # The corpus files the golden-image tests read, and the script that fetches
    # them, decide what the backend and GPU jobs test. The rest of corpus/ is
    # local only (the rule above).
    ("corpus/manifest.json", {"backend", "gpu"}),
    ("corpus/golden-inputs.txt", {"backend", "gpu"}),
    ("scripts/fetch-raw-corpus.py", {"backend", "gpu"}),
    # What decides the jobs reruns all of them.
    ("scripts/ci-changes.py", set(JOBS)),
    (".github/workflows/ci.yml", set(JOBS)),
]


def is_docs(path: str) -> bool:
    name = path.rsplit("/", 1)[-1]
    return any(fnmatch.fnmatch(path, p) or fnmatch.fnmatch(name, p) for p in DOCS)


def jobs_for(path: str) -> set[str]:
    if is_docs(path):
        return set()
    matched = [jobs for pattern, jobs in RULES if fnmatch.fnmatch(path, pattern)]
    if not matched:
        return set(JOBS)
    return set().union(*matched)


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


def main() -> None:
    base, head = sys.argv[1], sys.argv[2]
    files = changed_files(base, head)
    needed: set[str] = set()
    # An empty diff is odd enough to run everything.
    if not files:
        print("No changed files listed; running everything.", file=sys.stderr)
        needed = set(JOBS)
    else:
        print(f"{len(files)} files changed:", file=sys.stderr)
        for f in files:
            jobs = jobs_for(f)
            needed |= jobs
            label = "docs" if is_docs(f) else ", ".join(j for j in JOBS if j in jobs) or "no job"
            print(f"  {f}: {label}", file=sys.stderr)
    for job in JOBS:
        print(f"{job}={'true' if job in needed else 'false'}")


if __name__ == "__main__":
    main()
