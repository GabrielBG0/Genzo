#!/usr/bin/env bash
# Builds the benchmark kit for this OS: the app (release, no installer), the
# cli and run-bench.sh, packed as dist/genzo-bench-<os>-<arch>.tar.gz. The
# "Bench kit" workflow runs it on ubuntu-22.04, so the Linux binaries start on
# any distro with glibc 2.35 or newer (a build on a newer distro only runs there
# and newer), and on macos-15 with MACOSX_DEPLOYMENT_TARGET=13.0.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
case "$(uname -s)" in
  Darwin) NAME="genzo-bench-macos-$(uname -m)"
    BUILT_ON="macOS $(sw_vers -productVersion), target ${MACOSX_DEPLOYMENT_TARGET:-default}" ;;
  *) NAME="genzo-bench-linux-$(uname -m)"
    BUILT_ON="$(. /etc/os-release && echo "$PRETTY_NAME"), glibc $(ldd --version | head -1 | awk '{print $NF}')" ;;
esac
KIT="$ROOT/dist/$NAME"

(cd "$ROOT/frontend" && pnpm install --frozen-lockfile && pnpm app:build --no-bundle)
(cd "$ROOT/backend" && cargo build --locked --release -p genzo-cli)

rm -rf "$KIT"
mkdir -p "$KIT/bin"
cp "$ROOT/shell/target/release/genzo" "$ROOT/backend/target/release/genzo-cli" "$KIT/bin/"
cp "$ROOT/scripts/bench-kit/run-bench.sh" "$ROOT/scripts/bench-kit/README.txt" "$KIT/"
chmod +x "$KIT/run-bench.sh" "$KIT/bin/"*
{
  echo "commit $(git -C "$ROOT" rev-parse --short HEAD)$(git -C "$ROOT" diff --quiet HEAD -- || echo '-dirty')"
  echo "contract $(grep -o '"[0-9.]*"' "$ROOT/backend/crates/model/src/contract.rs" | head -1 | tr -d '"')"
  echo "built on $BUILT_ON"
} > "$KIT/VERSION"

COPYFILE_DISABLE=1 tar -czf "$ROOT/dist/$NAME.tar.gz" -C "$ROOT/dist" "$NAME"
echo "$ROOT/dist/$NAME.tar.gz"
