#!/usr/bin/env bash
# Genzo M0 benchmark kit (Linux and macOS). Measures slider input to pixels in the real
# app (M0 step 0.E) and GPU time per slider step (0.D), then packs the results
# into one file to send back. Run from the kit folder: ./run-bench.sh
#
# What it does, in order:
#   1. Records the hardware and driver facts (no user name, host name or paths).
#   2. Linux: checks that the app's libraries (WebKitGTK 4.1) are installed.
#      macOS: clears the download quarantine flag on this folder, so the
#      unsigned test build can start.
#   3. Runs the GPU benchmark in the command line tool.
#   4. Starts the app 2 or 3 times (once on macOS); each time it drags its own sliders for
#      about a minute, saves a report and is closed by this script.
#   5. Writes genzo-bench-results-*.tar.gz next to this script.
#
# Options: --no-questions  skip the "did the image change?" questions
#          --app-timeout S  give up on one app run after S seconds (default 600)
#          --skip-cli       skip step 3
#          --only NAME      run one app variant: default, no-dmabuf, or nvidia-offload
#                           (offload only runs when asked: it aborted at start on the
#                           RTX 3050 laptop, and the engine uses the NVIDIA GPU anyway)
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
# The commit the kit was built from, in every results name and file.
COMMIT="$(sed -n 's/^commit //p' "$KIT/VERSION" 2>/dev/null)"
COMMIT="${COMMIT:-unknown}"
case "$(uname -s)" in Darwin) OS_NAME=macos ;; *) OS_NAME=linux ;; esac
OUT="$KIT/results-$OS_NAME-$COMMIT-$STAMP"
mkdir -p "$OUT"

say() { printf '\n== %s\n' "$*"; }
have() { command -v "$1" >/dev/null 2>&1; }
# Keep the user's home folder, user name and host name out of every file.
# perl takes them as literal text (\Q..\E), and the names only as whole words
# (between non-word characters such as space, "-" or "."), so a short user name
# like "a" does not rewrite the rest of the output. The host name goes first:
# it often contains the user name.
scrub() {
  SCRUB_HOME="$HOME" SCRUB_USER="$(id -un)" SCRUB_HOST="$(hostname 2>/dev/null)" perl -pe '
    BEGIN { ($h, $u, $n) = @ENV{qw(SCRUB_HOME SCRUB_USER SCRUB_HOST)} }
    s/\Q$h\E/~/g if length $h;
    s/(?<!\w)\Q$n\E(?!\w)/host/g if length $n;
    s/(?<!\w)\Q$u\E(?!\w)/user/g if length $u;
  '
}
# macOS has no `timeout`; perl's alarm does the same.
with_timeout() { if have timeout; then timeout "$@"; else perl -e 'alarm shift; exec @ARGV' "$@"; fi; }
OS="$(uname -s)"

# ---------------------------------------------------------------- 1. system
say "Recording the system ($(basename "$OUT")/system.txt)"
linux_system() {
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
}

mac_system() {
  echo "os: macOS $(sw_vers -productVersion) ($(sw_vers -buildVersion))"
  echo "kernel: $(uname -r) $(uname -m)"
  echo "cpu: $(sysctl -n machdep.cpu.brand_string), $(sysctl -n hw.ncpu) cores"
  echo "ram_gb: $(( $(sysctl -n hw.memsize) / 1073741824 ))"
  echo "power: $(pmset -g batt | head -1)"
  pmset -g | grep -i lowpowermode
  echo
  echo "## GPU and displays (no serial numbers)"
  system_profiler SPDisplaysDataType 2>/dev/null \
    | grep -E 'Chipset Model|Total Number of Cores|Metal|Resolution|UI Looks like|Refresh|Display Type|Main Display|Connection Type' \
    | grep -v -i serial
  echo
  echo "## WebKit (WKWebView)"
  echo "webkit: $(defaults read /System/Library/Frameworks/WebKit.framework/Resources/Info CFBundleVersion 2>/dev/null)"
  echo "safari: $(defaults read /Applications/Safari.app/Contents/Info CFBundleShortVersionString 2>/dev/null)"
}

{
  echo "kit: $(cat "$KIT/VERSION" 2>/dev/null || echo unknown)"
  echo "date: $STAMP"
  if [ "$OS" = Darwin ]; then mac_system; else linux_system; fi
} 2>&1 | scrub > "$OUT/system.txt"
cat "$OUT/system.txt"

# ---------------------------------------------------------------- 2. libraries
if [ "$OS" = Darwin ]; then
  say "Allowing the test build to start"
  # Files that came from a download carry com.apple.quarantine, and macOS
  # refuses to run an unsigned binary that has it. Only this folder is touched.
  xattr -dr com.apple.quarantine "$KIT" 2>/dev/null || true
  echo "ok"
fi
[ "$OS" = Darwin ] || say "Checking the app's libraries"
MISSING=""
[ "$OS" = Darwin ] || MISSING="$(ldd "$APP" 2>/dev/null | grep 'not found' || true)"
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
[ "$OS" = Darwin ] || echo "ok"

# ---------------------------------------------------------------- 3. cli
say "GPU benchmark in the command line tool (a few minutes, nothing to do)"
[ "$CLI_BENCH" = 1 ] || echo "skipped"
[ "$CLI_BENCH" = 1 ] && for size in "4032 3024 12mp" "8064 6048 48mp"; do
  set -- $size
  echo "-- $3"
  with_timeout 900 "$CLI" bench gpu "$1" "$2" 30 --json "$OUT/cli-$3.json" 2>&1 | scrub | tee "$OUT/cli-$3.txt"
  echo "exit: ${PIPESTATUS[0]}" >> "$OUT/cli-$3.txt"
done

# ---------------------------------------------------------------- 4. app
# Each variant is one way WebKitGTK can draw. The default is what users get.
# NVIDIA's driver sometimes shows a blank window with the default; the report
# counts frames drawn, and the question checks what the eye saw.
# macOS has one way: WKWebView on Metal.
VARIANTS=("default|")
[ "$OS" = Darwin ] || VARIANTS+=("no-dmabuf|WEBKIT_DISABLE_DMABUF_RENDERER=1")
GPU_COUNT="$(lspci 2>/dev/null | grep -Eic 'vga|3d|display' || true)"
if [ "$ONLY" = nvidia-offload ] && [ "$OS" != Darwin ] && have nvidia-smi && [ "${GPU_COUNT:-0}" -gt 1 ]; then
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

say "App benchmark: ${#VARIANTS[@]} run(s) of about a minute each"
cat <<'EOF'
A Genzo window opens and moves its own sliders. While it runs:
  - do not touch the mouse or keyboard,
  - do not minimize, move or cover the window,
  - watch whether the photo in the middle changes brightness and color,
  - stay at the computer: if the screen dims or locks, the run is lost.
If the window opens behind this terminal, click its title bar once to bring
it to the front, then leave it alone.
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
  # Keep the screen on: with the display blanked or locked, the webview stops
  # drawing and the run measures nothing (seen on the dry run).
  awake=""
  if [ "$OS" = Darwin ]; then
    caffeinate -d -i -w "$pid" & awake=$!
  elif have gnome-session-inhibit; then
    gnome-session-inhibit --inhibit idle:suspend --reason "Genzo benchmark" sleep "$APP_TIMEOUT" >/dev/null 2>&1 & awake=$!
  elif have systemd-inhibit; then
    systemd-inhibit --what=idle:sleep --who="Genzo benchmark" --why="measuring" sleep "$APP_TIMEOUT" >/dev/null 2>&1 & awake=$!
  fi
  # GPU memory of the app every 250 ms, for its peak: 48 MP ran out of
  # memory on 4 GB before the fix in kit aa6a785.
  # Per process where the driver lists the app (Vulkan apps usually show as
  # C+G), and the whole card as a fallback.
  samplers=""
  if have nvidia-smi; then
    nvidia-smi --query-compute-apps=timestamp,pid,used_memory --format=csv,noheader,nounits -lms 250       > "$OUT/app-$name.gpu-mem.csv" 2>/dev/null & samplers="$!"
    nvidia-smi --query-gpu=timestamp,memory.used --format=csv,noheader,nounits -lms 250       > "$OUT/app-$name.gpu-mem-card.csv" 2>/dev/null & samplers="$samplers $!"
  fi
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
  [ -n "$awake" ] && kill "$awake" 2>/dev/null
  # shellcheck disable=SC2086
  [ -n "$samplers" ] && kill $samplers 2>/dev/null
  if [ -n "$samplers" ]; then
    app_peak="$(awk -F', *' -v pid="$pid" '$2 == pid && $3 + 0 > m { m = $3 + 0 } END { print m + 0 }' "$OUT/app-$name.gpu-mem.csv" 2>/dev/null)"
    card_peak="$(awk -F', *' '$2 + 0 > m { m = $2 + 0 } END { print m + 0 }' "$OUT/app-$name.gpu-mem-card.csv" 2>/dev/null)"
    echo "peak GPU memory: app ${app_peak:-0} MiB (0: not listed by the driver), whole card ${card_peak:-0} MiB"       | tee "$OUT/app-$name.gpu-mem-peak.txt"
  fi
  seen="not asked"
  if [ "$ASK" = 1 ]; then
    read -r -p "Did the photo visibly change while the sliders moved? [y/n] " seen
  fi
  printf '{"kitCommit": "%s", "variant": "%s", "env": "%s", "status": "%s", "seconds": %d, "imageVisiblyChanged": "%s"}\n' \
    "$COMMIT" "$name" "$envs" "$status" "$(( $(date +%s) - started ))" "$seen" > "$OUT/app-$name.run.json"
  scrub < "$log" > "$log.tmp" && mv "$log.tmp" "$log"
  echo "status: $status"
done

# ---------------------------------------------------------------- 5. pack
say "Summary"
# On a Mac without the developer tools, python3 is a stub that pops up an
# install dialog; only use it when the tools are there.
if have python3 && { [ "$OS" != Darwin ] || xcode-select -p >/dev/null 2>&1; }; then
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
        if c.get("pageHidden"):
            print("      the page was hidden during this case: these numbers are suspect")
        for err in c.get("pageErrors", []):
            print(f"      page error: {err}")
    col = r.get("color")
    if col:
        s = col["support"]
        print(f"  colors: display gamut {s['displayGamut']}, P3 canvas {'yes' if s['drawingBufferColorSpace'] else 'NO'}")
        for p in col["paths"]:
            print(f"    {p['format']:15} max error {p['maxError']:.1f}  {'ok' if p['pass'] else 'WRONG'}")
        if col.get("error"):
            print(f"    color check failed: {col['error']}")
EOF
fi
ARCHIVE="$KIT/genzo-bench-results-$OS_NAME-$COMMIT-$STAMP.tar.gz"
# COPYFILE_DISABLE: no macOS ._ metadata files in the archive.
COPYFILE_DISABLE=1 tar -czf "$ARCHIVE" -C "$KIT" "$(basename "$OUT")"
say "Done. Send this file back:"
echo "$ARCHIVE"
