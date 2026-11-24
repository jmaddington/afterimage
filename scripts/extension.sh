#!/bin/zsh
# Build build/extension/{chrome,firefox} (load unpacked, or zip for the stores) and check that they parse.
set -euo pipefail
cd "$(dirname "$0")/.."
(cd extension && node --test >/dev/null) || { (cd extension && node --test); exit 1; }
rm -rf build/extension
for b in chrome firefox; do
  D=build/extension/$b; mkdir -p $D
  cp extension/manifest.$b.json $D/manifest.json; 
  # Content scripts aren't modules: drop the exports and run it all in one private scope.
  { echo "(() => {"; sed 's/^export //' extension/detect.js; cat extension/content.js; echo "})();"; } > $D/content.js
  { echo "(() => {"; sed 's/^export //' extension/detect.js; cat extension/background.js; echo "})();"; } > $D/background.js
  node --check $D/content.js; node --check $D/background.js
done
# Store uploads. The Chrome Web Store assigns its own id and refuses a manifest with "key" in it.
mkdir -p build/dist-extension
python3 -I - <<'PY'
import json
d = json.load(open("build/extension/chrome/manifest.json"))
d.pop("key", None)
import os, shutil
shutil.rmtree("build/extension/chrome-store", ignore_errors=True)
shutil.copytree("build/extension/chrome", "build/extension/chrome-store")
json.dump(d, open("build/extension/chrome-store/manifest.json", "w"), indent=2)
PY
rm -f build/dist-extension/*.zip
(cd build/extension/chrome-store && zip -qr ../../dist-extension/afterimage-chrome.zip .)
(cd build/extension/firefox && zip -qr ../../dist-extension/afterimage-firefox.zip .)
rm -rf build/extension/chrome-store
echo "✓ build/extension/chrome  build/extension/firefox  build/dist-extension/*.zip"
