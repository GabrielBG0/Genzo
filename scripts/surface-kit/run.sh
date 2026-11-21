#!/usr/bin/env bash
# Genzo native-surface kit (macOS). Runs the spike of docs/native-surface-spike.md
# on this Mac: does a Metal surface under a transparent webview show, how fast
# is a full-size frame, and what the screen does with the frame's colors under
# different color-space tags. It records the facts, runs the cases, takes a
# screenshot of the test window for each, and packs everything into one file.
# Run from the kit folder: ./run.sh
#
# Options: --app-timeout S  give up on one case after S seconds (default 120)
#          --no-screenshots  only the timings (no Screen Recording permission needed)
set -uo pipefail

KIT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
APP="$KIT/bin/tauri-surface-probe"
APP_TIMEOUT=120
SHOTS=1
while [ $# -gt 0 ]; do
  case "$1" in
    --app-timeout) APP_TIMEOUT="$2"; shift ;;
    --no-screenshots) SHOTS=0 ;;
    *) echo "unknown option $1"; exit 2 ;;
  esac
  shift
done

STAMP="$(date -u +%Y%m%dT%H%M%SZ)"
COMMIT="$(sed -n 's/^commit //p' "$KIT/VERSION" 2>/dev/null)"
COMMIT="${COMMIT:-unknown}"
OUT="$KIT/results-macos-$COMMIT-$STAMP"
mkdir -p "$OUT"

say() { printf '\n== %s\n' "$*"; }
have() { command -v "$1" >/dev/null 2>&1; }
# Keep the user's home folder, user name and host name out of every file.
scrub() {
  SCRUB_HOME="$HOME" SCRUB_USER="$(id -un)" SCRUB_HOST="$(hostname 2>/dev/null)" perl -pe '
    BEGIN { ($h, $u, $n) = @ENV{qw(SCRUB_HOME SCRUB_USER SCRUB_HOST)} }
    s/\Q$h\E/~/g if length $h;
    s/(?<!\w)\Q$n\E(?!\w)/host/g if length $n;
    s/(?<!\w)\Q$u\E(?!\w)/user/g if length $u;
  '
}

# ---------------------------------------------------------------- 1. system
say "Recording the system"
{
  echo "kit: $(cat "$KIT/VERSION" 2>/dev/null || echo unknown)"
  echo "date: $STAMP"
  echo "os: macOS $(sw_vers -productVersion) ($(sw_vers -buildVersion))"
  echo "kernel: $(uname -r) $(uname -m)"
  echo "cpu: $(sysctl -n machdep.cpu.brand_string), $(sysctl -n hw.ncpu) cores"
  echo "ram_gb: $(( $(sysctl -n hw.memsize) / 1073741824 ))"
  echo "power: $(pmset -g batt | head -1)"
  pmset -g | grep -i lowpowermode
  echo
  echo "## GPU and displays (no serial numbers)"
  system_profiler SPDisplaysDataType 2>/dev/null \
    | grep -E 'Chipset Model|Total Number of Cores|Metal|Resolution|UI Looks like|Refresh|Display Type|Main Display|Connection Type|Color Profile|Mirror' \
    | grep -v -i serial
  echo
  echo "## Color profile of the main display (name only)"
  defaults read com.apple.ColorSync DisplayProfiles 2>/dev/null | grep -E 'Color LCD|\.icc|\.icm' | head -5 || true
  echo
  echo "## True Tone and Night Shift (should be off for the color part)"
  # These keys may not exist on every Mac; "does not exist" is a fine answer.
  defaults read com.apple.CoreBrightness 2>/dev/null | grep -iE 'CBBlueReductionStatus|TrueTone|CBColorAdaptation' | head -8 || true
} 2>&1 | scrub > "$OUT/system.txt"
cat "$OUT/system.txt"

# ---------------------------------------------------------------- 2. start
say "Allowing the test build to start"
# Files that came from a download carry com.apple.quarantine, and macOS refuses
# to run an unsigned binary that has it. Only this folder is touched.
xattr -dr com.apple.quarantine "$KIT" 2>/dev/null || true
chmod +x "$APP" 2>/dev/null || true
echo "ok"

if [ "$SHOTS" = 1 ]; then
  say "Screenshots"
  echo "The color part needs screenshots. The first time, macOS asks whether"
  echo "Terminal may record the screen: allow it (System Settings > Privacy &"
  echo "Security > Screen & System Audio Recording), then run this script again if it"
  echo "asked while the test was already running."
fi

# One case: run the app with the given environment, wait for a marker line in
# its output, optionally screenshot the test window, let it finish.
#   run_case NAME MARKER SHOT VAR=VALUE...
run_case() {
  local name="$1" marker="$2" shot="$3"; shift 3
  local log="$OUT/$name.txt" err="$OUT/$name.err"
  say "Case: $name"
  env "$@" "$APP" > "$log" 2> "$err" &
  local pid=$!
  local waited=0
  # Wait for the marker, the app exiting, or the timeout.
  while kill -0 "$pid" 2>/dev/null && ! grep -q "$marker" "$log" 2>/dev/null && [ "$waited" -lt "$APP_TIMEOUT" ]; do
    sleep 0.5; waited=$((waited + 1))
  done
  if [ "$shot" = 1 ] && [ "$SHOTS" = 1 ] && kill -0 "$pid" 2>/dev/null; then
    sleep 2
    capture "$OUT/$name.png" "$log"
  fi
  # The app ends itself after its hold time; give it the rest of the timeout.
  while kill -0 "$pid" 2>/dev/null && [ "$waited" -lt "$((APP_TIMEOUT * 2))" ]; do
    sleep 0.5; waited=$((waited + 1))
  done
  if kill -0 "$pid" 2>/dev/null; then echo "(still running, stopping it)"; kill "$pid" 2>/dev/null; sleep 1; kill -9 "$pid" 2>/dev/null; fi
  wait "$pid" 2>/dev/null
  scrub < "$log" > "$log.s" && mv "$log.s" "$log"
  scrub < "$err" > "$err.s" && mv "$err.s" "$err"
  # The part of the output worth reading here.
  grep -E 'adapter|surface:|step|acquire|submit|gpu|settled|presented|sample|frame |window|monitor' "$log" | cut -c1-200 | head -30
}

# Screenshot of the test window only, from the position the app printed
# (physical pixels / scale = points). Falls back to the whole screen.
capture() {
  local png="$1" log="$2" region=""
  local line scale x y w h
  scale="$(sed -n 's/^window scale \([0-9.]*\).*/\1/p' "$log" | head -1)"
  line="$(grep '^window inner position' "$log" | head -1)"
  x="$(echo "$line" | sed -n 's/.*position PhysicalPosition { x: \(-*[0-9]*\), y: \(-*[0-9]*\) }.*/\1/p')"
  y="$(echo "$line" | sed -n 's/.*position PhysicalPosition { x: \(-*[0-9]*\), y: \(-*[0-9]*\) }.*/\2/p')"
  w="$(echo "$line" | sed -n 's/.*inner size PhysicalSize { width: \([0-9]*\), height: \([0-9]*\) }.*/\1/p')"
  h="$(echo "$line" | sed -n 's/.*inner size PhysicalSize { width: \([0-9]*\), height: \([0-9]*\) }.*/\2/p')"
  if [ -n "$scale" ] && [ -n "$x" ] && [ -n "$w" ]; then
    region="$(perl -e 'printf("%d,%d,%d,%d", $ARGV[0]/$ARGV[4], $ARGV[1]/$ARGV[4], $ARGV[2]/$ARGV[4], $ARGV[3]/$ARGV[4])' "$x" "$y" "$w" "$h" "$scale")"
  fi
  # Only ever the test window: with no usable position, no screenshot is taken
  # (a whole-screen one could show other windows).
  if [ -z "$region" ]; then
    echo "no screenshot: the window's position was not printed ($png)" | tee -a "$OUT/screenshot-errors.txt"
    return
  fi
  if screencapture -x -R"$region" "$png" 2>>"$OUT/screenshot-errors.txt"; then
    echo "screenshot of region $region"
  else
    echo "no screenshot: screencapture failed for region $region ($png)" | tee -a "$OUT/screenshot-errors.txt"
  fi
}

# ---------------------------------------------------------------- 3. cases
# a. Is the surface visible through the webview's hole, at the default pacing?
run_case "a-visible" "settled frame" 1 HARNESS=native STEPS=60 HOLD=8

# b. Timing of full-size drag steps in a 2560 x 1440 view, on a 24 MP and a
#    48 MP photo, with the display's pacing and without.
run_case "b-step-24mp-mailbox" "settled frame" 0 HARNESS=native STEPS=300 VIEW=2560x1440 PHOTO_MP=24
run_case "b-step-24mp-fifo" "settled frame" 0 HARNESS=native STEPS=300 VIEW=2560x1440 PHOTO_MP=24 GENZO_PRESENT=fifo
run_case "b-step-48mp-mailbox" "settled frame" 0 HARNESS=native STEPS=200 VIEW=2560x1440 PHOTO_MP=48
run_case "b-step-readback" "settled frame" 0 HARNESS=readback STEPS=300 VIEW=2560x1440 PHOTO_MP=24

# c. Color: the frame's own bytes, then the screen under each color-space tag.
for target in srgb p3; do
  run_case "c-bytes-$target" "frame origin" 0 HARNESS=color FRAME_TARGET="$target" HOLD=1
done
for target in srgb p3; do
  for tag in auto srgb p3; do
    run_case "c-screen-$target-$tag" "presented:" 1 HARNESS=color NATIVE=1 FRAME_TARGET="$target" COLORSPACE="$tag" HOLD=8
  done
done

# d. Following the window: a layout that moves for 20 seconds (optional to
#    touch; drag the window's corner while it runs if you like).
say "Case: d-follow-layout (20 seconds; drag the window's corner if you like)"
env HARNESS=native ANIM=1 PAGE=anim.html STEPS=5 "$APP" > "$OUT/d-follow-layout.txt" 2> "$OUT/d-follow-layout.err" &
DPID=$!
sleep 20
kill "$DPID" 2>/dev/null; sleep 1; kill -9 "$DPID" 2>/dev/null; wait "$DPID" 2>/dev/null
scrub < "$OUT/d-follow-layout.txt" > "$OUT/d-follow-layout.s" && mv "$OUT/d-follow-layout.s" "$OUT/d-follow-layout.txt"

# ---------------------------------------------------------------- 4. pack
say "Packing"
ARCHIVE="$KIT/genzo-surface-results-macos-$COMMIT-$STAMP.tar.gz"
if ! COPYFILE_DISABLE=1 tar -czf "$ARCHIVE" -C "$KIT" "$(basename "$OUT")"; then
  echo
  echo "Packing failed (is the disk full?). The results are in this folder:"
  echo "  $OUT"
  echo "Free some space and run this to pack them again:"
  echo "  tar -czf results.tar.gz -C \"$KIT\" \"$(basename "$OUT")\""
  exit 1
fi
echo
echo "Done. Please send this file back:"
echo "  $ARCHIVE"
du -h "$ARCHIVE" | cut -f1
