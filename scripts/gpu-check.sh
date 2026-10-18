#!/usr/bin/env bash
# The real-GPU check, run by hand on the reference machine (the owner's RTX 3060
# desktop). It replaces CI on a self-hosted runner (development plan, Decided:
# "Real-GPU checks"). Run it before merging a PR that touches the pipeline, the
# engine or the benchmarks, and paste its summary into the PR.
#
#   scripts/gpu-check.sh            # check against benchmarks/baseline-rtx3060.json
#
# 1. Fails unless the engine picks the expected adapter (GENZO_EXPECT_ADAPTER,
#    default "RTX 3060"), so a fallback to a software adapter cannot pass.
# 2. Runs the GPU tests of genzo-pipeline and genzo-engine.
# 3. Runs the lavapipe gate's benchmark cases 3 times and compares them with the
#    baseline at 10 percent. Without a baseline it only warns.
# 4. Writes the merged runs as a baseline candidate; see benchmarks/README.md for
#    building the baseline from several of them.
#
# Works in Git Bash on Windows and in bash on Linux and macOS.

set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
expect=${GENZO_EXPECT_ADAPTER:-RTX 3060}
export GENZO_REQUIRE_GPU=1

# Native Windows programs (python, cargo) need Windows paths.
native() { if command -v cygpath >/dev/null; then cygpath -m "$1"; else echo "$1"; fi; }
# The first interpreter that runs: on Windows, `python3` can be the Microsoft
# Store placeholder, which exists but only prints an install hint.
python=
for candidate in python3 python py; do
  if "$candidate" -c "import sys" > /dev/null 2>&1; then python=$candidate; break; fi
done
if [ -z "$python" ]; then echo "error: no working Python found" >&2; exit 1; fi

out_dir="$root/target-gpu-check/$(date +%Y%m%d-%H%M%S)"
mkdir -p "$out_dir"

cd "$root/backend"
echo "== adapter"
adapter=$(cargo run --locked --release -q -p genzo-cli -- bench gpu 1280 960 3 | grep '^adapter:')
echo "$adapter"
if ! grep -q "adapter: .*$expect" <<< "$adapter"; then
  echo "error: the adapter is not '$expect'" >&2
  exit 1
fi

echo "== GPU tests"
cargo test --locked -q -p genzo-pipeline -p genzo-engine

echo "== benchmarks (3 runs)"
for i in 1 2 3; do
  cargo run --locked --release -q -p genzo-cli -- bench gpu 4032 3024 30 \
    --json "$(native "$out_dir/bench-$i.json")" > /dev/null
done

cd "$root"
echo "== comparison with benchmarks/baseline-rtx3060.json"
"$python" scripts/check-bench.py benchmarks/baseline-rtx3060.json \
  "$(native "$out_dir/bench-1.json")" "$(native "$out_dir/bench-2.json")" "$(native "$out_dir/bench-3.json")"
"$python" scripts/check-bench.py --update "$(native "$out_dir/baseline-candidate.json")" \
  "$(native "$out_dir/bench-1.json")" "$(native "$out_dir/bench-2.json")" "$(native "$out_dir/bench-3.json")" > /dev/null
echo "== done: $adapter; candidate in $out_dir/baseline-candidate.json"
