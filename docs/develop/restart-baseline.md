# 重启基线：旧产品与 Alpha 架构证据

## 证据范围

对照的冻结提交是 `archive-main-2026-10-07`（`de8079d`）和 `archive-alpha-v1.3.0-2026-10-07`（`330ffae`），共同祖先为 `977f169`。这两个标签指向原提交，没有被本次实现改写。本记录来自 Git 中的源码、文档和构建脚本静态检查；本轮没有重新运行旧版本安装包，因此不把旧发布说明当成新主线的运行时验收。

以下使用三种证据口径：

- **源码事实**：可在归档提交的实现、manifest 或脚本中直接找到。
- **文档记录**：旧项目文档对发布范围或设计意图的描述，仍需结合源码看。
- **待重验**：本次没有构建旧安装包或在目标机器实测，不能据此宣称交付故障已复现。

## `main`：已交付产品经验

`de8079d` 对应 v1.2.1 发布线，Flutter 版本字段为 `1.2.1+8`。发布记录覆盖 macOS/Windows 桌面端、本地媒体任务、视频/音频/图片处理、FFmpeg/FFprobe 调用、队列、设置和 Drift/SQLite 状态。

可复核的工程经验包括：中间产物成功后再原子发布、输出检查、权限和重名错误处理、硬件编码失败回退、数据库迁移兼容。它们只作为已做过的产品经验，新主线仍要按新引擎边界重新实现和验收。旧 SQLite 数据不迁移，源媒体和用户输出媒体不属于代码清理范围。

旧版的 FFmpeg CLI 路径不作为新主线引擎调用方式。`develop/next` 的媒体请求必须经过 Flutter → Work → FLL；FLL 内部使用进程内 libav。

## Alpha：原架构、职责与调用链

Alpha 的目标拓扑和关键源码形成以下链路：

```text
Flutter EngineGateway
→ FEngine loopback daemon（随机端口与 endpoint token）
→ 子 Worker 进程（stdio）
→ Worker 请求队列、幂等、会话事件与 Snapshot 管理
→ DynamicRuntimeHost / libloading
→ framelean_fll_get_api C ABI
→ FLL Runtime / Pipeline / Scheduler
→ 进程内 libav
```

### 三个项目的职责

- `desktop-client/` 是 Flutter 前端，负责窗口和界面、应用设置、通知、本地应用数据以及 Client 侧的任务身份；通过 `EngineGateway` 调 FEngine，不直接加载 DLL，也不组装或执行 FFmpeg 命令。
- `fengine/` 是 Rust Work。它负责 Client/Work 协议、loopback daemon、endpoint/token、请求 admission、幂等键、队列与 session 事件、Client identity 映射和分析 Snapshot 持久化；Worker 进程通过 FFI 动态接入 FLL。
- `fll/` 是 Rust 媒体引擎，不是完整桌面应用。它负责媒体分析与能力决策、配置、Pipeline、Runtime、执行状态、资源调度和进程内 libav 适配。
- `protocol/` 是接口契约文档，不是第四个可执行项目。Flutter ↔ Work 使用长度帧 JSON；Work ↔ FLL 使用独立版本化 C ABI。

FLL ABI v1 使用单一 bootstrap 导出、版本化函数表、opaque Runtime handle、边界错误码和库内 `buffer_free` 释放结果。数据以有界 JSON 文档过 ABI，不跨越 Rust 类型、引用或 allocator 所有权。

### 已实现到什么程度

- **源码事实**：Alpha FLL Workspace manifest 实际列出 10 个 crate：core、media、analysis、environment、decision、ffmpeg、ffi、pipeline、plugin、runtime。Alpha 的 README 称“九个”，与 manifest 数量不一致，应以 manifest 为准。
- **源码事实**：FLL 已实现媒体分析与 Snapshot、部分配置决策、事件式 execution Runtime 和 Pipeline；FFmpeg adapter 使用 libav API，没有通过 FFmpeg CLI 完成这些执行。
- **源码事实**：Analysis Snapshot 可序列化并由 FEngine 保存/恢复；这不等于 FEngine 守护进程崩溃后的媒体任务续作。Alpha 文档明确没有完成跨进程 execution 恢复。
- **文档与代码事实**：确认写明的真实处理链只有兼容输入的 packet stream-copy/remux、受限 SDR 视频转码，以及 PCM/AAC 音轨到 AAC M4A。未资格化的 codec、HDR、字幕/数据/附件或其他组合应拒绝，不能描述为通用转码。

### Alpha 没有收敛的边界

- **源码事实**：FEngine manifest 仍依赖若干 FLL crates / `framelean-ffmpeg`，同时保留静态 `RuntimeHost`、`build_default_runtime()` 兼容路径。动态宿主虽已出现，但其周围仍有过渡 DTO 和旧 Runtime 路径。
- **设计判断**：新主线应让 FEngine 只依赖动态加载边界。否则同一引擎既静态又动态接入，测试、错误语义和发布构建会有两条真实运行路径，容易再次造成“代码到哪一步”的混乱。
- **源码与文档冲突 / 待重验**：Alpha `fengine/CONTEXT.md` 声称桌面发布会同目录携带 FLL 动态库；本次找到的 release workflow 和 Xcode/CMake 打包步骤明确构建/复制了 FEngine，但没有找到把 FLL 动态库放入最终应用目录的对应步骤。没有实际生成并启动 Alpha 安装包，所以这是强打包风险，尚非已复现的用户故障。
- **源码事实**：Alpha 协议范围远大于当前 smoke 所需，覆盖批量工作、队列重排、抢占恢复、事件 sequence、Snapshot 对账和 pause/resume/cancel。移植这些文本或 DTO 不等于新主线已实现这些语义。

## `develop/next` 已确认的重新开发边界

- 三个独立项目仍在一个 Git 仓库：`desktop-client/`、`fengine/`、`fll/`；`protocol/` 只维护契约。
- FEngine 是唯一本机 Work 服务，按需启动并在 Flutter 断线后继续运行；去掉 daemon 与子 Worker 的第二层进程桥。
- Flutter 仅经 `EngineGateway` 使用 Work。Work 仅通过动态 C ABI 加载 FLL；产品运行路径不静态链接 FLL，不启动 FFmpeg/FFprobe CLI。
- 当前代码先建立 `hello`、`ping`、`analyze_media` 的最小真实链路。它不代表 Alpha 的执行队列和媒体编码能力已经迁入，也不提前搬迁旧 UI 或旧数据库。
- 对 macOS 和 Windows，构建产物须并排包含 Work 与 FLL。发布验收需核对目标架构、ABI、依赖库和包内实际加载；当前尚未通过两平台验收。
