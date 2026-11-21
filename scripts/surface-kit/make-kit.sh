#!/usr/bin/env bash
# Builds the native-surface kit for this OS: the probe app (release, no
# installer), the run script and the notes, packed as
# dist/genzo-surface-<os>-<arch>-<commit>.tar.gz. The commit in the name, in the
# kit's VERSION file and in every results file ties a measurement to the code it
# measured; a working tree with changes gets "-dirty". Run by the "Surface kit"
# workflow on macos-15 (MACOSX_DEPLOYMENT_TARGET=13.0).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
COMMIT="$(git -C "$ROOT" rev-parse --short=7 HEAD)$([ -z "$(git -C "$ROOT" status --porcelain)" ] || echo '-dirty')"
case "$(uname -s)" in
  Darwin) OS=macos
    BUILT_ON="macOS $(sw_vers -productVersion), target ${MACOSX_DEPLOYMENT_TARGET:-default}" ;;
  *) echo "the kit's run script is for macOS only"; exit 2 ;;
esac
NAME="genzo-surface-$OS-$(uname -m)-$COMMIT"
KIT="$ROOT/dist/$NAME"

(cd "$ROOT/spikes/tauri-surface-probe" && cargo build --locked --release)

rm -rf "$KIT"
mkdir -p "$KIT/bin"
cp "$ROOT/spikes/tauri-surface-probe/target/release/tauri-surface-probe" "$KIT/bin/"
cp "$ROOT/scripts/surface-kit/README.txt" "$ROOT/scripts/surface-kit/run.sh" "$KIT/"
chmod +x "$KIT/run.sh" "$KIT/bin/"*
{
  echo "commit $COMMIT"
  echo "built on $BUILT_ON"
} > "$KIT/VERSION"

ARCHIVE="$ROOT/dist/$NAME.tar.gz"
# COPYFILE_DISABLE: no macOS ._ metadata files in the archive.
COPYFILE_DISABLE=1 tar -czf "$ARCHIVE" -C "$ROOT/dist" "$NAME"
echo "$ARCHIVE"
