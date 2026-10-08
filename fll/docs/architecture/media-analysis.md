# Media analysis 与能力结果

FLL 的分析 Runtime 将媒体事实、环境、Backend catalog 和 Decision 结果组合为分析响应和可版本化 Snapshot。媒体是否成功探测与配置是否存在完整执行链是不同状态；不能仅凭 Native 库发现推导某种转换已可执行。

这份实现代码选自 Alpha 归档。Rust analyzer、Runtime 和 Schema 的实现事实位于 `crates/framelean-analysis`、`crates/framelean-runtime` 与 `schemas/`。当前 `develop/next` 的 Work 只把 `analyze_media` 作为一个操作映射到 C ABI；旧 Client 对候选配置、Snapshot 恢复和执行提交的完整协议没有迁入。
