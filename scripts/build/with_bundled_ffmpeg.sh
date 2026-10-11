#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SDK_DIR="${FRAMELEAN_FFMPEG_SDK_DIR:-}"

if [[ -z "$SDK_DIR" ]]; then
  case "$(uname -s)" in
    Darwin)
      case "$(uname -m)" in
        arm64) SDK_DIR="$ROOT/build/dependencies/ffmpeg/macos-arm64" ;;
        x86_64) SDK_DIR="$ROOT/build/dependencies/ffmpeg/macos-x64" ;;
        *)
          echo "error: unsupported macOS architecture: $(uname -m)" >&2
          exit 1
          ;;
      esac
      ;;
    MINGW*_NT* | MSYS*_NT*) SDK_DIR="$ROOT/build/dependencies/ffmpeg/windows-x64" ;;
    *)
      echo "error: set FRAMELEAN_FFMPEG_SDK_DIR to a bundled static libav SDK for $(uname -s)" >&2
      exit 1
      ;;
  esac
fi

if [[ ! -d "$SDK_DIR/include" || ! -d "$SDK_DIR/lib" ]]; then
  echo "error: bundled FFmpeg SDK is missing include/ or lib/: $SDK_DIR" >&2
  echo "build it with the platform build script, or set FRAMELEAN_FFMPEG_SDK_DIR." >&2
  exit 1
fi

export FFMPEG_LINK_MODE=static

case "$(uname -s)" in
  Darwin | Linux)
    export FFMPEG_INCLUDE_DIR="$SDK_DIR/include"
    if [[ "$(uname -s)" == "Darwin" ]]; then
      export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-12.0}"
    fi
    if [[ ! -d "$SDK_DIR/lib/pkgconfig" ]]; then
      echo "error: bundled FFmpeg SDK is missing lib/pkgconfig: $SDK_DIR" >&2
      exit 1
    fi
    export FFMPEG_PKG_CONFIG_PATH="$SDK_DIR/lib/pkgconfig"
    ;;
  MINGW*_NT* | MSYS*_NT*)
    if [[ ! -d "$SDK_DIR/lib/pkgconfig" ]]; then
      echo "error: bundled FFmpeg SDK is missing lib/pkgconfig: $SDK_DIR" >&2
      exit 1
    fi
    if ! command -v pkg-config >/dev/null 2>&1; then
      echo "error: pkg-config is required to link the bundled Windows static SDK" >&2
      exit 1
    fi
    if ! command -v cygpath >/dev/null 2>&1; then
      echo "error: cygpath is required to pass SDK paths to the Windows Rust toolchain" >&2
      exit 1
    fi

    sdk_dir_windows="$(cygpath -w "$SDK_DIR")"
    export FFMPEG_INCLUDE_DIR="${sdk_dir_windows}\\include"
    export FFMPEG_LIBS_DIR="${sdk_dir_windows}\\lib"
    export FFMPEG_PKG_CONFIG_PATH="$SDK_DIR/lib/pkgconfig"

    pkg_config_flags="$(
      PKG_CONFIG_PATH="$SDK_DIR/lib/pkgconfig" \
      PKG_CONFIG_LIBDIR="$SDK_DIR/lib/pkgconfig" \
        pkg-config --static --libs \
          libavdevice libavfilter libavformat libavcodec \
          libswresample libswscale libavutil
    )"
    rustflags="${RUSTFLAGS:-}"
    # Rust disables default libraries; rescan static media dependencies with their C/GNU runtimes.
    rustflags+=" -C link-arg=-Wl,--start-group,-Bstatic"
    for linker_flag in $pkg_config_flags; do
      case "$linker_flag" in
        -D*) continue ;;
      esac
      rustflags+=" -C link-arg=${linker_flag}"
    done
    rustflags+=" -C link-arg=-lmingw32 -C link-arg=-lmingwex -C link-arg=-lmsvcrt -C link-arg=-lkernel32 -C link-arg=-lgcc -C link-arg=-lgcc_eh -C link-arg=-lwinpthread"
    rustflags+=" -C link-arg=-Wl,--end-group,-Bdynamic"
    export RUSTFLAGS="$rustflags"
    ;;
esac

exec "$@"
