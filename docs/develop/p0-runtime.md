# P0 运行与接口

实施进度：技术贯通。P0 是单视频执行链，P1 才引入持久化任务队列。

## 进程和生命周期

Flutter 发现或按需启动唯一 Worker。Worker 持有系统文件锁，随机回环端口与私有认证令牌保存在用户应用运行目录。macOS 运行目录为 `~/Library/Application Support/FrameLean/runtime`，Windows 为 `%LOCALAPPDATA%\FrameLean\runtime`。测试可用 `FRAMELEAN_RUNTIME_DIR` 隔离目录。

Worker 每个操作启动一个 Host，通过 stdin 发送 JSON 请求、通过 stdout 读取 JSON 事件；stderr 写入私有 `host.log`。只有 Host 加载 FLL。能力探测同样在独立 Host 内完成。

关闭窗口保留 Worker；重新打开以快照恢复状态。软件内“退出软件”会确认后取消任务、等待后台清理并退出。macOS 菜单 Quit/Cmd-Q 进入同一确认流程。Worker 被杀时 Host 检测控制通道 EOF 并退出，未完成输出不会发布；P0 不恢复此类任务，遗留隐藏临时文件需要人工确认后清理。

## HTTP v1

从 `endpoint.json` 读取地址和令牌；全部请求使用 `Authorization: Bearer <token>`。拒绝带 Origin 的浏览器请求。POST 为 UTF-8 JSON，最大 64 KiB。入口只有本机回环地址，无远程监听。

| 方法与路径 | 请求内容 | 返回 |
| --- | --- | --- |
| GET `/v1/health` | 无 | 协议版本、Worker PID、FLL 探测与编码器可用性 |
| GET `/v1/snapshot` | 无 | instance_id、seq、当前 task、组件状态 |
| GET `/v1/events` | 无 | SSE；首条为完整快照，之后发送变更快照和心跳 |
| POST `/v1/analyze` | request_id、input（本地路径） | 接受后的任务；最终事件包含 result 和 analysis_id |
| POST `/v1/execute` | request_id、analysis_id、output_dir | 接受后的任务；固定预设，无任意命令或参数透传 |
| POST `/v1/cancel` | request_id、task_id | accepted；最终状态另由事件确认 |
| POST `/v1/shutdown` | request_id | accepted；取消执行、清理完成后服务退出 |

状态为 running、validating、succeeded、failed、canceled。响应和事件使用 request_id/task_id 关联；错误为 `{code,message}`。API 拒绝请求时返回 HTTP 400，未授权为 401，浏览器来源为 403。当前仅保留最近分析记录；再次分析后旧 analysis_id 失效。

任务接受响应带 instance_id/seq，客户端按序号合并，防止稍晚到达的 HTTP 响应覆盖已完成的 SSE 状态。重连读取快照，不依赖完整历史事件回放。Worker 重启后 instance_id 改变，客户端清除过期分析记录。

## FLL ABI v1

`fll_abi_version` → `fll_versions` → `fll_start` → `fll_poll`/`fll_cancel` → `fll_release`。JSON 缓冲区用 `fll_free` 释放；操作句柄在终态后释放，释放不与其他句柄调用并发。Host 使用锁保护控制线程与释放操作。

FLL 请求为 `{kind:"analyze",input}` 或 `{kind:"execute",input,temp}`。Host 每 100 ms 轮询，Worker 将执行进度映射至事件。取消先请求协作式停止，3 秒未退出则终止 Host。

## 输入与输出

分析拒绝多视频、多音轨、字幕/附件/数据轨、非 H.264、非 yuv420p、奇数尺寸、HDR/BT.2020、旋转矩阵和非方形像素。AAC 音频直接复制，编码线程数固定 2。

执行前后比较源文件大小与修改时间；这用于检测常规源文件变化，不是针对恶意修改的内容完整性证明。输出先写 `.framelean-<task_id>.partial.mp4`，关闭媒体输出后再次分析尺寸、时长和轨道；时长允许 250 ms 封装误差。

正式结果命名为 `<原名>-compressed.mp4`，重名递增。用同卷硬链接发布以避免覆盖竞争，成功后删除临时名称；目录需要支持硬链接（如 APFS/NTFS）。不支持时返回错误而不覆盖已有文件。发布前同步文件内容；取消与发布在 Worker 状态锁内顺序提交。

## 开发与打包

依赖通过各平台 SDK 头文件生成绑定，Rust 锁文件固定依赖版本。macOS 脚本递归收集非系统 dylib，改写到 `@loader_path`，重新签名并启动包内探测。Windows 脚本将同一 SDK 的 DLL 放入 runtime；需要 Windows 实际构建与测试。

本机编码包来自已安装 FFmpeg 9.0.2（启用 GPL/libx264）；许可证文本沿用旧项目。当前没有执行公开分发、公证或发布。
