#!/usr/bin/env bash
set -euo pipefail
export LC_ALL=C

ARCH="${1:-}"
case "$ARCH" in
  arm64)
    ARCH_LABEL="arm64"
    ;;
  x86_64)
    ARCH_LABEL="x64"
    ;;
  *)
    echo "usage: $0 <arm64|x86_64>" >&2
    exit 64
    ;;
esac

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
BUILD_DIR="${ROOT}/build/ffmpeg-macos-${ARCH_LABEL}"
SRC_DIR="${BUILD_DIR}/src"
PREFIX="${BUILD_DIR}/dist"
OUT_DIR="${ROOT}/build/dependencies/ffmpeg/macos-${ARCH_LABEL}"
FFMPEG_VERSION="${FFMPEG_VERSION:-9.0}"
LIBWEBP_VERSION="${LIBWEBP_VERSION:-1.5.0}"
X264_COMMIT="${X264_COMMIT:-0480cb05fa188d37ae87e8f4fd8f1aea3711f7ee}"
MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-12.0}"
JOBS="${JOBS:-$(sysctl -n hw.ncpu)}"
ARCH_FLAGS="-arch ${ARCH} -mmacosx-version-min=${MACOSX_DEPLOYMENT_TARGET}"

require_command() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "error: missing required command: $1" >&2
    exit 1
  fi
}

download_archive() {
  local url="$1"
  local archive="$2"
  local partial_archive="${archive}.partial"

  if [[ -f "$archive" ]] && tar -tf "$archive" >/dev/null 2>&1; then
    return
  fi

  rm -f "$archive" "$partial_archive"
  curl --http1.1 --fail --location --retry 5 --retry-all-errors --retry-delay 2 \
    --output "$partial_archive" "$url"
  if ! tar -tf "$partial_archive" >/dev/null 2>&1; then
    echo "error: downloaded archive is invalid: $url" >&2
    rm -f "$partial_archive"
    exit 1
  fi
  mv "$partial_archive" "$archive"
}

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "error: this script only builds the macOS FFmpeg runtime" >&2
  exit 1
fi

if [[ "$(uname -m)" != "$ARCH" ]]; then
  echo "error: $ARCH runtime must be built on a native $ARCH macOS host" >&2
  exit 1
fi

for command_name in clang clang++ cmake cp curl find git install lipo make nasm pkg-config sed tar xcrun; do
  require_command "$command_name"
done

SDK_PATH="$(xcrun --show-sdk-path)"
LIBCXX_SDK_FLAGS="-isysroot ${SDK_PATH} -isystem ${SDK_PATH}/usr/include/c++/v1"

export MACOSX_DEPLOYMENT_TARGET
rm -rf "$PREFIX"
mkdir -p "$SRC_DIR" "$PREFIX" "$OUT_DIR"

cd "$SRC_DIR"
download_archive \
  "https://storage.googleapis.com/downloads.webmproject.org/releases/webp/libwebp-${LIBWEBP_VERSION}.tar.gz" \
  "libwebp-${LIBWEBP_VERSION}.tar.gz"

rm -rf "libwebp-${LIBWEBP_VERSION}"
tar -xf "libwebp-${LIBWEBP_VERSION}.tar.gz"

cd "$SRC_DIR/libwebp-${LIBWEBP_VERSION}"
make distclean >/dev/null 2>&1 || true
CFLAGS="$ARCH_FLAGS" CXXFLAGS="$ARCH_FLAGS" LDFLAGS="$ARCH_FLAGS" \
  ./configure \
    --prefix="$PREFIX" \
    --disable-shared \
    --enable-static \
    --disable-gl \
    --disable-sdl \
    --disable-png \
    --disable-jpeg \
    --disable-tiff \
    --disable-gif \
    --disable-wic
make -j"$JOBS"
make install

cd "$SRC_DIR"
if [[ ! -d x264 ]]; then
  git clone https://code.videolan.org/videolan/x264.git
fi

cd "$SRC_DIR/x264"
git fetch --quiet origin "$X264_COMMIT" || echo "warning: x264 fetch failed; using the local checkout"
if ! git checkout --quiet --detach "$X264_COMMIT"; then
  echo "error: x264 commit is unavailable: $X264_COMMIT" >&2
  exit 1
fi
make distclean >/dev/null 2>&1 || true
CFLAGS="$ARCH_FLAGS" LDFLAGS="$ARCH_FLAGS" \
  ./configure \
    --prefix="$PREFIX" \
    --enable-static \
    --disable-cli \
    --disable-opencl
make -j"$JOBS"
make install

cd "$SRC_DIR"
download_archive \
  "https://ffmpeg.org/releases/ffmpeg-${FFMPEG_VERSION}.tar.xz" \
  "ffmpeg-${FFMPEG_VERSION}.tar.xz"

rm -rf "ffmpeg-${FFMPEG_VERSION}"
tar -xf "ffmpeg-${FFMPEG_VERSION}.tar.xz"

cd "$SRC_DIR/ffmpeg-${FFMPEG_VERSION}"
make distclean >/dev/null 2>&1 || true

PKG_CONFIG_PATH="${PREFIX}/lib/pkgconfig" \
PKG_CONFIG_LIBDIR="${PREFIX}/lib/pkgconfig" \
./configure \
  --prefix="$PREFIX" \
  --arch="$ARCH" \
  --target-os=darwin \
  --cc=clang \
  --enable-gpl \
  --enable-version3 \
  --enable-libx264 \
  --enable-libwebp \
  --enable-videotoolbox \
  --enable-audiotoolbox \
  --disable-shared \
  --enable-static \
  --disable-programs \
  --disable-sdl2 \
  --disable-debug \
  --disable-doc \
  --disable-ffplay \
  --pkg-config-flags="--static" \
  --extra-cflags="${ARCH_FLAGS} -I${PREFIX}/include" \
  --extra-ldflags="${ARCH_FLAGS} -L${PREFIX}/lib"

make -j"$JOBS"
make install

rm -rf "$OUT_DIR/include" "$OUT_DIR/lib"
mkdir -p "$OUT_DIR/include" "$OUT_DIR/lib/pkgconfig"
cp -R "$PREFIX/include/." "$OUT_DIR/include/"
find "$PREFIX/lib" -maxdepth 1 -type f -name '*.a' -exec cp {} "$OUT_DIR/lib/" \;
find "$PREFIX/lib/pkgconfig" -maxdepth 1 -type f -name '*.pc' -exec cp {} "$OUT_DIR/lib/pkgconfig/" \;

for library_name in avcodec avdevice avfilter avformat avutil swresample swscale; do
  library_path="$OUT_DIR/lib/lib${library_name}.a"
  if [[ ! -f "$library_path" ]]; then
    echo "error: missing required static library: $library_path" >&2
    exit 1
  fi
  built_arches="$(lipo -archs "$library_path")"
  if [[ "$built_arches" != "$ARCH" ]]; then
    echo "error: expected $library_name architecture $ARCH, got $built_arches" >&2
    exit 1
  fi
done

pkg_config_root='${pcfiledir}/../..'
while IFS= read -r pkg_config_file; do
  sed -i '' \
    -e "s|${PREFIX}|${pkg_config_root}|g" \
    -e "s|${OUT_DIR}|${pkg_config_root}|g" \
    "$pkg_config_file"
done < <(find "$OUT_DIR/lib/pkgconfig" -maxdepth 1 -type f -name '*.pc' -print)

cat > "$OUT_DIR/ffmpeg-build-info.txt" <<EOF
FFmpeg version: ${FFMPEG_VERSION}
x264 source: https://code.videolan.org/videolan/x264.git
x264 commit: ${X264_COMMIT}
libwebp version: ${LIBWEBP_VERSION}
libwebp source: https://storage.googleapis.com/downloads.webmproject.org/releases/webp/libwebp-${LIBWEBP_VERSION}.tar.gz
FFmpeg source: https://ffmpeg.org/releases/ffmpeg-${FFMPEG_VERSION}.tar.xz
Target: macOS ${ARCH}
Minimum macOS: ${MACOSX_DEPLOYMENT_TARGET}
Nonfree enabled: no
Distribution: static libav SDK; no ffmpeg or ffprobe executables
EOF

echo
echo "Built macOS $ARCH static libav SDK:"
echo "$OUT_DIR"
