#!/bin/zsh
# CI: fetch create-dmg v1.3.0 at a fixed commit, check it, and put it on PATH. Runs beside signing keys, so nothing floating.
set -euo pipefail
: "${RUNNER_TEMP:?}" "${GITHUB_PATH:?}"
COMMIT=a2b71d0fda6d0df2a86dc7f67082d4d73e84c59f
SHA256=36577b966f16c12dd78d5bb5107c2ae3d069b044226b6ebbffa6a434ce142d0a
D="$RUNNER_TEMP/create-dmg"; mkdir -p "$D"
curl -fsSL -o "$D/src.tgz" "https://github.com/create-dmg/create-dmg/archive/$COMMIT.tar.gz"
echo "$SHA256  $D/src.tgz" | shasum -a 256 -c - >/dev/null || { echo "::error title=create-dmg changed::checksum mismatch"; exit 1; }
tar -xzf "$D/src.tgz" -C "$D" --strip-components 1
echo "$D" >> "$GITHUB_PATH"
