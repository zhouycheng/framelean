#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/../.."
export FFMPEG_PKG_CONFIG_PATH="${FFMPEG_PKG_CONFIG_PATH:-$(pkg-config --variable=libdir libavcodec)/pkgconfig}"
export FFMPEG_LINK_MODE=dynamic
cargo build --release --locked --manifest-path fll/Cargo.toml
cargo build --release --locked --manifest-path fengine/Cargo.toml
(cd desktop-client && flutter build macos --release)
app=desktop-client/build/macos/Build/Products/Release/framelean.app
runtime="$app/Contents/Resources/runtime"
python3 -c 'import pathlib,shutil,sys; p=pathlib.Path(sys.argv[1]); shutil.rmtree(p) if p.exists() else None; p.mkdir()' "$runtime"
cp fengine/target/release/framelean-engine "$runtime/"
cp fll/target/release/libframelean_fll.dylib "$runtime/"
python3 scripts/build/bundle_macos.py "$runtime"
codesign --force --deep --sign - "$app"
"$runtime/framelean-engine" probe "$PWD/$runtime/libframelean_fll.dylib"
printf 'Built: %s\n' "$app"
