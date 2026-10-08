# Source Code and Build Information

FrameLean is distributed under the GNU General Public License, version 3 or any later version published by the Free Software Foundation.

## Corresponding Source

For a binary release, corresponding source should be made available at the same location or through a clearly linked public repository for the applicable GPL period. It includes the source for the Flutter application, Rust Work service, FLL engine, build scripts, protocol/ABI definitions, and the exact source and configuration for any statically linked third-party media libraries.

`legal/third-party/ffmpeg/ffmpeg-build-info.txt` records only the default source-level recipe. Each actual build must preserve and publish its generated `build/dependencies/ffmpeg/<platform>/ffmpeg-build-info.txt` together with the corresponding source locations and license notices.

## Current Default Media Library Recipe

The build scripts currently pin FFmpeg 9.0, x264 commit `0480cb05fa188d37ae87e8f4fd8f1aea3711f7ee`, and libwebp 1.5.0. They enable `--enable-gpl`, `--enable-version3`, `--enable-libx264`, and `--enable-libwebp`, use static libraries, and do not enable `--enable-nonfree`. FLL links libav in process; the build scripts do not produce FFmpeg or FFprobe CLI programs.

- FFmpeg: https://ffmpeg.org/releases/ffmpeg-9.0.tar.xz
- x264: https://code.videolan.org/videolan/x264
- libwebp: https://storage.googleapis.com/downloads.webmproject.org/releases/webp/libwebp-1.5.0.tar.gz

macOS additionally configures VideoToolbox and AudioToolbox. Windows additionally configures D3D11VA and DXVA2. Always use the generated build record as the source for an actual release; this file alone does not certify a package.

## Rebuild and Replacement

Build scripts produce `framelean_fll` as a macOS dylib or Windows DLL and place it beside the Work executable in packaged output. Replacing that library requires a compatible FLL ABI; the source and ABI contract are under `fll/` and `protocol/fll-abi/v1/`. The current product has no UI for custom runtime paths.

A release bundle includes the root LICENSE and `legal/` directory. Update these materials from the actual Cargo/Pub lockfiles and generated native build metadata before public distribution.
