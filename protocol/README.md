# FrameLean 跨项目契约

仓库有三个可独立构建的项目：`desktop-client/`、`fengine/` 和 `fll/`。本目录只保存跨项目契约，不是第四个运行时项目。

- Flutter Client ↔ Work：`v1/` 长度帧 JSON，当前 smoke 范围为认证握手、引擎健康检查、媒体分析和受控关闭。
- Work ↔ FLL：`fll-abi/v1/` 版本化 C ABI；FLL 的 Rust 类型和 Runtime Schema 由 `fll/` 独立拥有。

协议文档描述当前实际边界。Alpha 归档中的执行队列、恢复、快照持久化等完整协议能力并未自动迁入新主线。
