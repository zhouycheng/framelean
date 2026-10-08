# FLL 项目上下文

## 项目边界

FLL 是独立 Rust 媒体引擎项目，负责媒体模型、分析、能力决策、Pipeline、Runtime、执行调度和进程内 libav。它不是 Flutter 应用，也不负责 Work/Client 网络协议、进程宿主、应用设置或 SQLite。

`fll/Cargo.toml` 是 FLL 的独立 Cargo workspace。十个 crate 由该 manifest 列出；macOS 构建 `libframelean_fll.dylib`，Windows 构建 `framelean_fll.dll`。Work 通过 `framelean_fll_get_api` 版本化 C ABI 和 `libloading` 动态加载，不能添加指向本 workspace crate 的 Cargo 依赖。

## 实现与产品接入状态

此目录的 Rust 实现从 Alpha 归档中选取性恢复，包含媒体分析、Snapshot、配置决策、预览 helper 和部分执行 Runtime。当前 `develop/next` 的 FEngine 只开放 `ping` 与 `analyze_media`；源码中存在其他引擎操作不代表 Flutter 产品已接入。

归档代码确认的执行范围是兼容输入的 packet stream-copy/remux、受限 SDR 视频转码和 PCM/AAC 到 AAC M4A。未资格化转换链、HDR 或其他格式组合失败关闭。旧协议、静态 FEngine 兼容路径和旧产品 UI/数据库没有迁回。

## Native media dependency

`framelean-ffmpeg` 使用 `rusty_ffmpeg` 和进程内 libav，不调用 FFmpeg/FFprobe CLI。构建必须显式提供平台对应的静态 SDK；Work 与 FLL 独立构建、并排打包。新打包入口见仓库根 `scripts/build/` 和 `docs/develop/workflow.md`。

## 真相来源

- Crate 列表与 Cargo 依赖：`Cargo.toml` 及各 crate manifest。
- ABI bootstrap 和 C 函数表：`crates/framelean-ffi/src/lib.rs`。
- Runtime JSON Schema：`schemas/`。
- 当前三项目边界和 smoke 接入：仓库根 `docs/develop/architecture.md`。
- 归档 Alpha 调用链和未完成项：仓库根 `docs/develop/restart-baseline.md`。
