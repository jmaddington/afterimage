#!/bin/zsh
# Sign an app bundle, inside out.   scripts/sign.sh build/Afterimage.app
# Identity: AFTERIMAGE_SIGN_IDENTITY, else the first Developer ID in the keychain, else ad-hoc.
set -euo pipefail
source "${0:A:h}/lib.sh"
APP=${1:?usage: sign.sh <app>}
ID=${AFTERIMAGE_SIGN_IDENTITY:-$(security find-identity -v -p codesigning 2>/dev/null | awk -F'"' '/Developer ID Application/ {print $2; exit}')}
ts=(); [[ -n "$ID" ]] && ts=(--timestamp)
step "Signing bridge" codesign --force --options runtime $ts --sign "${ID:--}" "$APP/Contents/MacOS/afterimage-bridge"
step "Signing app" codesign --force --options runtime $ts --sign "${ID:--}" "$APP"
step "Verifying signature" codesign --verify --strict --verbose=2 "$APP"
echo "✓ $APP (signed: ${ID:-ad-hoc})"
