#!/usr/bin/env bash
# Builds the benchmark kit for this OS: the app (release, no installer), the
# cli and the run script, packed as dist/genzo-bench-<os>-<arch>-<commit>.tar.gz
# (.zip on Windows). The commit in the name, in the kit's VERSION file and in
# every results file ties a measurement to the code it measured; a build from
# a working tree with changes gets "-dirty" after it.
#
# The "Bench kit" workflow runs this on ubuntu-22.04, so the Linux binaries
# start on any distro with glibc 2.35 or newer (a build on a newer distro only
# runs there and newer), on macos-15 with MACOSX_DEPLOYMENT_TARGET=13.0, and on
# windows-2022 (Git Bash).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# Untracked files count too: a build can pick up a new source file.
COMMIT="$(git -C "$ROOT" rev-parse --short=7 HEAD)$([ -z "$(git -C "$ROOT" status --porcelain)" ] || echo '-dirty')"
EXE=""
case "$(uname -s)" in
  Darwin) OS=macos
    BUILT_ON="macOS $(sw_vers -productVersion), target ${MACOSX_DEPLOYMENT_TARGET:-default}" ;;
  MINGW* | MSYS* | CYGWIN*) OS=windows EXE=.exe
    BUILT_ON="Windows $(cmd //c ver | tr -d '\r' | grep -o '[0-9][0-9.]*')" ;;
  *) OS=linux
    BUILT_ON="$(. /etc/os-release && echo "$PRETTY_NAME"), glibc $(ldd --version | head -1 | awk '{print $NF}')" ;;
esac
NAME="genzo-bench-$OS-$(uname -m)-$COMMIT"
KIT="$ROOT/dist/$NAME"

(cd "$ROOT/frontend" && pnpm install --frozen-lockfile && pnpm app:build --no-bundle)
(cd "$ROOT/backend" && cargo build --locked --release -p genzo-cli)

rm -rf "$KIT"
mkdir -p "$KIT/bin"
cp "$ROOT/shell/target/release/genzo$EXE" "$ROOT/backend/target/release/genzo-cli$EXE" "$KIT/bin/"
cp "$ROOT/scripts/bench-kit/README.txt" "$KIT/"
if [ "$OS" = windows ]; then
  cp "$ROOT/scripts/bench-kit/run-bench.ps1" "$ROOT/scripts/bench-kit/run-bench.cmd" "$KIT/"
else
  cp "$ROOT/scripts/bench-kit/run-bench.sh" "$KIT/"
  chmod +x "$KIT/run-bench.sh" "$KIT/bin/"*
fi
{
  echo "commit $COMMIT"
  echo "contract $(grep -o '"[0-9.]*"' "$ROOT/backend/crates/model/src/contract.rs" | head -1 | tr -d '"')"
  echo "built on $BUILT_ON"
} > "$KIT/VERSION"

if [ "$OS" = windows ]; then
  ARCHIVE="$ROOT/dist/$NAME.zip"
  rm -f "$ARCHIVE"
  (cd "$ROOT/dist" && powershell -NoProfile -Command "Compress-Archive -Path '$NAME' -DestinationPath '$NAME.zip'")
else
  ARCHIVE="$ROOT/dist/$NAME.tar.gz"
  # COPYFILE_DISABLE: no macOS ._ metadata files in the archive.
  COPYFILE_DISABLE=1 tar -czf "$ARCHIVE" -C "$ROOT/dist" "$NAME"
fi
echo "$ARCHIVE"
