# 当前决策

| 日期 | 决策 | 状态 | 依据 |
| --- | --- | --- | --- |
| 2026-10-07 | `develop/next` 采用同仓库三项目：Flutter `desktop-client/`、Rust Work `fengine/`、Rust 动态引擎 `fll/`；`protocol/` 仅存契约 | 有效 | `docs/develop/restart-baseline.md`、`docs/develop/architecture.md` |
| 2026-10-07 | Work 只通过版本化动态 C ABI 加载 FLL；去掉 Alpha 静态兼容运行路径及 daemon→子 Worker 桥接 | 有效 | `protocol/fll-abi/v1/README.md` |
| 2026-10-07 | 从 ping 和媒体分析建立真实闭环后，再迁入第一个产品媒体功能；不迁移旧 SQLite | 有效 | `docs/work/active.md` |

旧 `main` 与 Alpha 线冻结归档，归档标签保持原提交。
