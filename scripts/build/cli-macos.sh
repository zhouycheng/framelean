#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/../.."
export FFMPEG_PKG_CONFIG_PATH="${FFMPEG_PKG_CONFIG_PATH:-$(pkg-config --variable=libdir libavcodec)/pkgconfig}"
export FFMPEG_LINK_MODE=dynamic
cargo build --release --locked --manifest-path fll/Cargo.toml
cargo build --release --locked --manifest-path fengine/Cargo.toml
runtime=outputs/cli-macos
python3 -c 'import pathlib,shutil,sys; p=pathlib.Path(sys.argv[1]); shutil.rmtree(p) if p.exists() else None; p.mkdir(parents=True)' "$runtime"
cp fengine/target/release/framelean fengine/target/release/framelean-engine "$runtime/"
cp fll/target/release/libframelean_fll.dylib "$runtime/"
python3 scripts/build/bundle_macos.py "$runtime"
codesign --force --sign - "$runtime/framelean" "$runtime/framelean-engine"
"$runtime/framelean-engine" probe "$PWD/$runtime/libframelean_fll.dylib"
printf 'CLI bundle: %s\n' "$runtime"
