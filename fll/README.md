# FLL（FrameLean Lib）

FLL 是 FrameLean 的 Rust 媒体引擎项目，独立构建为供 `fengine/` 动态加载的 `framelean_fll` 库。它负责媒体模型与分析、能力决策、Pipeline、Runtime、执行状态和调度；FFmpeg 仅通过进程内 libav API 使用，不启动 FFmpeg/FFprobe CLI。

FLL 不拥有桌面 UI、应用设置、应用数据库、Client/Work 协议或操作系统级 Work 进程宿主。新主线通过 `protocol/fll-abi/v1/` 描述的版本化 C ABI 接入；`fengine/` 不静态依赖本 Workspace crate。

## Workspace

`Cargo.toml` 实际列出 10 个 crate：

| Crate | 职责 |
| --- | --- |
| `framelean-core` | ID、错误和基础类型 |
| `framelean-media` | 媒体模型与 Processor API |
| `framelean-analysis` | 媒体分析领域与 Analyzer 抽象 |
| `framelean-environment` | 环境快照与资源监控 |
| `framelean-decision` | Capability、Recommendation、Preset 与 Estimator |
| `framelean-ffmpeg` | 进程内 libav Adapter 与 Backend |
| `framelean-pipeline` | Pipeline 计划与顺序执行 |
| `framelean-plugin` | Plugin、Factory 与 Registry |
| `framelean-runtime` | Task、Snapshot、Scheduler 和 composition root |
| `framelean-ffi` | C ABI `cdylib`，公开唯一 bootstrap 符号 |

## 当前接入状态

FLL 来源于 Alpha 归档代码。引擎源码包含媒体分析与 Snapshot、预览辅助、受限执行 Runtime 等实现，但 `develop/next` 的 Work 当前只开放 `ping` 和 `analyze_media`。Alpha 的实现库存不等于新产品能力已迁入或已验收。

旧代码明确写出的执行链包括兼容输入的 packet stream-copy/remux、受限 SDR 视频转码，以及 PCM/AAC 到 AAC M4A。未资格化 codec、HDR、字幕/数据/附件等组合必须失败关闭。新主线后续逐项验收前不承诺这些执行能力。

## 构建

FLL 需要按目标平台构建的静态 libav SDK。SDK 位于被忽略的 `build/dependencies/ffmpeg/<platform>/`，不是用户运行时的 FFmpeg 可执行文件。

```sh
scripts/build/build_runtime_macos.sh
```

Windows x64 从 MSYS2 MINGW64 shell 运行：

```sh
bash scripts/build/build_runtime_windows.sh
```

这两个入口构建 `framelean_fll` 与独立的 Work 可执行文件，并验证目标架构及无动态 libav 依赖。FLL 的直接 Cargo manifest 和 Runtime Schema 位于本目录。
