# 进程内 libav 构建与分发

## 新主线事实

FLL 通过 `rusty_ffmpeg` 绑定进程内 libav；产品运行包中不提供或调用 `ffmpeg` / `ffprobe` CLI。Rust runtime 需要随应用分发：macOS 的 `libframelean_fll.dylib` 或 Windows 的 `framelean_fll.dll`。源码级默认构建配方位于 `scripts/build/build_ffmpeg_*`，实际构建日志和版本事实由生成 SDK 中的 `ffmpeg-build-info.txt` 记录。

当前脚本默认 FFmpeg 9.0、静态 libav、`--enable-gpl` / `--enable-version3`、x264 与 libwebp；不启用 `--enable-nonfree`。实际公开分发前仍需按生成的构建元数据更新第三方许可证与源码提供材料。本文件不是已构建二进制或公开发行包的验收证明。

## 构建输出与边界

```text
build/dependencies/ffmpeg/<platform>/  # 静态开发链接 SDK
build/runtime/<platform>/               # Work 与 FLL 并排的运行时
<FrameLean.app>/Contents/Resources/     # macOS 包内 Work 与 dylib
<Windows Release directory>/             # Runner、Work、FLL 与法律材料
```

每个平台的打包验收都要核对包内 Work/FLL、目标架构、ABI 和实际动态库加载；源码存在或 Rust 编译成功不能代替包内 smoke。
