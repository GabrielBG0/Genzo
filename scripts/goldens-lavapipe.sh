#!/usr/bin/env bash
# Writes golden-image candidates on lavapipe, the reference device, in a
# Docker container (scripts/lavapipe/Dockerfile), so a PR can carry its new
# GPU goldens from its first push. The CI "GPU tests (lavapipe)" job stays the
# check; this only produces what scripts/accept-goldens.py may accept.
#
#   scripts/goldens-lavapipe.sh                      # the tests CI's lavapipe job runs
#   scripts/goldens-lavapipe.sh -p genzo-pipeline --test look_golden
#   scripts/goldens-lavapipe.sh --accept look/synthetic-cropped    # from the last run
#
# 1. Builds the image (once; `docker build` reuses its layers after that).
# 2. Copies the working tree (uncommitted changes included, without target/ and
#    corpus/) into the container, mounts corpus/ read-only for the corpus
#    goldens, and runs `cargo test` with GENZO_ADAPTER=software. The cargo
#    registry and target folder live in named volumes, so later runs are
#    incremental.
# 3. Leaves the candidates of failed or missing goldens, with the driver's
#    vulkaninfo, in backend/target/golden-lavapipe/.
# 4. With --accept, hands the named cases to scripts/accept-goldens.py, which
#    checks they came from lavapipe.
#
# Needs Docker Desktop running. Stop it again before benchmarks or
# scripts/gpu-check.sh: it skews the engine benches. Works in Git Bash on
# Windows and in bash on Linux and macOS.

set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
native() { if command -v cygpath >/dev/null; then cygpath -m "$1"; else echo "$1"; fi; }
python=
for candidate in python3 python py; do
  if "$candidate" -c "import sys" >/dev/null 2>&1; then python=$candidate; break; fi
done

accept=()
args=()
while [ $# -gt 0 ]; do
  case "$1" in
    --accept) shift; while [ $# -gt 0 ] && [ "${1#-}" = "$1" ]; do accept+=("$1"); shift; done ;;
    *) args+=("$1"); shift ;;
  esac
done
out="$root/backend/target/golden-lavapipe"
# --accept alone takes the last run's candidates without testing again.
if [ ${#accept[@]} -gt 0 ] && [ ${#args[@]} -eq 0 ]; then
  exec "$python" "$(native "$root/scripts/accept-goldens.py")" "$(native "$out")" --only "${accept[@]}"
fi
if [ ${#args[@]} -eq 0 ]; then
  args=(-p genzo-pipeline -p genzo-engine)
fi

rm -rf "$out"
mkdir -p "$out" "$root/corpus"

image=genzo-lavapipe
echo "== image $image"
docker build -q -t "$image" "$(native "$root/scripts/lavapipe")" >/dev/null

echo "== cargo test ${args[*]} on lavapipe"
status=0
# MSYS_NO_PATHCONV keeps Git Bash from rewriting the container's paths.
MSYS_NO_PATHCONV=1 docker run --rm \
  -e GENZO_ADAPTER=software -e GENZO_REQUIRE_GPU=1 -e CARGO_TERM_COLOR=never \
  -v "$(native "$root"):/src:ro" \
  -v "$(native "$root/corpus"):/work/corpus:ro" \
  -v "$(native "$out"):/out" \
  -v genzo-cargo-registry:/root/.cargo/registry \
  -v genzo-lavapipe-target:/work/backend/target \
  "$image" bash -c '
    set -uo pipefail
    cd /src && tar --exclude=./backend/target --exclude=./corpus --exclude=./.git \
      --exclude=./frontend/node_modules -cf - . | (cd /work && tar -xf -)
    vulkaninfo --summary > /out/vulkaninfo.txt 2>&1
    grep -m1 -i "deviceName" /out/vulkaninfo.txt || true
    cd /work/backend
    rm -rf target/golden-actual
    cargo test --locked --no-fail-fast "$@" -- --nocapture > /out/test.log 2>&1
    status=$?
    if [ -d target/golden-actual ]; then cp -r target/golden-actual/. /out/; fi
    exit $status
  ' bash "${args[@]}" || status=$?

grep -E "^golden |FAILED|test result: FAILED" "$out/test.log" || true
candidates=$(find "$out" -name '*.meta.json' | wc -l)
echo "== $candidates candidate(s) in backend/target/golden-lavapipe (log: test.log)"

if [ ${#accept[@]} -gt 0 ]; then
  echo "== accepting ${accept[*]}"
  "$python" "$(native "$root/scripts/accept-goldens.py")" "$(native "$out")" --only "${accept[@]}"
elif [ "$candidates" -gt 0 ]; then
  echo "Accept with: scripts/goldens-lavapipe.sh --accept CASE..."
fi
exit $status
