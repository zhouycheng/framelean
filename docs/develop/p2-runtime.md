# P2 运行与接口 / HTTP v4

当前版本：HTTP 4、FLL JSON 3、MediaFacts 2、ExecutionPlan 2、C ABI 1、SQLite user_version 2。CLI、Flutter 和 Worker 同步升级；旧 Worker 需通过对应客户端正式退出。媒体判断及参数只在 FLL，Worker 不链接 FLL/libav。

## 调用与媒体职责

沿用 [P1 模块边界与网关状态语义](p1-runtime.md)，所有接口前缀改为 `/v4/`。回环鉴权、请求 ID 去重、每批 1–100 项、独立 Host、单重媒体槽位、短查询有界通道、分页、SSE 和取消规则延续。Host 装载上限 30 秒，就绪后短查询上限 10 秒；取消 3 秒后强制回收。

| 边界 | P2 行为 |
| --- | --- |
| FLL analysis | 所有轨道事实增加语言、disposition、side_data；容器增加 chapter_count、program_count |
| FLL adaptation | 统一检查轨道、章节、附件、方向、像素、色彩和时间事实；无完整保留链则排除候选 |
| FLL capabilities | 报告 H.264/HEVC 解码器、libx264 初始化、MP4/Matroska 封装能力 |
| FLL planning | 每种媒体生成完整候选；预设只在这里解析；H.264 转换采用 remux，HEVC 转换采用 balanced transcode |
| FLL backend | 直接 libav 读写；转封装同时解码视频检查完整性，转码复用现有解码/编码链；显式检查写尾、flush、close |
| FLL validation | 重探测临时输出，按方案检查容器、轨道、语言/显示语义、尺寸、像素、色彩、时长和音视频偏移 |
| Worker | 保存不可变计划，按计划扩展名创建临时文件，调度/取消/重试，计算输出哈希并无覆盖发布 |
| 客户端 | 展示 FLL 候选和计划；选择配置，提交期望摘要，观察持久状态 |

输入 MP4/MOV/MKV，单 H.264/HEVC 8-bit yuv420p SDR 视频，零或单 AAC。输出 MP4/MKV 内 H.264 和可选 AAC。压缩三预设统一使用 medium、yuv420p；保持尺寸与相对时间线。方向信息、HDR、非方形像素、多轨、字幕、附件、章节及不能保留的显示语义保持无候选。

输出兼容性按目标容器筛选。带 `zh-Hans` 等扩展轨道语言标记的 MKV 仅提供可完整保留标记的 MKV 方案，MP4 被排除并返回 `mp4_language_unsupported` 诊断。Flutter 当前压缩目标为 MP4，查询候选时显式传入该目标；规则由 FLL 统一维护。

## options、plan、execute

```json
{"request_id":"query-1","analysis_ids":["analysis-1"],"intent":{"operation":"convert","preservation":"preserve"},"format":"mkv"}
```

发送至 `POST /v4/options`。`intent` 默认 compress/preserve；可选字符串 `format` 筛选目标容器。响应含逐项 `items[].result`、全批 `available_choices`、`default_selection` 和 `presets`（id、label、aliases）。压缩最多有三预设 × 两容器；转换最多有 auto × 两容器，取决于媒体事实与实际后端能力。任一输入无候选，则整批交集为空。

```json
{"request_id":"plan-1","analysis_ids":["analysis-1"],"intent":{"operation":"convert","preservation":"preserve"},"selection":{"preset":"auto","container":"mkv"}}
```

发送至 `POST /v4/plan`。计划中的字段示例：

```json
{"schema_version":2,"output":{"container":"mkv","muxer":"matroska","extension":"mkv"},"parameters":{"mode":"remux"}}
```

压缩或 HEVC 转换的 `parameters` 为：

```json
{"mode":"transcode","video":{"encoder":"libx264","crf":23,"preset":"medium","pixel_format":"yuv420p","width":1280,"height":720}}
```

尺寸来自实际媒体。其余字段包括 facts/source/capability 摘要、intent、selection、stream_map、steps、effects、validation、resource_hint 和 plan_digest。`h264_crf23` 在 FLL 规范化为 balanced。convert 的效果明确表示保留码流或有损重编码，调用者通过参数和步骤观察实际链。

`POST /v4/execute` 沿用 `items[{analysis_id,intent,selection,expected_plan_digest}]` 和 output_directory。Worker 经 Host 重新规划，整批有效后事务入队；不接收客户端自定义的可信 plan。输出名为 `原名-compressed.ext` 或 `原名-converted.ext`，冲突增加编号。

FLL ABI 请求的 schema_version 为 3；媒体与计划各自检查版本和摘要。旧等待计划要求重新分析/规划，旧任务重试不会改变参数。历史及旧发布中的结果按 [数据库规则](p1-database.md) 保留与对账。

## 失败和结果收敛

写尾、刷新或关闭任一步失败，FLL 返回处理错误；Worker 不发布结果并清理登记的临时文件。RAII 负责异常路径释放资源。只有临时结果通过校验、哈希和发布事务后，产品任务才变为 succeeded。

数据库错误使 Worker 进入可诊断 degraded 状态并停止新任务启动；事务保留已有等待记录。恢复存储后重启，再由用户继续队列。publishing 后取消返回拒绝，由发布/恢复流程完成状态收敛；卷不可访问进入 reconciling。

运行路径使用包内 libav，无 FFmpeg/FFprobe 子命令。FFmpeg CLI 仅用于样本准备与独立验收。当前为本机签名验证包；Windows、真实断电和物理拔盘行为分别记录，见 [验收记录](p2-validation.md)。
