#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
ARCH="$(uname -m)"
case "$ARCH" in
  arm64) LABEL=arm64; TARGET=aarch64-apple-darwin ;;
  x86_64) LABEL=x64; TARGET=x86_64-apple-darwin ;;
  *) echo "unsupported macOS architecture: $ARCH" >&2; exit 1 ;;
esac
SDK="$ROOT/build/dependencies/ffmpeg/macos-$LABEL"
OUT="$ROOT/build/runtime/macos-$LABEL"

if [[ ! -d "$SDK/include" || ! -d "$SDK/lib/pkgconfig" ]]; then
  "$ROOT/scripts/build/build_ffmpeg_macos_arch.sh" "$ARCH"
fi

rustup target add "$TARGET"
FRAMELEAN_FFMPEG_SDK_DIR="$SDK" \
  "$ROOT/scripts/build/with_bundled_ffmpeg.sh" \
  cargo build --manifest-path "$ROOT/fll/Cargo.toml" --release --locked --target "$TARGET"
FRAMELEAN_FFMPEG_SDK_DIR="$SDK" \
  "$ROOT/scripts/build/with_bundled_ffmpeg.sh" \
  cargo test --manifest-path "$ROOT/fll/Cargo.toml" --workspace --locked --target "$TARGET"
cargo test --manifest-path "$ROOT/fengine/Cargo.toml" --locked
cargo build --manifest-path "$ROOT/fengine/Cargo.toml" --release --locked --target "$TARGET"

mkdir -p "$OUT"
cp "$ROOT/fengine/target/$TARGET/release/framelean-engine" "$OUT/framelean-engine"
cp "$ROOT/fll/target/$TARGET/release/libframelean_fll.dylib" "$OUT/libframelean_fll.dylib"
for binary in "$OUT/framelean-engine" "$OUT/libframelean_fll.dylib"; do
  actual="$(lipo -archs "$binary")"
  [[ "$actual" == "$ARCH" ]] || { echo "expected $ARCH, got $actual for $binary" >&2; exit 1; }
done
if otool -L "$OUT/framelean-engine" "$OUT/libframelean_fll.dylib" | grep -Eiq 'libav(codec|device|filter|format|util)|libsw(resample|scale)'; then
  echo "engine package unexpectedly depends on dynamic libav" >&2
  exit 1
fi
"$OUT/framelean-engine" --version
printf 'Built %s Work and FLL runtime in %s\n' "$ARCH" "$OUT"
