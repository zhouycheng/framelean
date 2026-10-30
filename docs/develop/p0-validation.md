# P0 验证记录

日期：2026-10-30。分支：develop/next。状态：**技术贯通已实现，双平台 P0 验收进行中**。

## 已验证

本机 Apple Silicon / macOS；Flutter 3.47.5、Dart 3.13.4、Rust 1.98.1、FFmpeg 9.0.2。打包脚本生成 Release framelean.app，运行目录包含 Worker、FLL 和 20 个依赖动态库；依赖路径改写为包内相对路径，包内 probe 成功。当前构建为本机 ad-hoc 签名技术包。

- 包内 Worker/FLL 的真实视频集成脚本：17 项通过，见 [机器结果](p0-macos-results.json)。覆盖中文和空格路径、无音轨、损坏/不支持输入、取消、强制停止无响应 Host、Host 崩溃后再次处理、源变化、输出重名、客户端重连、单实例和正式退出。
- 独立 FFmpeg 解码输出成功；逐帧 PTS 与输入差异小于 2 ms，帧数一致，AAC 数据包哈希一致，音视频起始相对偏移差异小于 2 ms。源文件 SHA-256 未改变。
- 故障脚本：缺失 FLL、ABI 不兼容、缺失包内依赖、Worker 异常退出使 Host 结束，4 项通过。
- Flutter 静态分析通过；状态合并与恢复测试通过。真实 Dart 客户端使用包内 Worker 完成分析、执行、断开重连、结果恢复与关闭测试。
- Rust 测试与 Clippy 检查通过。

原生窗口补测：选中 bbb-30s.mp4 后正确显示 320 × 180、30.01 秒、H.264、AAC / 48000 Hz / 2 声道。压缩完成显示源文件 3.26 MB、输出 1.25 MB。再次点击压缩并关闭窗口，重开客户端恢复完成状态，输出自动命名为 bbb-30s-compressed-1.mp4。Cmd-Q 弹出退出确认，选择返回后界面可继续操作；正式退出后 endpoint.json 已删除。

QuickTime 成功打开输出，显示 30 秒时长，启动播放后观察到画面推进。音画同步证据来自前述逐帧时间戳与 AAC 数据校验，尚无人工听音验收。短样本关窗重开验证了结果恢复，长任务关闭后持续进度仍需原生界面专项观察。

## 复现

在仓库根目录执行（FFmpeg/FFprobe CLI 仅用于准备素材和独立测试）：

```sh
python3 scripts/test/prepare_media.py
bash scripts/build/macos.sh
python3 scripts/test/p0_integration.py \
  --engine desktop-client/build/macos/Build/Products/Release/framelean.app/Contents/Resources/runtime/framelean-engine \
  --library desktop-client/build/macos/Build/Products/Release/framelean.app/Contents/Resources/runtime/libframelean_fll.dylib \
  --sample .workspace/p0-media/bbb-30s.mp4 \
  --report .workspace/p0-packaged.json
python3 scripts/test/p0_faults.py \
  --runtime desktop-client/build/macos/Build/Products/Release/framelean.app/Contents/Resources/runtime \
  --sample .workspace/p0-media/bbb-30s.mp4
```

客户端真实进程测试从 desktop-client 目录执行；使用绝对路径指定测试素材、包内 Worker 和临时运行目录：

```sh
FRAMELEAN_ENGINE="$PWD/build/macos/Build/Products/Release/framelean.app/Contents/Resources/runtime/framelean-engine" \
FRAMELEAN_TEST_SAMPLE="$(dirname "$PWD")/.workspace/p0-media/bbb-30s.mp4" \
FRAMELEAN_RUNTIME_DIR="$(mktemp -d)/runtime" \
flutter test test/runtime_test.dart
```

## 素材来源与可追溯性

作品：Big Buck Bunny；署名：© 2008 Blender Foundation / www.bigbuckbunny.org。许可：[CC BY 3.0](https://creativecommons.org/licenses/by/3.0/)；[官方作品与许可说明](https://peach.blender.org/about/)。

[官方 MP4 ZIP](https://download.blender.org/peach/bigbuckbunny_movies/BigBuckBunny_320x180.mp4.zip) 下载后只提取命名的视频文件。截取起点 30 秒、时长 30 秒，视频与音频流复制，用作开发测试。无音轨和损坏样本由测试脚本在临时目录生成。

| 文件 | SHA-256 |
| --- | --- |
| 原始 ZIP | `109e3ede8790bd633f374ca311d9cc61dce8d7f98f5b0797ca98199c9fbceedf` |
| 原始 MP4 | `f78f39603e6774907f2faafabf26a667f4a6fc31769ec304a8a8f7c62d280508` |
| bbb-30s.mp4 | `643c804eee71360f210775b95aaee7d3d0cce613e9249ba0beba7918c9847bb9` |

截取命令（测试用途）：

```sh
ffmpeg -v error -ss 30 -i BigBuckBunny_320x180.mp4 -t 30 \
  -map 0:v:0 -map 0:a:0 -c copy bbb-30s.mp4
```

## 待验证与边界

- Windows 构建和实机测试均未执行，由用户按 [清单](p0-windows-checklist.md) 记录；不能据此标记双平台通过。
- macOS 原生结果恢复、退出和 QuickTime 画面播放已补测；长任务关窗后的持续进度观察与人工听音尚未完成。
- 输出目录需支持硬链接；Worker 异常退出可能遗留隐藏临时文件。持久化恢复属于 P1。
- 未进行公证、发布或第三方依赖完整分发许可审计；当前依赖构建含 GPL/libx264。
