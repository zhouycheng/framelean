# FrameLean 项目上下文

## 产品方向

FrameLean 是面向 macOS 和 Windows 的本地桌面媒体压缩与格式处理工具。新主线继续使用 Flutter 前端、Rust Work 本机后端和 Rust FLL 媒体引擎动态库；不通过 FFmpeg CLI 执行媒体处理。

## 当前开发状态

`develop/next` 是长期开发集成线。当前代码已建立三项目目录和第一条最小跨进程/动态 ABI 链路：Flutter `EngineGateway` → loopback FEngine → `libloading` → FLL C ABI → FLL Runtime。Work 只提供认证、ping、媒体分析和 smoke 关闭；不能据此称为完整产品或完整媒体功能迁移。

2026-10-07 已在 macOS arm64 构建静态 libav SDK、FLL Release dylib 与 Work，并通过真实 Flutter gateway smoke：PCM WAV 被识别为 8000 Hz、单声道，新的 Gateway 实例接回同一个 Work PID。FLL 的 136 项测试、Work 的 4 项测试及 Flutter 分析通过。

2026-10-08 已完成 macOS arm64 Release App / ZIP 打包、包内 gateway smoke、解压后 ad hoc 签名完整性检查，以及 Release App 默认路径连接和 WAV 分析的手动验证。打包生成完整文件清单与 SHA-256；核对方案见 `docs/develop/package-checklist.md`。Windows、Intel/Universal 2、Developer ID 签名与公证仍未验收。

旧版数据库不迁移，初始应用数据为空。用户源媒体和输出媒体不会由代码清理处理。

## 当前结构

- `desktop-client/`：Flutter 桌面 UI、设置与未来的应用本地数据。
- `fengine/`：Rust Work 服务，是 Flutter 与 FLL 的唯一网关；动态加载 FLL，不静态依赖 FLL crates。
- `fll/`：Rust 媒体引擎项目，生成 macOS `.dylib` 和 Windows `.dll`；通过进程内 libav 适配器处理媒体。
- `protocol/`：Flutter ↔ Work JSON 协议与 Work ↔ FLL ABI 的契约说明，不是第四个项目。

当前 Smoke 操作和版本事实见 `docs/develop/architecture.md`、`protocol/` 与 `fll/README.md`。

## Git 与资料边界

- `develop/next` 是当前集成目标；功能通过短生命周期分支合入。
- 旧 `main` 和 `develop/v1.3.0-alpha` 冻结。`archive-main-2026-10-07`、`archive-alpha-v1.3.0-2026-10-07` 保存原提交。
- 旧项目经验与 Alpha 调用链、未完成边界见 `docs/develop/restart-baseline.md`。
- 工作区未提交清理和重建改动均是现有资产；不要重置、覆盖、暂存或提交。
