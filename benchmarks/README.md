# Performance baselines

`baseline-<device>.json` holds the p50 and p95 (ms) of each benchmarked stage on one
device class. CI fails when a p95 is more than the file's threshold above its baseline
and more than 0.5 ms slower (`scripts/check-bench.py`). The rule in `CLAUDE.md` is 10
percent; a baseline file may set its own threshold in `_meta` when its runner cannot
support that (below).

Baselines only mean something on the machine class that made them, so there is one file
per CI runner. Today that is `baseline-lavapipe.json`, from the `gpu` job (ubuntu-22.04,
lavapipe software Vulkan). Numbers from a developer GPU or from WARP must not go in it.

## Why the lavapipe baseline uses 25 percent and the slowest run

The `gpu` job runs on a shared GitHub runner, and the hardware behind `ubuntu-22.04`
varies. The same code measured 1.9 s and 3.8 s for the settled white balance render
in two runs of that job (eight runs on 2026-10-15 spread by about 2x on every metric,
in a fast and a slow group). A baseline from one run, or from the median, would fail
a quarter of the runs with no code change.

So `baseline-lavapipe.json` is an envelope: each p95 is the **slowest** p95 seen across
the runs, and the threshold is **25 percent**. What that costs: a regression is only
caught once it pushes a run past the slowest runner seen so far plus 25 percent. On
a fast runner that means roughly 2x for the long stages and 1.5x for the short ones
(checked: 1.5x fails two metrics, 2x fails three, 4x fails all that are not advisory).
It catches a big regression like the 4x one fixed in 0.E, not a 15 percent drift.

A metric baselined from fewer than 5 runs is **advisory**: it is printed and flagged
but cannot fail the build yet. The `... preview` metrics were baselined from 2 runs.

The 10 percent rule is meant for a stable device. When a private runner with a real
GPU exists, give it its own `baseline-<runner>.json` with the default 10 percent, and
keep this one as a coarse check on the software adapter.

## Arming or updating the gate

1. Download the `bench-lavapipe` artifacts (`baseline-candidate.json`) of as many
   recent successful runs of the current code as you can, one folder per run:
   `gh run download RUN_ID -n bench-lavapipe -D runs/RUN_ID`. Runs on `main` are best.
2. Build the baseline from all of them:
   `python3 scripts/check-bench.py --update --envelope --threshold 25 benchmarks/baseline-lavapipe.json runs/*/baseline-candidate.json`
3. Commit it. Do this in the same PR as an intended slowdown or a deliberate change to
   the benchmark, and say why in the PR. Add runs over time: more runs make the
   envelope more trustworthy and turn advisory metrics into gating ones.

Without a baseline file the gate only prints a warning.

## Running locally

```
cd backend
for i in 1 2 3; do cargo run --release -p genzo-cli -- bench gpu 4032 3024 30 --json /tmp/b$i.json; done
python3 ../scripts/check-bench.py ../benchmarks/baseline-lavapipe.json /tmp/b?.json
```

The criterion benchmarks (`cargo bench -p genzo-pipeline`) are for investigating a
regression, not part of the gate: criterion's own baselines are per machine and not stored.
