# FrameLean CLI

CLI 通过本地 Worker 网关执行；独立 CLI 包包含 Worker、FLL 和依赖，无需运行 Flutter。

```sh
bash scripts/build/cli-macos.sh
outputs/cli-macos/framelean doctor
outputs/cli-macos/framelean compress /media/input.mp4 --output /media/output --wait
```

输出目录必须已存在、可写并支持同卷硬链接。全量桌面包内也包含 CLI，位置是 `.app/Contents/Resources/runtime/framelean`。Windows 独立构建使用 `scripts/build/cli-windows.ps1 -FFmpegRoot <SDK目录>`，产物为 `outputs/cli-windows`，复制整个目录。

## 命令

```sh
framelean --json analyze /media/a.mp4 /media/b.mov --wait
framelean --json options <analysis_id> <analysis_id>
framelean --json plan <analysis_id>
framelean --json compress --analysis-id <analysis_id> --output /media/output
framelean --json tasks --limit 50
framelean --json tasks --cursor <next_cursor>
framelean --json task <task_id>
framelean --json task <task_id> --cursor <next_cursor>
framelean --json watch <task_id>
framelean --json cancel --attempt-id <attempt_id>
framelean --json cancel --analysis-id <analysis_id>
framelean --json retry <task_id> --wait
framelean --json task <task_id> --reconcile
framelean queue pause
framelean queue resume
framelean queue status
framelean worker start
framelean worker status
framelean worker stop
```

compress 先分析、查询有效交集、规划，再提交整批任务。任何一项无有效方案时整批拒绝，并返回逐项原因。默认在准备完成并入队后返回 ID；`--wait` 等待最终结果。analyze 默认立即返回分析 ID，`--wait` 等待事实。

退出 watch 或按 Ctrl-C 只结束观察；任务继续运行，可用 ID 再次观察。准备阶段退出时，已经提交的分析继续，尚未创建的处理任务不会自动提交。Worker 重启后等待队列暂停，执行 `queue resume` 后继续。观察过程中 Worker 断开会返回连接错误；重新运行 watch 获取恢复状态。

`--json` 的 stdout 仅输出一个 JSON 结果，准备和进度信息写 stderr。`--request-id` 为写操作提供稳定去重键；重复使用时参数必须相同。CLI 按准备/执行步骤派生子键，防止重复入队。ID 适合由自动化调用者保留。

| 退出码 | 含义 |
| --- | --- |
| 0 | 请求接受、查询成功，或等待到成功 |
| 2 | 参数错误、无共同方案、方案失效、请求冲突 |
| 3 | Worker/组件不可用或协议不匹配 |
| 4 | 等待到失败、中断，或需要重新核对 |
| 5 | 等待到取消 |

异步命令退出码 0 只证明接受成功。当前普通文本输出为格式化 JSON，保持最小交互入口。

## 数据与诊断

默认数据在应用支持目录的 `FrameLean/data-v2`，运行连接在 `FrameLean/runtime`。macOS 基目录为 `$HOME/Library/Application Support`；Windows 为 `%LOCALAPPDATA%`。运行目录可清理，但应先正式退出 Worker；数据库与迁移备份保留在数据目录。

开发与测试可设置 `FRAMELEAN_RUNTIME_DIR`、`FRAMELEAN_DATA_DIR`、`FRAMELEAN_ENGINE`、`FRAMELEAN_FLL`。独立包运行时无需这些覆盖。`doctor` 返回组件版本与恢复状态，日志在运行目录 `worker.log`、`logs/host.log`；Host 日志轮换。endpoint.json 含私有令牌，不进入仓库或问题报告。
