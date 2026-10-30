#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/../.."
bash scripts/build/cli-macos.sh
(cd desktop-client && flutter build macos --release)
app=desktop-client/build/macos/Build/Products/Release/framelean.app
runtime="$app/Contents/Resources/runtime"
python3 -c 'import pathlib,shutil,sys; p=pathlib.Path(sys.argv[1]); shutil.rmtree(p) if p.exists() else None; p.mkdir()' "$runtime"
cp outputs/cli-macos/* "$runtime/"
codesign --force --deep --sign - "$app"
"$runtime/framelean-engine" probe "$PWD/$runtime/libframelean_fll.dylib"
printf 'Built: %s\n' "$app"
