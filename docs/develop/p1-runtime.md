# P1 运行与接口 / v3

当前实现采用 HTTP v3、FLL C ABI 1、媒体 JSON schema 2、数据库 user_version 2。CLI 与 Flutter 同步升级。旧 Worker 需要正式退出后再启动新版；旧 HTTP 接口返回 protocol_mismatch。数据库路径沿用 `data-v2/framelean-v2.sqlite3`，文件名中的 v2 是产品数据域，与 SQLite user_version 分开。

## 模块边界

| 模块 | 当前职责 |
| --- | --- |
| `crates/fll-contracts` | 纯数据：事实、known/unknown/estimated、意图、完整候选、计划、校验报告、事件和错误；不依赖 libav |
| FLL `source` / `analysis` | 文件身份与首尾抽样指纹、全部轨道基础事实、未知字段、可复用分析 |
| FLL `capabilities` / `adaptation` / `planning` | 后端主版本与组件探测、16×16 yuv420p 软件编码器初始化检查、集中媒体规则、固定预设解析、完整方案与摘要 |
| FLL `operations` / `backends/libav` / `validation` | 重新核对源及方案、按方案配置编解码器、协作取消、重开输出校验 |
| Worker `coordinator` / `repository` | 串行状态变更、短事务、请求去重、取消意图、FIFO 单重媒体槽位 |
| Worker `gateway` / `queries` / `events` | 回环鉴权、分页读取、内存进度、会话事件缓冲与快照 |
| Worker `host` | 独立进程加载 FLL、ABI/JSON 协商、受管管道、3 秒取消上限及回收 |
| Worker `publisher` / `storage` | 文件核验、发布登记、无覆盖硬链接、恢复对账、文件身份保护与清理 |
| Worker `service_runtime` / CLI `client` | 单实例与数据锁、运行目录、启动/正式退出、端点发现 |

只有 Host 模式加载动态库；Worker 不链接 FLL/libav。FLL 生成 CRF 23、medium、yuv420p、MP4 参数，Worker 保存和传递方案。当前固定执行链直接位于 libav 后端；适配规则集中在一个模块，后续实际接通特殊媒体处理时再拆成特征模块。

## 网关

随机回环端口，私有 endpoint.json 保存地址、凭证、进程和会话 ID。Bearer 鉴权；拒绝带 Origin 的请求。POST 上限 64 KiB、每批 1–100 项。常规连接最多 16，SSE 最多 8；短 FLL 查询独立单槽位、就绪后 10 秒超时；Host 首次动态库装载另有 30 秒上限，忙时返回 query_busy。重媒体执行与查询不共用槽位。

| `/v3/` 路径 | 方法与主要字段 | 响应 |
| --- | --- | --- |
| `health` / `snapshot` | GET | 组件/恢复状态；snapshot 另含暂停、队列计数、持久 revision、内存 active 与 event_id |
| `analyze` | POST inputs | analysis_ids，逐项异步分析；可识别但不能压缩的媒体仍保存事实 |
| `analyses` / `analyses/{id}` | GET；列表 limit/cursor | 分页事实或单项信息 |
| `options` | POST analysis_ids，选填 intent | 各项 available_choices / diagnostics，以及全批有效交集 |
| `plan` | POST analysis_ids、selection，选填 intent | 每项不可变计划或结构化错误 |
| `execute` | POST items、output_directory；每项 analysis_id、intent、selection、expected_plan_digest | 重新规划核对，整批原子入队；task_ids、attempt_ids |
| `tasks` / `tasks/{id}` | GET；limit/cursor | 列表与最新尝试；详情含分页历史、输出和原方案 |
| `cancel` | POST attempt_id 或 analysis_id，二选一 | 先持久取消意图；publishing/reconciling 返回 accepted=false |
| `retry` | POST task_id | 核对原方案后新增尝试；仅 failed/canceled/interrupted 可重试 |
| `reconcile` | POST attempt_id | 对 reconciling 执行重新核对 |
| `queue/pause` / `queue/resume` | POST | 暂停只阻止后续启动 |
| `shutdown` | POST | 暂停等待项、取消当前工作、完成发布和清理后退出 |
| `events` | GET SSE；Last-Event-ID | progress/state_changed/lifecycle；游标失效时 reset |

写命令带 request_id（1–127 字符）。规范化内容加协议版本、操作名生成摘要；相同请求返回原回执，不同内容返回 request_conflict。纯查询重新计算。execute 中客户端不能直接注入任意编码方案；仅能确认 FLL 生成的摘要。

列表默认 50、最大 200，cursor 为返回的 next_cursor。任务列表按初次入队顺序倒序；详情历史按尝试入队顺序倒序。持久 revision 只随关键状态变化；SSE 序号随会话递增，不用它替代数据库版本。进度最多每 250 ms 推送，事件环保留 1024 条；Flutter 先 snapshot，再用 event_id 订阅，断线后重新取得快照与分页历史。

## FLL C ABI 与内部请求

导出 `fll_abi_version`、`fll_versions`、`fll_start`、`fll_poll`、`fll_cancel`、`fll_release`、`fll_free`。Host 核对 ABI、JSON 和 libav 主版本，再使用动态库。

```json
{"schema_version":2,"operation_id":"operation-1","operation":{"kind":"analyze","input":"/media/input.mp4","cached":null}}
```

kind 支持 capabilities、analyze、options、plan、execute。命令未知字段被拒绝；媒体事实未知值显式保存为 `{"state":"unknown","value":null,"reason":"not reported"}`。详尽类型以共享契约为准。poll 返回 owned JSON 缓冲区，必须由 fll_free 释放。终态之后释放句柄并等待操作线程结束，再卸载动态库。

execute 收到 media、plan、input、temp 和 grant。FLL 重探测源并重新生成方案进行完整等值检查，执行消费确认后的参数。grant 当前为每个编解码器最多 2 个工作线程；未提供全进程 CPU/内存硬隔离。Host 控制管道断开会退出；超时或异常路径均回收子进程。

当前 SourceStamp 使用原生文件身份、大小、mtime、首尾各 1 MiB 摘要（小文件全读）。它用于普通文件变化检测；同身份、同 mtime 且首尾不变的中部恶意改写不在本轮完整性保证内。输出全量 SHA-256 用于发布对账。

[数据库](p1-database.md) · [CLI](cli.md) · [本轮验证](v3-validation.md)。P0/P1 v2 旧测试脚本保留历史用途，本版本使用 `scripts/test/v3_integration.py`。

CLI watch 当前每秒读取任务详情与快照；Flutter 使用 SSE。两者共享相同持久状态与选项，不分别判断媒体兼容性。
