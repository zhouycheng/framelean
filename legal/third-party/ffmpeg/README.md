# FFmpeg / libav

FLL 使用 FFmpeg 的进程内 libav API。当前构建配方位于 `scripts/build/build_ffmpeg_macos_arch.sh` 和 `scripts/build/build_ffmpeg_windows_x64.sh`，不构建 FFmpeg/FFprobe CLI。

默认源码版本和配置以脚本及各次生成 SDK 中的 `ffmpeg-build-info.txt` 为准。公开分发前，应从实际 SDK 的元数据确认 FFmpeg、x264、libwebp 的版本、许可证及源码提供方式；不得启用 `--enable-nonfree`。
