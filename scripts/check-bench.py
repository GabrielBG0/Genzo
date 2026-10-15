#!/usr/bin/env python3
"""Compare benchmark results against a stored baseline (0.A step 9).

    check-bench.py BASELINE.json RUN.json [RUN.json ...]
    check-bench.py --update BASELINE.json RUN.json [RUN.json ...]

Each RUN.json comes from `genzo-cli bench ... --json FILE`. With several runs the
lowest p95 of each metric is used, which filters out noise from a shared CI
runner. A metric fails when its p95 is more than THRESHOLD percent above the
baseline *and* more than FLOOR_MS slower in absolute terms, so a 0.2 ms stage
cannot fail the build over scheduler jitter. A missing baseline is not a failure:
the script says so and exits 0, and `--update` writes one from the runs.
Baselines are only comparable on the same device class, so there is one file per
runner (see benchmarks/README.md).
"""
import json
import sys
from pathlib import Path

THRESHOLD = 10.0  # percent, from CLAUDE.md: a p95 regression above 10 percent fails the build
FLOOR_MS = 0.5


def merge(paths):
    best = {}
    for path in paths:
        for name, m in json.loads(Path(path).read_text()).items():
            if name not in best or m["p95_ms"] < best[name]["p95_ms"]:
                best[name] = m
    return best


def main(argv):
    update = argv[:1] == ["--update"]
    if update:
        argv = argv[1:]
    if len(argv) < 2:
        print(__doc__)
        return 2
    baseline_path, runs = Path(argv[0]), merge(argv[1:])

    if update:
        baseline_path.parent.mkdir(parents=True, exist_ok=True)
        baseline_path.write_text(json.dumps(runs, indent=2, sort_keys=True) + "\n")
        print(f"wrote {baseline_path} ({len(runs)} metrics)")
        return 0
    if not baseline_path.exists():
        print(f"::warning::no baseline at {baseline_path}; nothing to compare against. "
              f"Download the bench artifact of this run and commit it there to arm the gate.")
        for name, m in sorted(runs.items()):
            print(f"  {name}: p95 {m['p95_ms']:.2f} ms")
        return 0

    baseline = json.loads(baseline_path.read_text())
    failed = []
    print(f"{'metric':<44} {'baseline p95':>12} {'now p95':>10} {'change':>8}")
    for name, base in sorted(baseline.items()):
        now = runs.get(name)
        if now is None:
            print(f"{name:<44} {base['p95_ms']:>12.2f} {'missing':>10}")
            failed.append(f"{name}: not measured any more")
            continue
        b, n = base["p95_ms"], now["p95_ms"]
        change = (n - b) / b * 100 if b else 0.0
        bad = change > THRESHOLD and n - b > FLOOR_MS
        print(f"{name:<44} {b:>12.2f} {n:>10.2f} {change:>+7.1f}%{'  FAIL' if bad else ''}")
        if bad:
            failed.append(f"{name}: p95 {b:.2f} -> {n:.2f} ms ({change:+.1f}%)")
    for name in sorted(set(runs) - set(baseline)):
        print(f"{name:<44} {'new':>12} {runs[name]['p95_ms']:>10.2f}")
    if failed:
        print("\nperformance regression:\n  " + "\n  ".join(failed))
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
