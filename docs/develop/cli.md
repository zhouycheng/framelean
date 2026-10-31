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
framelean --json options <analysis_id> --operation convert --format mkv
framelean --json plan <analysis_id> --operation convert --format mp4
framelean compress /media/a.mov /media/b.mkv --preset quality --format mkv --output /media/output --wait
framelean convert /media/a.mkv --format mp4 --output /media/output --wait
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

compress 和 convert 先分析、查询有效交集、规划，再提交整批任务。任何一项无有效方案时整批拒绝，并返回逐项原因。默认在准备完成并入队后返回 ID；`--wait` 等待最终结果。analyze 默认立即返回分析 ID，`--wait` 等待事实。

## 操作、预设和目标格式

| 命令 | 参数与默认值 | 实际处理 |
| --- | --- | --- |
| `compress` | `--preset balanced`、`--format mp4` | 始终重新编码视频为 H.264，AAC 复制 |
| `convert` | 必填 `--format mp4` 或 `mkv` | H.264 转封装；HEVC 以均衡参数转为 H.264 |
| `options` | `--operation compress`；可选 `--format` 筛选 | 查询各项完整候选及批次交集 |
| `plan` | `--operation compress`、`--format mp4`；可选 `--preset` | 预览每个文件的实际执行链与参数 |

| 显示名称 | 标识 | FLL 解析结果 |
| --- | --- | --- |
| 画质优先 | `quality` | CRF 18 |
| 均衡 | `balanced`（默认） | CRF 23；兼容别名 `h264_crf23` |
| 体积优先 | `small` | CRF 28 |

三个预设均使用 libx264、medium、yuv420p，保留尺寸、帧时序和音视频相对偏移。它们表达质量和体积取舍，结果可能比原文件大。转换的候选标识为 `auto`；混合 H.264/HEVC 批次共享该选择，各文件分别使用转封装或转码链。CLI 的准备诊断写明实际模式，`plan.parameters.mode` 可供机器读取。

支持 MP4/MOV/MKV 输入、单 H.264/HEVC 8-bit yuv420p SDR 视频、零或单 AAC 音轨，输出 MP4/MKV。不具备完整保留链的方向信息、非方形像素、HDR、多轨、字幕、附件、章节等输入没有可选组合。媒体事实仍可查询。

目标格式也按媒体事实筛选。例如带 `zh-Hans` 扩展轨道语言标记的 MKV 可输出 MKV，MP4 选项会被排除；显式指定 MP4 时在入队前返回原因。

正式名称使用 `原名-compressed` 或 `原名-converted`，重名增加编号，扩展名取自 FLL 方案。Flutter 提供三个有效压缩预设并显示转换任务；转换入口当前在 CLI。

## 后台与输出

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

本版本 HTTP 4 / FLL JSON 3 / 事实 2 / 计划 2 / C ABI 1。旧 Worker 需用对应旧版客户端正式退出，再启动新版；新版不会自动强制替换。SQLite 保持版本 2，历史和输出保留，旧等待计划显示 `interrupted / plan_required`；重新分析并创建任务，重试不会自动改成新参数。
