# FrameLean 构建与清理脚本

FrameLean 有三个独立可构建项目。以下脚本服务于新主线；FFmpeg 脚本构建静态 libav SDK，不产出或调用 FFmpeg/FFprobe 命令行程序。

## 静态 libav SDK 与 Rust runtime

| 脚本 | 平台 | 作用 |
| --- | --- | --- |
| `build/build_ffmpeg_macos_arch.sh <arm64|x86_64>` | 原生 macOS | 构建当前 FLL 所需的静态 libav SDK |
| `build/build_ffmpeg_windows_x64.sh` | MSYS2 MINGW64 | 构建 Windows x64 静态 libav SDK |
| `build/with_bundled_ffmpeg.sh` | macOS / MSYS2 | 向 Cargo 显式提供 SDK，禁止静默回退到系统 FFmpeg |
| `build/build_runtime_macos.sh` | macOS | 构建并校验 FLL dylib 与 Work 可执行文件 |
| `build/build_runtime_windows.sh` | MSYS2 MINGW64 | 构建并校验 FLL DLL 与 Work 可执行文件 |

静态 SDK 放在被忽略的 `build/dependencies/ffmpeg/`。Work 与 FLL 的构建输出并排位于 `build/runtime/<platform>/`，不提交到仓库。

## 桌面打包

- `build/package_macos.sh`：构建 runtime、分析/测试 Flutter 项目、生成当前主机架构的 macOS app，复制 Work/FLL 到 `Contents/Resources/` 后重新 ad hoc 签名并校验，从打包位置执行 gateway smoke，生成 ZIP、完整条目清单 `.zip.files.txt` 和 SHA-256 `.zip.sha256`。
- `build/package_windows.ps1`：验证已有 MSYS2 runtime、分析/测试 Flutter 项目、Release 构建 Windows app，要求 CMake 将 Work/FLL 放到 Runner 旁，从包内运行 smoke 并生成 ZIP。

Windows Rust runtime 必须在 MSYS2 MINGW64 shell 执行 `build_runtime_windows.sh` 后，再运行 PowerShell 打包入口。

Windows CI 的 MSYS2 使用 `path-type: inherit`，保留前一步安装的 Rust 工具路径。本地运行时也需确保 `rustup`、`cargo`、`gcc`、`pkg-config`、`cygpath` 和 `objdump` 位于 shell 的 PATH；runtime 入口在构建静态 SDK 前检查这些工具并准备 Rust target。

Windows CI 在 SDK 构建完成后独立保存缓存；以最后生成的 `ffmpeg-build-info.txt` 为完成标记，后续 Rust 编译或测试失败也能保留完整 SDK。

## 清理本地构建物

macOS 或 MSYS2 Bash：

```sh
scripts/clean.sh
```

Windows PowerShell：

```powershell
.\scripts\clean.ps1
```

清理脚本删除仓库内的最终包、静态 libav SDK 与源码缓存、Flutter/Xcode 输出、Dart 缓存、Rust `target/` 和 `.workspace/` 下的日志。下次打包时会重新生成；静态 libav SDK 缺失时，构建脚本会重新下载并编译依赖。计划和 release 草稿、源码、锁文件、协议 Schema、清理目标外的媒体文件及仓库外的应用数据会保留。脚本只删除列出的生成路径，不调用 `git clean`。
