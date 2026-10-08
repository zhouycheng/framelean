#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TARGET=x86_64-pc-windows-gnu
SDK="$ROOT/build/dependencies/ffmpeg/windows-x64"
OUT="$ROOT/build/runtime/windows-x64"
case "$(uname -s)" in
  MINGW64_NT* | MSYS_NT* | MINGW32_NT*) ;;
  *) echo "run this script from the MSYS2 MINGW64 shell" >&2; exit 1 ;;
esac
if [[ ! -d "$SDK/include" || ! -d "$SDK/lib/pkgconfig" ]]; then
  "$ROOT/scripts/build/build_ffmpeg_windows_x64.sh"
fi

rustup target add "$TARGET"
FRAMELEAN_FFMPEG_SDK_DIR="$SDK" \
  "$ROOT/scripts/build/with_bundled_ffmpeg.sh" \
  cargo build --manifest-path "$ROOT/fll/Cargo.toml" --release --locked --target "$TARGET"
FRAMELEAN_FFMPEG_SDK_DIR="$SDK" \
  "$ROOT/scripts/build/with_bundled_ffmpeg.sh" \
  cargo test --manifest-path "$ROOT/fll/Cargo.toml" --workspace --locked --target "$TARGET"
cargo test --manifest-path "$ROOT/fengine/Cargo.toml" --locked
RUSTFLAGS="${RUSTFLAGS:-} -C target-feature=+crt-static -C link-arg=-static-libgcc -C link-arg=-static-libstdc++" \
  cargo build --manifest-path "$ROOT/fengine/Cargo.toml" --release --locked --target "$TARGET"

mkdir -p "$OUT"
cp "$ROOT/fengine/target/$TARGET/release/framelean-engine.exe" "$OUT/framelean-engine.exe"
cp "$ROOT/fll/target/$TARGET/release/framelean_fll.dll" "$OUT/framelean_fll.dll"
for binary in "$OUT/framelean-engine.exe" "$OUT/framelean_fll.dll"; do
  objdump -f "$binary" | grep -q "architecture: i386:x86-64" || { echo "expected Windows x64 binary: $binary" >&2; exit 1; }
  if objdump -p "$binary" | grep -Eiq 'DLL Name:.*(avcodec|avdevice|avfilter|avformat|avutil|swresample|swscale|libstdc\+\+|libgcc|libwinpthread)'; then
    echo "unexpected dynamic media or GNU runtime dependency in $binary" >&2
    objdump -p "$binary" | grep -Ei 'DLL Name:' >&2
    exit 1
  fi
done
"$OUT/framelean-engine.exe" --version
printf 'Built Windows x64 Work and FLL runtime in %s\n' "$OUT"
