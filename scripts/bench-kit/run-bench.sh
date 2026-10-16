#!/usr/bin/env bash
# Genzo M0 benchmark kit (Linux). Measures slider input to pixels in the real
# app (M0 step 0.E) and GPU time per slider step (0.D), then packs the results
# into one file to send back. Run from the kit folder: ./run-bench.sh
#
# What it does, in order:
#   1. Records the hardware and driver facts (no user name, host name or paths).
#   2. Checks that the app's libraries (WebKitGTK 4.1) are installed.
#   3. Runs the GPU benchmark in the command line tool.
#   4. Starts the app 2 or 3 times; each time it drags its own sliders for
#      about 2 minutes, saves a report and is closed by this script.
#   5. Writes genzo-bench-results-*.tar.gz next to this script.
#
# Options: --no-questions  skip the "did the image change?" questions
#          --app-timeout S  give up on one app run after S seconds (default 600)
#          --skip-cli       skip step 3
#          --only NAME      run one app variant: default, no-dmabuf or nvidia-offload
set -uo pipefail

KIT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
APP="$KIT/bin/genzo"
CLI="$KIT/bin/genzo-cli"
ASK=1
APP_TIMEOUT=600
CLI_BENCH=1
ONLY=""
while [ $# -gt 0 ]; do
  case "$1" in
    --no-questions) ASK=0 ;;
    --skip-cli) CLI_BENCH=0 ;;
    --only) ONLY="$2"; shift ;;
    --app-timeout) APP_TIMEOUT="$2"; shift ;;
    *) echo "unknown option $1"; exit 2 ;;
  esac
  shift
done
[ -t 0 ] || ASK=0

STAMP="$(date -u +%Y%m%dT%H%M%SZ)"
OUT="$KIT/results-$STAMP"
mkdir -p "$OUT"

say() { printf '\n== %s\n' "$*"; }
have() { command -v "$1" >/dev/null 2>&1; }
# Keep the user's home folder, user name and host name out of every file.
scrub() { sed -e "s#$HOME#~#g" -e "s#$(id -un)#user#g" -e "s#$(hostname 2>/dev/null || echo __nohost__)#host#g"; }

# ---------------------------------------------------------------- 1. system
say "Recording the system (results-$STAMP/system.txt)"
{
  echo "kit: $(cat "$KIT/VERSION" 2>/dev/null || echo unknown)"
  echo "date: $STAMP"
  . /etc/os-release 2>/dev/null && echo "os: $PRETTY_NAME"
  echo "kernel: $(uname -r) $(uname -m)"
  echo "session: ${XDG_SESSION_TYPE:-unknown}, desktop: ${XDG_CURRENT_DESKTOP:-unknown}"
  have lscpu && lscpu | grep -E '^(Model name|CPU\(s\)):'
  have free && free -g | awk '/^Mem:/ {print "ram_gb: " $2}'
  echo
  echo "## GPUs (lspci)"
  have lspci && lspci -nn | grep -Ei 'vga|3d|display'
  echo
  echo "## NVIDIA"
  if have nvidia-smi; then nvidia-smi --query-gpu=name,driver_version,memory.total,pstate --format=csv; else echo "no nvidia-smi"; fi
  echo
  echo "## OpenGL (glxinfo -B)"
  # WebKitGTK draws the page with OpenGL or EGL, on this GPU unless offloaded.
  if have glxinfo; then glxinfo -B 2>&1 | grep -E 'vendor|renderer|version' ; else echo "glxinfo not installed (mesa-utils)"; fi
  if have eglinfo; then eglinfo -B 2>&1 | grep -Ei 'platform|vendor|renderer' | head -20; fi
  echo
  echo "## Vulkan (vulkaninfo --summary)"
  if have vulkaninfo; then vulkaninfo --summary 2>/dev/null | sed -n '/Devices:/,$p'; else echo "vulkaninfo not installed (vulkan-tools)"; fi
  echo
  echo "## WebKitGTK"
  have pkg-config && pkg-config --modversion webkit2gtk-4.1 2>/dev/null | sed 's/^/pkg-config: /'
  have dpkg-query && dpkg-query -W -f='dpkg: ${Package} ${Version}\n' 'libwebkit2gtk-4.1-0' 2>/dev/null
  have rpm && rpm -q webkit2gtk4.1 2>/dev/null | sed 's/^/rpm: /'
  have pacman && pacman -Q webkit2gtk-4.1 2>/dev/null | sed 's/^/pacman: /'
  echo
  echo "## Displays"
  have xrandr && xrandr --current 2>/dev/null | grep -E ' connected|\*'
} 2>&1 | scrub > "$OUT/system.txt"
cat "$OUT/system.txt"

# ---------------------------------------------------------------- 2. libraries
say "Checking the app's libraries"
MISSING="$(ldd "$APP" 2>/dev/null | grep 'not found' || true)"
if [ -n "$MISSING" ]; then
  echo "The app cannot start, these libraries are missing:"
  echo "$MISSING"
  cat <<'EOF'

Install WebKitGTK 4.1 and run this script again:
  Ubuntu, Debian, Mint, Pop!_OS:  sudo apt install libwebkit2gtk-4.1-0 libgtk-3-0
  Fedora:                         sudo dnf install webkit2gtk4.1 gtk3
  Arch, Manjaro:                  sudo pacman -S webkit2gtk-4.1 gtk3
  openSUSE:                       sudo zypper install libwebkit2gtk-4_1-0
EOF
  echo "$MISSING" > "$OUT/missing-libraries.txt"
  exit 1
fi
echo "ok"

# ---------------------------------------------------------------- 3. cli
say "GPU benchmark in the command line tool (a few minutes, nothing to do)"
[ "$CLI_BENCH" = 1 ] || echo "skipped"
[ "$CLI_BENCH" = 1 ] && for size in "4032 3024 12mp" "8064 6048 48mp"; do
  set -- $size
  echo "-- $3"
  timeout 900 "$CLI" bench gpu "$1" "$2" 30 --json "$OUT/cli-$3.json" 2>&1 | scrub | tee "$OUT/cli-$3.txt"
  echo "exit: ${PIPESTATUS[0]}" >> "$OUT/cli-$3.txt"
done

# ---------------------------------------------------------------- 4. app
# Each variant is one way WebKitGTK can draw. The default is what users get.
# NVIDIA's driver sometimes shows a blank window with the default; the report
# counts frames drawn, and the question checks what the eye saw.
VARIANTS=("default|")
VARIANTS+=("no-dmabuf|WEBKIT_DISABLE_DMABUF_RENDERER=1")
GPU_COUNT="$(lspci 2>/dev/null | grep -Eic 'vga|3d|display' || true)"
if have nvidia-smi && [ "${GPU_COUNT:-0}" -gt 1 ]; then
  # Hybrid laptop: also draw the page on the NVIDIA GPU instead of the integrated one.
  prime="__NV_PRIME_RENDER_OFFLOAD=1 __GLX_VENDOR_LIBRARY_NAME=nvidia __VK_LAYER_NV_optimus=NVIDIA_only"
  for f in /usr/share/glvnd/egl_vendor.d/10_nvidia.json /etc/glvnd/egl_vendor.d/10_nvidia.json; do
    [ -f "$f" ] && prime="$prime __EGL_VENDOR_LIBRARY_FILENAMES=$f" && break
  done
  VARIANTS+=("nvidia-offload|$prime")
fi
if [ -n "$ONLY" ]; then
  for v in "${VARIANTS[@]}"; do [ "${v%%|*}" = "$ONLY" ] && VARIANTS=("$v") && break; done
  [ "${VARIANTS[0]%%|*}" = "$ONLY" ] || { echo "no variant $ONLY here"; exit 2; }
fi

say "App benchmark: ${#VARIANTS[@]} runs of about 2 minutes each"
cat <<'EOF'
A Genzo window opens and moves its own sliders. While it runs:
  - do not touch the mouse or keyboard,
  - do not minimize, move or cover the window,
  - watch whether the photo in the middle changes brightness and color.
EOF
[ "$ASK" = 1 ] && read -r -p "Press Enter to start. " _

for v in "${VARIANTS[@]}"; do
  name="${v%%|*}"
  envs="${v#*|}"
  report="$OUT/app-$name.json"
  log="$OUT/app-$name.log"
  say "Run '$name' ${envs:+($envs)}"
  started=$(date +%s)
  # shellcheck disable=SC2086
  env $envs GENZO_AUTOBENCH="$report" "$APP" > "$log" 2>&1 &
  pid=$!
  status="timeout"
  snapped=0
  while :; do
    # Mid-run, which processes use the NVIDIA GPU: shows whether the page
    # (WebKitWebProcess) is drawn there or on the integrated GPU.
    if [ "$snapped" = 0 ] && [ $(( $(date +%s) - started )) -ge 20 ] && have nvidia-smi; then
      nvidia-smi 2>&1 | scrub > "$OUT/app-$name.nvidia-smi.txt"
      snapped=1
    fi
    if [ -f "$report" ]; then status="ok"; break; fi
    if ! kill -0 "$pid" 2>/dev/null; then wait "$pid"; status="exited ($?)"; break; fi
    if [ $(( $(date +%s) - started )) -ge "$APP_TIMEOUT" ]; then break; fi
    sleep 1
  done
  sleep 1
  kill "$pid" 2>/dev/null; sleep 1; kill -9 "$pid" 2>/dev/null
  wait "$pid" 2>/dev/null
  seen="not asked"
  if [ "$ASK" = 1 ]; then
    read -r -p "Did the photo visibly change while the sliders moved? [y/n] " seen
  fi
  printf '{"variant": "%s", "env": "%s", "status": "%s", "seconds": %d, "imageVisiblyChanged": "%s"}\n' \
    "$name" "$envs" "$status" "$(( $(date +%s) - started ))" "$seen" > "$OUT/app-$name.run.json"
  scrub < "$log" > "$log.tmp" && mv "$log.tmp" "$log"
  echo "status: $status"
done

# ---------------------------------------------------------------- 5. pack
say "Summary"
if have python3; then
  python3 - "$OUT" <<'EOF'
import json, pathlib, sys
out = pathlib.Path(sys.argv[1])
for f in sorted(out.glob("app-*.json")):
    if f.name.endswith(".run.json"):
        continue
    r = json.loads(f.read_text())
    print(f"{f.stem}: engine on {r['engine']['adapter']['name']}, page on {r['page']['webgl']}")
    for c in r["cases"]:
        zoom = "Fit" if c["zoom"] == "fit" else "1:1"
        if c.get("error"):
            res = c["error"]
        elif c["n"]:
            b = c["breakdown"]
            fmt = lambda s: "n/a" if s["p50"] is None else f"{s['p50']:.0f}/{s['p95']:.0f}"
            res = (f"p50 {c['p50']:.1f} ms, p95 {c['p95']:.1f} ms, n {c['n']}\n"
                   f"      one request p50/p95: {fmt(b['sendToDraw'])} ms = send to event {fmt(b['sendToEvent'])}"
                   f" (engine {fmt(b['engine'])}) + event to draw {fmt(b['eventToDraw'])}"
                   f" (fetch {fmt(b['fetch'])}, {b['fetchKB']['p50']} KB)")
        else:
            b = c["breakdown"]
            res = (f"no samples: {b['engine']['n']} engine frames, {b['framesDrawn']} drawn"
                   + (f", viewport error: {b['viewportError']}" if b["viewportError"] else ""))
        print(f"  {c['source']:16} {c['slider']:8} {zoom:3}  {res}")
        for err in c.get("pageErrors", []):
            print(f"      page error: {err}")
EOF
fi
ARCHIVE="$KIT/genzo-bench-results-$STAMP.tar.gz"
tar -czf "$ARCHIVE" -C "$KIT" "results-$STAMP"
say "Done. Send this file back:"
echo "$ARCHIVE"
