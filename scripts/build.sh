#!/bin/zsh
# Build build/Afterimage.app: Rust engine -> Swift bindings -> app bundle -> signed.
#   VERSION=0.2.0 BUILD=2 ./scripts/build.sh
#   AFTERIMAGE_SIGN_IDENTITY="Developer ID Application: Name (TEAMID)" ./scripts/build.sh
# CI (.github/workflows/release.yml) sets VERSION/BUILD and the identity from a throwaway keychain.
# AIDEV-NOTE: macOS ties Screen Recording and Accessibility to the signature. Local builds use the same Developer ID
# AIDEV-NOTE: as releases so an update never asks again; ad-hoc signing would ask on every build.
set -euo pipefail
cd "$(dirname "$0")/.."
source scripts/lib.sh
export DEVELOPER_DIR=${DEVELOPER_DIR:-/Applications/Xcode.app/Contents/Developer}
VERSION=${VERSION:-0.1.0}
BUILD=${BUILD:-1}
export MACOSX_DEPLOYMENT_TARGET=26.0

echo "▸ Engine"
step "Engine build" ./scripts/ffi.sh
echo "▸ App (v$VERSION build $BUILD)"
(cd app && step "Swift build" swift build -c release --arch arm64 --product Afterimage)
BIN="$(cd app && swift build -c release --arch arm64 --show-bin-path)/Afterimage"

APP=build/Afterimage.app
rm -rf "$APP"; mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN" "$APP/Contents/MacOS/"; cp Resources/Info.plist "$APP/Contents/"; cp NOTICE "$APP/Contents/Resources/"
/usr/libexec/PlistBuddy -c "Set :CFBundleShortVersionString $VERSION" -c "Set :CFBundleVersion $BUILD" "$APP/Contents/Info.plist"
if [[ ! -f build/AppIcon.icns || scripts/make-icon.swift -nt build/AppIcon.icns ]]; then
  rm -rf build/AppIcon.iconset; swift scripts/make-icon.swift build/AppIcon.iconset; iconutil -c icns build/AppIcon.iconset -o build/AppIcon.icns
fi
cp build/AppIcon.icns "$APP/Contents/Resources/"

ID=${AFTERIMAGE_SIGN_IDENTITY:-$(security find-identity -v -p codesigning 2>/dev/null | awk -F'"' '/Developer ID Application/ {print $2; exit}')}
if [[ -n "$ID" ]]; then
  step "Signing" codesign --force --options runtime --timestamp --sign "$ID" "$APP"
else
  step "Signing (ad-hoc)" codesign --force --options runtime --sign - "$APP"
fi
step "Verifying signature" codesign --verify --strict --verbose=2 "$APP"
echo "✓ $APP (v$VERSION, signed: ${ID:-ad-hoc})"
