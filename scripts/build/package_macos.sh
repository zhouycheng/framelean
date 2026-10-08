#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
"$ROOT/scripts/build/build_runtime_macos.sh"

pushd "$ROOT/desktop-client" >/dev/null
flutter pub get
flutter analyze
flutter test --reporter expanded
if [[ "$(uname -m)" == arm64 ]]; then
  FLUTTER_MACOS_ARM64_ONLY=true flutter build macos --release
else
  flutter build macos --release
fi
popd >/dev/null

PRODUCTS="$ROOT/desktop-client/build/macos/Build/Products/Release"
APP=""
for candidate in "$PRODUCTS"/*.app; do
  if [[ -d "$candidate" ]]; then APP="$candidate"; break; fi
done
[[ -n "$APP" ]] || { echo "Flutter macOS app was not produced under $PRODUCTS" >&2; exit 1; }
RESOURCES="$APP/Contents/Resources"
RUNTIME="$ROOT/build/runtime/macos-$(uname -m | sed 's/x86_64/x64/')"
install -m 755 "$RUNTIME/framelean-engine" "$RESOURCES/framelean-engine"
install -m 755 "$RUNTIME/libframelean_fll.dylib" "$RESOURCES/libframelean_fll.dylib"

APP_EXECUTABLE="$(/usr/libexec/PlistBuddy -c 'Print CFBundleExecutable' "$APP/Contents/Info.plist")"
APP_BINARY="$APP/Contents/MacOS/$APP_EXECUTABLE"
APP_ARCH="$(lipo -archs "$APP_BINARY")"
[[ "$APP_ARCH" == "$(uname -m)" ]] || { echo "unexpected app architecture: $APP_ARCH" >&2; exit 1; }
for runtime in "$RESOURCES/framelean-engine" "$RESOURCES/libframelean_fll.dylib"; do
  [[ -f "$runtime" ]] || { echo "missing packaged runtime: $runtime" >&2; exit 1; }
  actual="$(lipo -archs "$runtime")"
  [[ "$actual" == "$(uname -m)" ]] || { echo "unexpected runtime architecture: $runtime ($actual)" >&2; exit 1; }
done
[[ -f "$RESOURCES/legal/LICENSE" && -f "$RESOURCES/legal/NOTICE.md" ]] || {
  echo "legal materials are missing from the app bundle" >&2
  exit 1
}

# Runtime files are added after Xcode seals the app; reseal this local build.
codesign --force --sign - "$RESOURCES/framelean-engine" "$RESOURCES/libframelean_fll.dylib"
codesign --force --sign - --preserve-metadata=identifier,entitlements,flags "$APP"
codesign --verify --deep --strict "$APP"

(
  unset FRAMELEAN_FLL_LIBRARY
  export FRAMELEAN_FENGINE_BINARY="$RESOURCES/framelean-engine"
  cd "$ROOT/desktop-client"
  flutter test test/engine_gateway_smoke_test.dart --reporter expanded
)

PACKAGE_DIR="$ROOT/build/packages"
mkdir -p "$PACKAGE_DIR"
PACKAGE="$PACKAGE_DIR/FrameLean-macos-$(uname -m).zip"
ditto -c -k --sequesterRsrc --keepParent "$APP" "$PACKAGE"
unzip -tq "$PACKAGE"
unzip -Z1 "$PACKAGE" > "$PACKAGE.files.txt"
shasum -a 256 "$PACKAGE" > "$PACKAGE.sha256"
printf 'Packaged and smoke-tested %s\n' "$PACKAGE"
