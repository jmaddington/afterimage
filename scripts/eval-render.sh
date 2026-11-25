#!/bin/bash
# Renders the eval screens (eval/cases/*.html and the public pages in eval/cases.json) to eval/png at Retina size.
# Needs a Chrome that can run headless: CHROME=/path/to/chrome ./scripts/eval-render.sh
set -euo pipefail
cd "$(dirname "$0")/.."
CHROME="${CHROME:-/Applications/Google Chrome.app/Contents/MacOS/Google Chrome}"
mkdir -p eval/png
PROFILE="$(mktemp -d)"
trap 'rm -rf "$PROFILE"' EXIT
shot() { "$CHROME" --headless=new --disable-gpu --hide-scrollbars --no-first-run --user-data-dir="$PROFILE" --window-size=1440,900 --force-device-scale-factor=2 --virtual-time-budget=8000 --screenshot="$PWD/eval/png/$1.png" "$2" >/dev/null 2>&1 || echo "could not render $1"; }
for f in eval/cases/*.html; do n="$(basename "$f" .html)"; shot "$n" "file://$PWD/$f"; done
python3 -I - <<'P' | while IFS=$'\t' read -r n u; do shot "$n" "$u"; done
import json
for k, v in json.load(open("eval/cases.json")).items():
    if "url" in v: print(k, v["url"], sep="\t")
P
ls eval/png | wc -l | xargs echo "pictures:"
