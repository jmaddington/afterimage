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
echo "✓ build/extension/chrome  build/extension/firefox"
