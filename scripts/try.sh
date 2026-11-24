#!/bin/sh
# Run the real app without a display and without touching your own settings, and take
# a picture of what it shows.
#
#   scripts/try.sh out.png [options for bilby...]
#
#   scripts/try.sh home.png --demo --ui tv --size 1920x1080
#   scripts/try.sh page.png --demo --open github:kestrel-sh/kestrel --script "wait wait ok"
#   scripts/try.sh real.png --page search --search firefox      (the real sources)
#
# Everything the app writes goes into a throw-away home, kept between runs so that a
# second run starts where the first left off. Remove it to start clean:
#   rm -rf "${TMPDIR:-/tmp}/bilby-try"
#
# Needs xvfb-run. BILBY=path/to/bilby runs another build (default: target/debug/bilby).
set -eu

[ $# -ge 1 ] || { sed -n '2,15p' "$0" | sed 's/^# \{0,1\}//'; exit 2; }
out=$1; shift
root=$(cd "$(dirname "$0")/.." && pwd)
bilby=${BILBY:-$root/target/debug/bilby}
home=${TMPDIR:-/tmp}/bilby-try
[ -x "$bilby" ] || { echo "No program at $bilby: run 'cargo build -p bilby' first." >&2; exit 1; }
command -v xvfb-run >/dev/null || { echo "xvfb-run is not installed." >&2; exit 1; }

mkdir -p "$home"
# Past the first-run welcome, which would cover the page.
[ -f "$home/state.json" ] || echo '{"settings":{"welcomed":true}}' > "$home/state.json"

env -u WAYLAND_DISPLAY -u XDG_DATA_HOME \
    HOME="$home" XDG_CONFIG_HOME="$home/.config" XDG_CACHE_HOME="$home/.cache" \
    XDG_DOWNLOAD_DIR="$home/Downloads" SLINT_BACKEND=winit-software \
    timeout "${TIMEOUT:-120}" xvfb-run -a "$bilby" --state "$home/state.json" \
    --screenshot "$out" --report "${out%.png}.txt" "$@"
echo "Picture: $out   What was on screen: ${out%.png}.txt"
