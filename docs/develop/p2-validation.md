# P1 本机收尾与 P2 视频处理验收

日期：2026-10-31；分支：develop/next。P1 macOS 收尾和 P2 首批视频转换、三压缩预设已实现并完成下述本机验收。Windows 构建与运行由用户单独实测，本记录不标记双平台完成。

环境：Apple Silicon / macOS，Rust 1.98.1、Flutter 3.47.5、FFmpeg 9.0.2。HTTP 4 / FLL JSON 3 / 事实 2 / 计划 2 / C ABI 1 / SQLite 2。素材来源、署名、许可、指纹及基础截取方式沿用 [P0 记录](p0-validation.md)；新增夹具由测试脚本从该素材派生，原始日志和媒体位于忽略的工作目录或系统临时目录。

## 已完成的修正

- FLL 显式检查 av_write_trailer、avio_flush 后的写错误及 avio_closep。失败路径保留 RAII 资源释放；写入错误返回失败，Worker 清理登记临时文件并保留源文件。
- HEVC 有限色彩范围向 H.264/MP4 转换时，部分输入只有 range 明确、其他色彩字段未知。输出主动写 colr 保留 range，严格色彩校验继续生效。
- 临时文件清理依据执行 ID、登记扩展名结构和文件身份确认归属，覆盖 MP4/MKV；修复旧 MP4 名称检查导致 MKV 被保留的问题。其他身份的文件保持保留和诊断。
- 三预设统一由 FLL 解析。执行计划明确 remux/transcode、目标容器和扩展名；Worker 负责队列、取消、持久化、发布与恢复。
- 提交前审查发现 MKV 扩展语言标记转为 MP4 后无法保留。FLL 现在在候选生成时排除该目标，MKV 方案继续可用；Flutter 按当前 MP4 目标查询，避免展示无法执行的选择。

## 自动验收

脱敏机器结果见 [p2-macos-results.json](p2-macos-results.json)。P1 基线先验收，再升级 P2；首次验收和提交前复核分别保存在 `results` 与 `pre_commit_review`，下列 v4 自动检查已在最终修复的运行包上重跑。

| 检查 | 结果与范围 |
| --- | --- |
| P1 基线 | 原 v3 的 15 组回归和新增 4 组边界检查通过；保留在报告中区分历史版本 |
| v4 核心回归 | 15 组通过：事实/未知字段、旧协议诊断、计划防篡改、原子批次/去重、重试/唯一活动执行、取消竞争、重连、故障恢复与分页响应 |
| 视频支持矩阵 | 12 种输入：H.264/HEVC × MP4/MOV/MKV × 有/无 AAC；各跑 compress/convert × MP4/MKV，共 48 组合通过 |
| 预设和批量 | 三预设实际码流 CRF 18/23/28 与计划一致，旧别名规范化；H.264/HEVC 同批共享 auto 选择，各自用 remux/transcode |
| 不支持素材 | 旋转、HDR、多轨、其他视频编码、HEVC 10-bit、字幕、附件、章节无候选；损坏头部分析失败，后半段损坏在压缩和转封装执行时失败且不发布 |
| 容器兼容性 | 扩展轨道语言标记提前排除 MP4，压缩和转换均可保留为 MKV；转换脚本共 18 组检查通过，包含上述 48 组合 |
| 转换生命周期 | 8 组通过：Host 崩溃、活动取消、运行 Worker 中断、四个发布节点中断、持久取消、观察退出/重连、重名/源变化/目录缺失、旧计划恢复 |
| 存储与时间边界 | compress 与 convert 各 4 组通过：VFR 和 300 ms 起始偏移、只读目录、真实输出写满、真实 SQLite 卷写满 |
| 组件与代码检查 | 契约 2 项、Worker 5 项、CLI 1 项测试；Worker/FLL Clippy warnings-as-errors 通过 |
| Flutter | analyze、2 项 widget 测试、1 项真实 Worker 执行和重连测试通过；画质优先计划确认为 CRF 18 |

48 组合逐项独立解码，核对完整帧数及逐帧 PTS（差异上限 2 ms）。转封装额外比较解码后的逐帧 MD5，避免将容器/数据包哈希当作视频内容证明。FLL 校验轨道、编码、尺寸、像素、色彩、语言/显示语义、时长和音视频偏移；基础回归另保留 AAC 数据及时间戳检查。

压缩与转换的写满测试都使用独立 **64 MiB HFS+ 稀疏映像**，断言卷总量小于 100 MiB 后才填充。输出 ENOSPC 返回失败、没有正式产物、登记临时文件被清理，源 SHA-256 保持一致。数据库 ENOSPC 回滚新批次，health 显示 degraded，已有等待项保留；释放空间、重启后队列保持暂停。测试结束卸载专用卷。

v4 回归继续覆盖目录离线模拟后 reconciling、不同身份输出冲突、清理失败、10,000 条历史分页、慢 SSE，以及 512 MiB 登记文件的哈希核验期间查询响应。512 MiB 夹具验证恢复 I/O，不代表同体积视频转码性能。

拆分提交时，单独导出只含 P1 输出关闭修复和边界脚本的暂存快照，重新构建 HTTP v3 包，15 组基线和 4 组边界检查通过。最终 P2 包重跑 15 组核心、18 组转换、8 组生命周期以及压缩/转换各 4 组边界检查；Rust、Clippy、Flutter 静态检查和真实 Worker 重连均通过。原生界面及播放观察保留本轮首次完整验收记录。

## 数据升级与发布

在升级前实际用旧 v3 Worker 生成一个完成任务及一个等待任务，再由 v4 打开同一测试数据目录：已完成历史、正式文件保留，旧 queued 变为 interrupted/plan_required，user_version 保持 2。

生命周期测试另将真实发布中断记录的计划改为旧 schema 1 形态，重启仍按登记路径、身份和哈希补记成功、完成清理；旧等待计划的 retry 返回 plan_required。v1→v2 的实际历史行迁移、异常结构与事务回滚仍由 repository 测试验证。任务参数没有在升级或重试时自动替换。

## 包与界面实测

独立 CLI 目录复制到新位置，使用最小系统 PATH，移除 FRAMELEAN_ENGINE/FLL 和 DYLD 覆盖配置，画质优先压缩与 MKV 转换成功。移走副本内 libavutil 后出现 component_unavailable。正式包仍保留完整依赖，缺依赖测试仅修改临时副本。

桌面 .app 同样复制到独立位置，使用隔离数据库与运行目录、系统 PATH，从包内加载组件。通过原生文件选择导入中文/空格路径的 120 秒 1280×720 素材，选择“画质优先”并入队。关闭窗口后同一 Worker 的进度从约 14% 推进至约 77%；重新以同一隔离配置打开客户端，显示已完成结果。CLI 发起 MKV 转换后 Flutter 显示 convert/remux/mkv 和真实输出。正式退出后 endpoint 消失，5 个执行均为 succeeded，cleanup_state 均为 done。

| 120 秒样本 | 字节数 | 解码帧数 | 相对源逐帧 PTS 最大差异 |
| --- | ---: | ---: | ---: |
| 源视频 | 91,650,939 | 2,880 | — |
| quality 压缩 MP4 | 50,690,161 | 2,880 | 0 |
| 转封装 MKV | 91,652,896 | 2,880 | 0.333 ms |

QuickTime 实际打开并播放压缩 MP4，显示两分钟时长及正确画面。独立解码和时间检查通过；本轮未进行人工听音。MKV 比源略大属于正常封装开销，结果界面如实显示。

## 质量与体积观察

同一 3 秒、320×180、72 帧 H.264/AAC 样本，源 387,379 字节，输出 MP4。耗时为一次 CLI 提交到成功的墙钟时间，包含规划与轮询；测量期间有其他验收任务，作为记录而非性能基准。

| 预设 | CRF | 输出字节 | 耗时（秒） | 相同源 SSIM |
| --- | ---: | ---: | ---: | ---: |
| quality | 18 | 272,908 | 1.673 | 0.985657 |
| balanced | 23 | 165,921 | 1.719 | 0.963125 |
| small | 28 | 110,957 | 1.675 | 0.925148 |
| h264_crf23 别名 | 23 | 165,921 | 1.866 | 0.963125 |

查看同一第 2 秒画面：三者构图、尺寸和颜色一致，small 的草地与叶片细节更平滑、块状痕迹更明显，quality 保留的纹理较多。这是单一低分辨率片段的观察，不能代替不同真实素材上的主观质量评估；不以固定压缩比例作为通过条件。

## 复现

在仓库根目录执行。先完成构建，再开始测试，避免运行中的包被构建脚本替换。

```sh
python3 scripts/test/prepare_media.py
bash scripts/build/macos.sh
args=(--engine outputs/cli-macos/framelean-engine
      --library outputs/cli-macos/libframelean_fll.dylib
      --cli outputs/cli-macos/framelean
      --sample .workspace/p0-media/bbb-30s.mp4)
python3 scripts/test/v4_integration.py "${args[@]}"
python3 scripts/test/p2_conversion.py "${args[@]}"
python3 scripts/test/p2_lifecycle.py "${args[@]}"
python3 scripts/test/media_edges.py "${args[@]}" --operation compress
python3 scripts/test/media_edges.py "${args[@]}" --operation convert
python3 scripts/test/packaged_cli.py --runtime outputs/cli-macos --sample .workspace/p0-media/bbb-30s.mp4
```

FFmpeg/FFprobe CLI 仅用于样本与独立校验。新增视频矩阵需测试用 libx265 编码器，产品执行只需 HEVC 解码器。数据目录、素材、原始日志、凭证和安装产物不提交到仓库。

## 交付边界

macOS CLI 和桌面验证包已构建并实测；本机 ad-hoc 签名，尚未公证或发布。Windows 脚本增加 P2 能力探测，桌面构建清除旧 runtime 后复制新依赖；尚未在 Windows 编译/运行，按 [W01–W28 清单](p0-windows-checklist.md)记录。

本轮本地交付压缩包为 `outputs/framelean-p2-cli-macos-arm64.zip` 与 `outputs/framelean-p2-desktop-macos-arm64.zip`，校验值在 `outputs/framelean-p2-macos-sha256.txt`。ZIP CRC、桌面深度签名检查、22 个包内二进制的依赖路径检查通过；Worker 没有直接链接 FLL/libav。构建脚本可重新生成对应 CLI 目录和 .app。

真实断电、物理拔盘、网络文件系统、长期压力及人工听音待单独验证。SourceStamp 仍为文件身份、大小、mtime 和首尾抽样，输出发布使用全量 SHA-256。当前串行执行；缩放、目标体积、分段、音频转码、图片和 Agent 留在后续阶段。HTML 完成版本、状态、结构和链接核对；本轮没有重新渲染 HTML 页面。
