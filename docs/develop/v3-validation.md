# FLL / Worker 内部落地与 CLI MVP 验证

本文为 HTTP v3 历史验收；当前版本和复现命令见 [P1 收尾与 P2 验证](p2-validation.md)。

日期：2026-10-31。分支：develop/next。固定压缩能力、FLL / Worker 分层及 CLI 已完成本机链路验证；Windows 单独待验。

## 已实现

- 纯数据共享契约；FLL 来源、分析、能力、集中适配、规划、执行、libav 后端和校验模块。未知事实保留状态，特殊媒体可分析，缺少执行链的配置不进入选择列表。
- FLL 解析固定预设，执行重新核对完整方案与源文件；验证尺寸、容器、轨道、音视频相对时间和媒体参数。编码错误、损坏帧或中途格式/色彩变化终止任务。
- Worker 串行协调器独占写入，重 I/O 在事务外。v1→v2 事务迁移与备份、不可变计划、取消意图、独立清理状态、发布身份和哈希对账。
- 本地 HTTP v3、分页历史、快照和有界 SSE；Rust CLI 独立入口，Flutter 最小操作链及重连。C ABI 1 / JSON 2 / libav 主版本检查。
- macOS 独立 CLI 包与桌面包；Windows 对应构建脚本与清单。

## 已验证

本机 Apple Silicon / macOS，Rust 1.98.1、Flutter 3.47.5、FFmpeg 9.0.2。媒体素材沿用 [P0 来源、许可与指纹记录](p0-validation.md)。新增旋转/HDR 标签、多音轨、无音轨、损坏和大文件夹具仅在隔离测试目录产生。

共享契约 2 项测试、Worker 5 项测试、CLI 1 项测试通过；覆盖未知值、命令未知字段、摘要顺序、真实 v1 历史升级、重复打开、迁移失败回滚、异常结构、外键与唯一约束、事件游标和清理身份。Worker/FLL Clippy（warnings as errors）通过。Flutter analyze 与两项测试通过，其中一项使用真实 Worker 执行压缩并重建客户端连接。

包内版本的 [15 组集成结果](v3-macos-results.json) 已通过：

- 混合素材事实与有效交集、不可执行选项排除、无效批次原子拒绝、请求去重、同任务唯一活动执行。
- JSON/ABI 不匹配、未知字段、未知尺寸、计划被修改、源文件硬链接保护；输出码流内的 CRF 23 与 medium 对应参数检查。
- Big Buck Bunny 真实视频与无音轨压缩；中文/空格路径、重名保护、独立解码、解码帧数和逐帧 PTS；AAC 数据包哈希与时间戳逐项相同。后半段损坏素材能探测，执行失败且没有发布残缺文件。
- Host 崩溃后继续处理；macOS SIGSTOP 挂起 Host 后，取消在 3 秒强制回收上限附近完成；CLI 退出观察不取消任务，重新观察取得终态。
- 执行时杀死 Worker；在校验、发布意图、文件链接、成功提交及取消意图提交节点实际 abort 后重启，对账状态符合预期。发布中的取消被拒绝，结果成功收敛。
- 目录暂不可访问进入 reconciling，恢复目录后显式核对成功；相同内容但不同文件身份的冲突文件保留；临时文件被替换时保留文件并记录清理错误，已成功结果保持成功。
- 一万条执行历史仍按 50 条分页；慢 SSE 订阅不阻塞查询。512 MiB 登记文件的恢复哈希在事务外完成，查询响应在测试的 2 秒断言以内。该夹具验证文件对账，未宣称是 512 MiB 视频压缩验收。

独立包测试将 CLI 分发目录复制到新位置，移除开发 PATH 与 FRAMELEAN_ENGINE/FLL 覆盖配置，仅保留隔离数据目录；无需 Flutter 完成压缩。移走副本内 libavutil 后出现 component_unavailable 诊断。首次冷装载曾触发原 5 秒超时，现为装载最多 30 秒、就绪后短查询最多 10 秒，修复后通过。依赖闭包包含 20 个 dylib 文件，包内引用已改写。

原生 Flutter 验证使用独立应用副本与数据库：通过文件选择导入 120 秒、1280×720 测试视频，界面入队。关闭窗口时 Worker 和会话不变，后台进度约 21%；重开窗口显示约 52% 的实时进度。末帧修复后重新压缩的结果为 28,217,668 字节，源文件 91,648,721 字节；解码得到完整 2,880 帧，与源逐帧 PTS 最大差异为 0。旋转素材显示基础事实和原因，界面没有对应压缩选择框。正式退出后 endpoint.json 消失，执行为 succeeded、cleanup_state 为 done。

## 复现

```sh
bash scripts/build/macos.sh
python3 scripts/test/v3_integration.py \
  --engine outputs/cli-macos/framelean-engine \
  --library outputs/cli-macos/libframelean_fll.dylib \
  --cli outputs/cli-macos/framelean \
  --sample .workspace/p0-media/bbb-30s.mp4
python3 scripts/test/packaged_cli.py \
  --runtime outputs/cli-macos \
  --sample .workspace/p0-media/bbb-30s.mp4
```

FFmpeg/FFprobe CLI 仅用于测试夹具与独立验证。原始日志、测试数据库、素材和端点位于忽略的工作目录或系统临时目录；正式结果只记录去除私有路径和凭证的验收项。

## 待验证与边界

Windows 尚未编译、运行；执行 [Windows 清单](p0-windows-checklist.md) 后单独记录。目录离线测试使用重命名模拟，物理拔盘、真实断电、网络卷和 Windows 文件锁行为待验证；本机普通磁盘和 512 MiB 核验不代表所有设备的性能保证。未进行本轮人工听音验收。

当前串行执行、固定 H.264 软件压缩；动态并发、通用格式转换、特殊格式处理链、Android、MCP 和 Agent 属于后续阶段。SourceStamp 使用抽样检测而非源文件全量哈希；已关闭或断开的观察者重新查询持久状态，运行中的编码不从旧百分比续作。

HTML 已更新实施记录并进行结构与相对链接检查；本地页面预览被工具拒绝，本轮没有重新渲染该 HTML。原生应用界面已实际查看。

逐帧补验发现过末帧呈现问题：原代码未启用 AV_CODEC_FLAG_FRAME_DURATION，MP4 最后样本缺少呈现时长。已传递有效帧时长，并把解码帧数及逐帧 PTS 加入真实素材回归；数据包数量与容器总时长不能替代此项验证。依据 [FFmpeg 9 编码器标志说明](https://ffmpeg.org/doxygen/9.0/group__lavc__core.html)。
