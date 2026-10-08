# Runtime 分析聚合 DTO

当前聚合 DTO 由 `framelean-runtime` 拥有，不单独创建 Protocol crate。

`AnalyzeMediaResponse` 分开表达：

- `media_analysis_status`
- `configuration_status`
- `environment_summary`
- `engine_backend_summary`
- `capabilities`
- `recommendation`
- 四个 `presets`
- 独立 `custom_target_size`
- 结构化 `warnings` 与 `error.code`

Runtime Schema 由 `cargo run -p framelean-runtime --example export_schemas` 生成到 `schemas/`。

当前 v1 在 0.1 阶段直接更新为完整候选链协议：`capabilities.execution_chains` 是原子链；Recommendation 的所有字段必须匹配其中一条链。`RecalculateConfigurationResponse.resolved_configuration` 仅在 Manual、Preset 或 Custom Target Size 成功解析到单一 Candidate 时返回。冲突时该字段为 null，Snapshot 和 revision 不变。

四个产品 Preset ID 为 `clear`、`balanced`、`chat`、`compact`。Preset 先按 Policy 硬过滤容器、Codec 和 HDR 链，再应用用户显式 overrides，不得选择业务不兼容 Candidate、不存在的 Backend 或覆盖用户字段。当前没有 execution-ready 全链时，presets 与 custom target size 仍保留在响应中，但标记不可用。

顶层 `warnings` 聚合媒体分析 Warning 与执行链 Warning；`error.retryable` 由稳定错误码决定。`engine_backend_summary` 分别报告总 Backend、Native Provider Backend 与 Plugin Backend 数量。

`AnalyzeTaskRequest` 直接持有 OS-native `PathBuf`，当前不是 JSON/IPC 输入协议。`AnalysisId` 只在同一 EngineRuntime 会话内有效，不得跨独立 CLI 进程使用。
