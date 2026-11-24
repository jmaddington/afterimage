#!/bin/zsh
# Build and sign build/Afterimage.app.   SIGN_IDENTITY="..." ./scripts/build.sh
# AIDEV-NOTE: sign with a stable identity: ad-hoc signing makes macOS forget Screen Recording on every build.
set -euo pipefail
cd "$(dirname "$0")/.."
export DEVELOPER_DIR=${DEVELOPER_DIR:-/Applications/Xcode.app/Contents/Developer}
echo "▸ Engine"; ./scripts/ffi.sh
echo "▸ App"; (cd app && swift build -c release --product Afterimage -q)
BIN="$(cd app && swift build -c release --show-bin-path)/Afterimage"
APP=build/Afterimage.app
rm -rf "$APP"; mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN" "$APP/Contents/MacOS/"; cp Resources/Info.plist "$APP/Contents/"; cp NOTICE "$APP/Contents/Resources/"
if [[ ! -f build/AppIcon.icns || scripts/make-icon.swift -nt build/AppIcon.icns ]]; then
  rm -rf build/AppIcon.iconset; swift scripts/make-icon.swift build/AppIcon.iconset; iconutil -c icns build/AppIcon.iconset -o build/AppIcon.icns
fi
cp build/AppIcon.icns "$APP/Contents/Resources/"
ID=${SIGN_IDENTITY:-$(security find-identity -v -p codesigning | awk -F'"' '/Apple Development/ {print $2; exit}')}
codesign --force --options runtime --sign "${ID:--}" "$APP"
codesign --verify --strict "$APP"
echo "✓ $APP (signed: ${ID:-ad-hoc})"
