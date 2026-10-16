#!/usr/bin/env bash
# Builds the Linux benchmark kit: the app (release, no installer), the cli and
# run-bench.sh, packed as dist/genzo-bench-linux-<arch>.tar.gz. CI runs this on
# ubuntu-22.04 (the "Bench kit" workflow) so the binaries start on any distro
# with glibc 2.35 or newer; a build on a newer distro only runs there and newer.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
NAME="genzo-bench-linux-$(uname -m)"
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
  echo "built on $(. /etc/os-release && echo "$PRETTY_NAME"), glibc $(ldd --version | head -1 | awk '{print $NF}')"
} > "$KIT/VERSION"

tar -czf "$ROOT/dist/$NAME.tar.gz" -C "$ROOT/dist" "$NAME"
echo "$ROOT/dist/$NAME.tar.gz"
