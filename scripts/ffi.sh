#!/bin/zsh
# Build the Rust engine and generate its Swift bindings into app/.
set -euo pipefail
cd "$(dirname "$0")/.."
export MACOSX_DEPLOYMENT_TARGET=26.0
cargo build --release -p afterimage-core --lib --bin uniffi-bindgen -q
rm -rf build/ffi && mkdir -p build/ffi app/Sources/CAfterimage/include app/Sources/AfterimageEngine
target/release/uniffi-bindgen generate --library target/release/libafterimage_core.dylib --language swift --no-format --out-dir build/ffi
cp build/ffi/afterimage_coreFFI.h app/Sources/CAfterimage/include/
cat > app/Sources/CAfterimage/include/module.modulemap <<'M'
module afterimage_coreFFI { header "afterimage_coreFFI.h" export * }
M
echo '// Anchors the C target.' > app/Sources/CAfterimage/anchor.c
cp build/ffi/afterimage_core.swift app/Sources/AfterimageEngine/
# Link the archive, never the dylib next to it.
mkdir -p build/lib && cp target/release/libafterimage_core.a build/lib/
