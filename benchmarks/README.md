# Performance baselines

`baseline-<device>.json` holds the p50 and p95 (ms) of each benchmarked stage on one
device class. CI fails when a p95 is more than 10 percent above its baseline and more
than 0.5 ms slower (`scripts/check-bench.py`).

Baselines only mean something on the machine class that made them, so there is one file
per CI runner. Today that is `baseline-lavapipe.json`, from the `gpu` job (ubuntu-22.04,
lavapipe software Vulkan). Numbers from a developer GPU or from WARP must not go in it.

## Arming or updating the gate

1. Open the CI run on the commit you want to baseline and download the `bench-lavapipe`
   artifact (`baseline-candidate.json`).
2. Save it as `benchmarks/baseline-lavapipe.json` and commit it.

Until that file exists the gate only prints a warning. Update it in the same PR as an
intended slowdown or a deliberate change to the benchmark, and say why in the PR.

## Running locally

```
cd backend
for i in 1 2 3; do cargo run --release -p genzo-cli -- bench gpu 4032 3024 30 --json /tmp/b$i.json; done
python3 ../scripts/check-bench.py ../benchmarks/baseline-lavapipe.json /tmp/b?.json
```

The criterion benchmarks (`cargo bench -p genzo-pipeline`) are for investigating a
regression, not part of the gate: criterion's own baselines are per machine and not stored.
