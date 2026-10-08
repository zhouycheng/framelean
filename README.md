# FrameLean

FrameLean 是面向 macOS 和 Windows 的本地桌面媒体压缩与格式处理工具。当前开发主线由 Flutter 前端、Rust Work 服务和 Rust FLL 动态媒体引擎组成；首个可验证闭环是连接 Work、检查 FLL 和分析音频 fixture。

## 项目

- [Flutter 前端](desktop-client/)
- [Rust Work 服务](fengine/)
- [Rust 媒体引擎](fll/)
- [跨项目协议](protocol/)
- [架构现状](docs/develop/architecture.md)
- [重启分析基线](docs/develop/restart-baseline.md)
- [构建与清理脚本](scripts/README.md)

## 开发

macOS：先按 [工程流程](docs/develop/workflow.md) 安装静态 libav 构建依赖，再运行：

```sh
scripts/build/build_runtime_macos.sh
cd desktop-client
flutter pub get
flutter analyze
FRAMELEAN_FENGINE_BINARY="../build/runtime/macos-$(uname -m | sed 's/x86_64/x64/')/framelean-engine" \
flutter test
```

Windows 的 FLL/Work 原生构建从 MSYS2 MINGW64 执行 `scripts/build/build_runtime_windows.sh`，Flutter release 打包由 `scripts/build/package_windows.ps1` 完成。

macOS app 打包入口：`scripts/build/package_macos.sh`。构建好的 app 必须同时包含 Work 与 FLL 动态库。

## 许可

FrameLean 按 [GPL-3.0-or-later](LICENSE) 分发。第三方许可与源码获取信息见 [legal/](legal/)。
