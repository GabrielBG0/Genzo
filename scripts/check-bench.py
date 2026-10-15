#!/usr/bin/env python3
"""Compare benchmark results against a stored baseline (0.A step 9).

    check-bench.py BASELINE.json RUN.json [RUN.json ...]
    check-bench.py --update [--envelope] [--threshold PCT] BASELINE.json RUN.json [RUN.json ...]

Each RUN.json comes from `genzo-cli bench ... --json FILE`.

Comparing: with several runs the lowest p95 of each metric is used, which filters
noise from a shared CI runner. A metric fails when its p95 is more than the
baseline's threshold percent above the baseline *and* more than FLOOR_MS slower
in absolute terms, so a 0.2 ms stage cannot fail the build over scheduler jitter.
The threshold is DEFAULT_THRESHOLD (10 percent, the rule in CLAUDE.md) unless the
baseline carries its own in `_meta`. A metric baselined from fewer than MIN_RUNS
runs is reported but cannot fail yet. A missing baseline is not a failure: the
script says so and exits 0.

Updating: writes a baseline from the runs. With --envelope, each metric's p95 is the
slowest p95 seen across the runs and its p50 the median, which is what a shared
runner of mixed hardware needs: lavapipe runs on the same code differ by about 2x
from one run to the next, so a baseline from a single run or a median fails a
quarter of the runs on its own. Baselines are only comparable on the same device
class, so there is one file per runner (see benchmarks/README.md).
"""
import json
import statistics
import sys
from pathlib import Path

DEFAULT_THRESHOLD = 10.0  # percent, from CLAUDE.md: a p95 regression above 10 percent fails the build
FLOOR_MS = 0.5
MIN_RUNS = 5  # a metric seen in fewer runs than this is advisory


def load_runs(paths):
    """Each file is one run: {metric: {p50_ms, p95_ms}}."""
    return [json.loads(Path(p).read_text()) for p in paths]


def lowest(runs):
    best = {}
    for run in runs:
        for name, m in run.items():
            if name.startswith("_"):
                continue
            if name not in best or m["p95_ms"] < best[name]["p95_ms"]:
                best[name] = m
    return best


def envelope(runs, threshold):
    seen = {}
    for run in runs:
        for name, m in run.items():
            if not name.startswith("_"):
                seen.setdefault(name, []).append(m)
    out = {
        name: {
            "p50_ms": statistics.median(m["p50_ms"] for m in ms),
            "p95_ms": max(m["p95_ms"] for m in ms),
            "runs": len(ms),
        }
        for name, ms in seen.items()
    }
    out["_meta"] = {
        "threshold_pct": threshold,
        "method": "p95 is the slowest p95 across runs, p50 the median",
        "runs": len(runs),
    }
    return out


def main(argv):
    update = "--update" in argv
    use_envelope = "--envelope" in argv
    threshold = None
    if "--threshold" in argv:
        threshold = float(argv[argv.index("--threshold") + 1])
    argv = [a for i, a in enumerate(argv) if not a.startswith("--") and not (i and argv[i - 1] == "--threshold")]
    if len(argv) < 2:
        print(__doc__)
        return 2
    baseline_path, runs = Path(argv[0]), load_runs(argv[1:])

    if update:
        data = envelope(runs, threshold or DEFAULT_THRESHOLD) if use_envelope else lowest(runs)
        baseline_path.parent.mkdir(parents=True, exist_ok=True)
        baseline_path.write_text(json.dumps(data, indent=2, sort_keys=True) + "\n")
        print(f"wrote {baseline_path} ({len([k for k in data if not k.startswith('_')])} metrics, {len(runs)} runs)")
        return 0

    now_all = lowest(runs)
    if not baseline_path.exists():
        print(f"::warning::no baseline at {baseline_path}; nothing to compare against. "
              f"Download the bench artifacts of recent runs and build one with --update --envelope.")
        for name, m in sorted(now_all.items()):
            print(f"  {name}: p95 {m['p95_ms']:.2f} ms")
        return 0

    baseline = json.loads(baseline_path.read_text())
    limit = threshold or baseline.get("_meta", {}).get("threshold_pct", DEFAULT_THRESHOLD)
    failed = []
    print(f"threshold {limit:g}% (and more than {FLOOR_MS} ms)")
    print(f"{'metric':<44} {'baseline p95':>12} {'now p95':>10} {'change':>8}")
    for name, base in sorted(baseline.items()):
        if name.startswith("_"):
            continue
        now = now_all.get(name)
        if now is None:
            print(f"{name:<44} {base['p95_ms']:>12.2f} {'missing':>10}")
            failed.append(f"{name}: not measured any more")
            continue
        b, n = base["p95_ms"], now["p95_ms"]
        change = (n - b) / b * 100 if b else 0.0
        over = change > limit and n - b > FLOOR_MS
        advisory = base.get("runs", MIN_RUNS) < MIN_RUNS
        note = ""
        if over:
            note = f"  ADVISORY (baselined from {base['runs']} runs)" if advisory else "  FAIL"
        print(f"{name:<44} {b:>12.2f} {n:>10.2f} {change:>+7.1f}%{note}")
        if over and not advisory:
            failed.append(f"{name}: p95 {b:.2f} -> {n:.2f} ms ({change:+.1f}%)")
    for name in sorted(set(now_all) - {k for k in baseline if not k.startswith("_")}):
        print(f"{name:<44} {'new':>12} {now_all[name]['p95_ms']:>10.2f}")
    if failed:
        print("\nperformance regression:\n  " + "\n  ".join(failed))
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
