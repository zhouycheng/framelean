# 当前架构

## 项目边界

FrameLean 在一个 Git 仓库中有三个可独立构建的项目，`protocol/` 保存跨项目契约，不构成第四个运行时项目：

```text
Flutter desktop-client
        │ protocol/v1: loopback TCP + length-prefixed JSON
        ▼
Rust fengine (Work service)
        │ fll-abi/v1: versioned C ABI + bounded JSON
        ▼
Rust fll (dynamic library → in-process libav)
```

- `desktop-client/` 拥有窗口、UI、设置、通知和应用本地数据。`EngineGateway` 是它唯一的 Work 调用入口；前端不加载 FLL，也不启动 FFmpeg CLI。
- `fengine/` 是唯一本机 Work 服务。它保存随机 loopback endpoint/token，接收 Flutter 请求，并动态加载旁边的 FLL 库；不派生第二层 Worker 子进程，也不直接依赖 FLL crate。
- `fll/` 拥有媒体分析、Runtime、Pipeline、调度和执行；FFmpeg 能力由进程内 libav adapter 提供。引擎能力不包含桌面设置或应用数据库。
- FLL 当前来自 Alpha 归档代码，目标处理范围受 `fll/README.md` 与实现约束；本次没有把 Alpha 全部处理能力迁成产品可用功能。

## 当前运行闭环

Flutter 按需 detached 启动 FEngine。Work 在当前用户应用数据目录原子写出 endpoint 状态、loopback 随机端口和访问 token。每个 TCP 连接先进行 `hello` token 校验，再发送一个 v1 请求；客户端断开后，Work 继续运行，新连接可以再次认证。

当前支持的真实操作只有：

1. `ping` 通过 C ABI 调 FLL Runtime 健康检查。
2. `analyze_media` 将请求经 Work 边界映射后交给 FLL，并返回真实媒体分析结果。
3. 开发/烟测用 `shutdown` 受控关闭 Work。

当前 smoke 不包含 UI 产品迁移、数据库、队列持久化、媒体执行控制、进度事件或崩溃续作。

2026-10-07 的 macOS arm64 本机 smoke 已通过真实静态 libav SDK 和 FLL Release 动态库：Flutter gateway 返回 PCM WAV 的 8000 Hz、单声道分析，新建 Gateway 后 Work PID 保持一致。2026-10-08 进一步完成 Release App / ZIP 打包、包内 smoke，以及从最终 ZIP 解压启动的 App 默认路径连接与 WAV 分析。Windows 链路仍待验收。

## 动态 ABI 与打包

FLL 只公开 `framelean_fll_get_api` bootstrap。Work 检查 ABI major/minor 和函数表大小，使用 opaque handle；FLL 返回的缓冲区由 FLL 自己释放。缺少库、导出符号不存在、ABI 不兼容或 Runtime 初始化失败时，Work 返回清楚的启动失败状态，不回退到静态 FLL。

macOS `.app` 中 Work 与 dylib 放在 `Contents/Resources/`；Windows 将 Work exe 与 DLL 放在 Runner exe 同目录。`build/runtime/<target>/` 是两个 Rust 项目的暂存输出，不提交到 Git。macOS arm64 本机包已核对目标架构、动态依赖、签名完整性和包内实际加载；Windows 仍待对应平台验收。

macOS 本地打包在复制 runtime 后重新 ad hoc 签名，检查 App 完整性及 ZIP 压缩数据，输出完整条目清单和 SHA-256。源码与包内容的核对方案见 `docs/develop/package-checklist.md`。本机 arm64 验证不代表 Developer ID 签名、公证或其他平台已验收。

## 当前未建立的能力

应用数据库、媒体任务产品流程、执行队列与控制协议、进度事件、安装更新、Universal 2 macOS 包以及跨平台签名/公证都不属于当前已验收能力。Alpha 中对应实现只可作为后续证据来源，迁入时必须按本边界逐项验证。
