#!/usr/bin/env python3
"""Decides whether a CI run needs the code jobs, from the files it changes.

A change that touches only documentation skips the build, test, GPU, license and
architecture jobs: nothing they check can differ. Documentation means files under
docs/, any Markdown file, and license files. Everything else, the workflows
included, counts as code.

When the changed files cannot be listed (a new branch, a force push, a missing
commit), it answers "code" so the full run happens.

Prints `code=true` or `code=false`, for $GITHUB_OUTPUT.

Usage: python3 scripts/ci-changes.py BASE HEAD
"""

import fnmatch
import subprocess
import sys

DOCS = ["docs/*", "*.md", "LICENSE", "LICENSE.*", "COPYING"]


def is_docs(path: str) -> bool:
    name = path.rsplit("/", 1)[-1]
    return any(fnmatch.fnmatch(path, p) or fnmatch.fnmatch(name, p) for p in DOCS)


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
    if files is None:
        print("Cannot list the changed files; running everything.", file=sys.stderr)
        code = True
    else:
        outside = [f for f in files if not is_docs(f)]
        print(f"{len(files)} files changed, {len(outside)} outside the docs:", file=sys.stderr)
        for f in outside:
            print(f"  {f}", file=sys.stderr)
        # An empty diff is odd enough to run everything.
        code = bool(outside) or not files
    print(f"code={'true' if code else 'false'}")


if __name__ == "__main__":
    main()
