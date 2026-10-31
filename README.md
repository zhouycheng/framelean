# FrameLean

开源的本地媒体处理工具。当前处于 **P2：视频转换与压缩预设**，通过 CLI / Flutter → Rust Worker → 隔离 Host → FLL/libav 完成受控的视频处理。

## 当前范围

- 输入 MP4/MOV/MKV，单 H.264 或 HEVC 8-bit yuv420p SDR 视频轨，无音轨或单 AAC 音轨；输出 MP4/MKV。
- 压缩预设：画质优先 quality（CRF 18）、均衡 balanced（23，默认）、体积优先 small（28）；H.264/libx264 medium，保持尺寸和相对时间线，AAC 复制。
- 转换：H.264 直接转封装，HEVC 使用均衡预设转为 H.264。完整候选来自 FLL，字幕、附件、章节、HDR、旋转、多轨等尚未接通的组合隐藏。
- 进度、取消、独立后台、客户端重连、重名保护和结果校验。
- 处理输出可能大于原文件；支持批量串行队列、重试历史与后台重启对账；目标体积压缩尚未实现。

## CLI 入口

```sh
bash scripts/build/cli-macos.sh
outputs/cli-macos/framelean compress /media/input.mp4 --output /media/output --wait
outputs/cli-macos/framelean compress /media/input.mov --preset quality --format mkv --output /media/output
outputs/cli-macos/framelean convert /media/input.mkv --format mp4 --output /media/output --wait
```

CLI 包独立使用；Worker 自动启动。输出目录需要已存在，关闭观察不取消任务。重启后若队列暂停，可执行 `framelean queue resume`。详见 [CLI 使用说明](docs/develop/cli.md)。

## macOS 构建

需要 Flutter、Xcode、Rust、Python 3、pkg-config，以及带头文件、动态库和 libx264 的 FFmpeg。当前验证环境为 Apple Silicon、Flutter 3.47.5、Rust 1.98.1、FFmpeg 9.0.2。

```sh
bash scripts/build/macos.sh
```

产物：`desktop-client/build/macos/Build/Products/Release/framelean.app`。脚本将依赖复制到包内并做本机 ad-hoc 签名；这是本机技术验证包，未公证、未发布。当前 runtime 按构建机器架构编译。

[Windows 构建与验收](docs/develop/p0-windows-checklist.md) · [接口与架构](docs/develop/p2-runtime.md) · [数据库 ER 图](docs/develop/p1-database.md) · [本轮验证记录](docs/develop/p2-validation.md) · [实施计划](docs/framelean-plan-roadmap.html)

## 目录

- `crates/fll-contracts`：FLL、Host、Worker 共用的纯数据契约。
- `desktop-client`：Flutter 单页界面、交互模型与 Worker 网关。
- `fengine`：Worker HTTP/SSE 服务、任务生命周期及 Host 启动模式。
- `fll`：动态 C ABI 和 libav 媒体操作；公开头文件在 `fll/include`。
- `scripts/build`、`scripts/test`：打包与真实进程测试。

沿用项目归档的 GPL v3 许可证文本。第三方媒体和编解码依赖保留各自许可；公开分发前核对构建配置、对应源码与第三方声明。
