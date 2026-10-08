# 进程内 libav adapter

`framelean-ffmpeg` 通过 `rusty_ffmpeg` 调用 libav API；FrameLean 媒体处理不通过 FFmpeg 或 FFprobe 命令行程序启动子进程。FLL 构建为 dylib/DLL，libav 静态链接到 FLL，由 FEngine 动态加载。

目标平台 SDK 由仓库根 `scripts/build/build_ffmpeg_macos_arch.sh` 或 `build_ffmpeg_windows_x64.sh` 构建，输出到被 Git 忽略的 `build/dependencies/ffmpeg/<platform>/`。`with_bundled_ffmpeg.sh` 显式设置 include、pkg-config 和静态链接参数；缺少 SDK 时失败，不回退到系统/Homebrew FFmpeg。

Alpha 归档代码内的真实执行实现有限：兼容 packet stream-copy/remux、受限 SDR 视频转码和 PCM/AAC 音轨到 AAC M4A。其余路径应失败关闭。当前 Work 只接入健康检查和媒体分析，执行能力尚未接入新主线。

实际构建版本以对应 SDK 目录生成的 `ffmpeg-build-info.txt` 为准。源码中的默认版本记录不证明依赖已成功构建、包内加载或已发布。
