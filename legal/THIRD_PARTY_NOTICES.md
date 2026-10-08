# Third-Party Notices

FrameLean source is licensed under GPL-3.0-or-later. See the repository root `LICENSE` and `legal/COPYING`.

## Media engine

FLL links static FFmpeg/libav libraries into `framelean_fll`. The source-level default recipe is FFmpeg 9.0 with GPL/version 3 and x264 plus libwebp; `--enable-nonfree` is disabled. No FFmpeg or FFprobe CLI executable is shipped by the current build scripts. Per-build versions and flags are recorded under the generated `build/dependencies/ffmpeg/<platform>/ffmpeg-build-info.txt`; public package notices must be verified against that record.

- FFmpeg: https://ffmpeg.org/
- x264: https://code.videolan.org/videolan/x264
- libwebp: https://chromium.googlesource.com/webm/libwebp/

## Flutter

The Flutter SDK supplies runtime and package license notices. The direct package list is defined by `desktop-client/pubspec.yaml` and must be reviewed from its lockfile for each public binary release.
