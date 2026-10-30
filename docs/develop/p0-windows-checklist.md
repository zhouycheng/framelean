# Windows 构建与验收 / CLI 与 v3

状态：**待用户实测**。macOS 验证不能替代本清单。先以 Windows x64 / NTFS 本地目录测试。

当前包含独立 CLI、FLL 分层、v3 网关和数据库 v2。独立 CLI 构建无需 Flutter；完整桌面包仍需要 Flutter。重启后等待队列保留且暂停，显式继续。全部 Windows 项目待实测。

## 1. 构建环境

需要 Flutter（本轮 macOS 使用 3.47.5）、Rust MSVC 工具链、Visual Studio 的 Desktop development with C++、CMake、LLVM/libclang，以及一套匹配架构的 FFmpeg shared 开发 SDK。优先采用 FFmpeg 9.0.2，必须含 libx264 编码器；头文件、导入库与 DLL 来自同一构建。

SDK 应具有 `include/libavcodec/avcodec.h`、`lib/avcodec.lib` 等 MSVC 可用导入库和 `bin/avcodec-*.dll` 等动态库。绑定构建需要完整 FFmpeg 头文件与各 libav 库。若只有 ffmpeg.exe，不能用于编译 FLL。LLVM 未自动发现时设置 `LIBCLANG_PATH` 指向 libclang.dll 所在目录。

在仓库根目录的 PowerShell 中执行：

```powershell
flutter doctor -v
rustc -V
# 将 SDK 路径替换为你实际准备的目录。
.\scripts\build\cli-windows.ps1 -FFmpegRoot 'D:\SDK\ffmpeg-shared'
# 完整桌面包
.\scripts\build\windows.ps1 -FFmpegRoot 'D:\SDK\ffmpeg-shared'
```

脚本遇到任何原生命令失败会停止。成功产物在 `desktop-client\build\windows\x64\runner\Release`。复制**整个 Release 目录**到新的测试目录，双击 framelean.exe；不要只复制 exe。运行组件在 runtime 子目录。

## 2. 测试素材

机器有 Python 3 与测试用 ffmpeg.exe 时，从仓库根目录执行：

```powershell
python scripts/test/prepare_media.py
```

脚本从 Blender 官方下载公开样本并核对 SHA-256，截取 30 秒片段。版权：© 2008 Blender Foundation / www.bigbuckbunny.org；CC BY 3.0。网络下载失败可按脚本中的官方 URL 下载到 `.workspace/p0-media/bbb.zip`，再次运行脚本。

## 3. 界面与后台验收

每项记录“通过 / 失败 / 未测”，附应用构建、系统版本、实际输入和错误信息。

| 编号 | 操作 | 预期结果 | 状态 |
| --- | --- | --- | --- |
| W01 | 在新目录启动完整应用 | 自动连接后台，不依赖开发目录；组件缺失有诊断 | 待测 |
| W02 | 选择 bbb-30s.mp4 | 显示尺寸、时长、H.264 和 AAC 信息 | 待测 |
| W03 | 选择输出目录并压缩 | 进度变化，输出 MP4 可播放，音画同步，源文件不变 | 待测 |
| W04 | 对同一素材连续执行两次 | 新结果增加后缀，不覆盖第一次结果 | 待测 |
| W05 | 输入与输出放到中文、空格路径 | 分析、压缩、打开结果位置可用 | 待测 |
| W06 | 处理较长样本时点击取消 | 3 秒强制终止上限外加清理开销；没有正式残缺文件，可重新处理 | 待测 |
| W07 | 处理过程中关闭窗口再打开 | Worker PID 不变，恢复当前状态，不重复启动编码 | 待测 |
| W08 | 用任务管理器终止 Host（不是 Worker） | 当前任务失败，UI 可用；重新分析、压缩成功 | 待测 |
| W09 | 执行中点击“退出软件”并确认 | 取消处理，后台退出，endpoint.json 消失 | 待测 |
| W10 | 在测试副本中移走 FLL 或某个 libav DLL 后冷启动 | 显示组件诊断，不导致 UI 崩溃；测试后还原副本 | 待测 |
| W11 | 选择损坏文件、非支持编码或旋转视频 | 损坏文件显示失败；可识别特殊媒体显示事实，隐藏不可执行选项 | 待测 |
| W12 | 执行中终止 Worker | Host 随后结束；重启显示中断，可重试；等待项暂停 | 待测 |

关闭窗口后 Worker 继续运行。当前支持重启后状态恢复与发布对账；执行中断从头重试。publishing 阶段取消被拒绝；目录不可访问时保留 reconciling。身份无法确认的临时文件保留并记录清理错误。

## 4. 自动化与日志

安装 ffmpeg/ffprobe 测试工具后，可运行跨平台集成脚本：

```powershell
python scripts/test/v3_integration.py `
  --engine desktop-client/build/windows/x64/runner/Release/runtime/framelean-engine.exe `
  --library desktop-client/build/windows/x64/runner/Release/runtime/framelean_fll.dll `
  --cli desktop-client/build/windows/x64/runner/Release/runtime/framelean.exe `
  --sample .workspace/p0-media/bbb-30s.mp4
```

Windows 没有 SIGSTOP，自动测试只执行普通取消，挂起 Host 的强制回收需手工补验。发生失败时提供终端失败尾部与 `%LOCALAPPDATA%\FrameLean\runtime\logs\host.log` 的相关片段。`endpoint.json` 包含本地凭证，**不要分享该文件或令牌**。

辅助读取 Host PID（仅在本机执行，不对外发送输出）：

```powershell
$endpoint = Get-Content "$env:LOCALAPPDATA\FrameLean\runtime\endpoint.json" | ConvertFrom-Json
$headers = @{ Authorization = "Bearer $($endpoint.token)" }
$snapshot = Invoke-RestMethod "$($endpoint.url)/v3/snapshot" -Headers $headers
$snapshot.active | Select-Object id, host_pid
```

如自动化脚本失败，保留实际失败结果；本轮代码尚未在 Windows 编译或运行。

## 5. CLI 与数据库专项

复制 `outputs/cli-windows` 整目录到独立位置。移除开发 SDK 的 PATH 引用后运行 `framelean.exe doctor` 和 `compress 文件 --output 目录 --wait`，确认依赖来自包内。CLI 和 Flutter 连接同一 Worker。

| 编号 | 验收 | 状态 |
| --- | --- | --- |
| W13 | 独立 CLI、JSON stdout、退出码、关终端后重新观察 | 未测 |
| W14 | 真实 v1 数据库备份与 v2 迁移，旧等待项要求重新规划，历史可查 | 未测 |
| W15 | 执行、校验、发布登记、链接、成功提交节点异常退出并恢复 | 未测 |
| W16 | 输出卷离线保持 reconciling，重连后 task ID --reconcile | 未测 |
| W17 | 临时文件被替换或无法删除：保留成功结果并记录清理错误 | 未测 |
| W18 | NTFS 无覆盖硬链接与文件身份；不支持硬链接时给出诊断 | 未测 |
| W19 | 包内缺依赖、ABI/JSON 不匹配可诊断；旧后台不会被静默替换 | 未测 |
| W20 | 大量历史分页、慢 SSE、大文件核验期间查询响应 | 未测 |

故障注入只在设置独立 FRAMELEAN_RUNTIME_DIR / FRAMELEAN_DATA_DIR 的测试进程使用。macOS 目录同步与 SIGSTOP 证据不能替代 Windows 文件锁、强制结束和断电行为。结果记录为通过、失败或未测，附构建版本和去除凭证的日志尾部。
