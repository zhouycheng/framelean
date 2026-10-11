# 当前工作

`develop/next` 已建立三项目主线：Flutter `desktop-client/`、Rust Work `fengine/`、Rust 动态媒体引擎 `fll/`。当前实现的最小操作为 Work token 握手、FLL ping 和 WAV 媒体分析。

2026-10-07 已完成 macOS arm64 本机验证：FFmpeg 9.0 静态 libav SDK、FLL Release dylib、Work Release 可执行文件均已生成；架构与动态依赖检查通过，FLL 仅导出 `framelean_fll_get_api`。FLL 的 136 项测试和 Work 的 4 项测试通过。真实 Flutter gateway smoke 验证了引擎 ping、PCM WAV 的 8000 Hz / 单声道分析，以及新 Gateway 接回同一个 Work PID。

本轮修复了 runtime 构建入口遗漏 FLL Release 构建的问题，以及 macOS 接入 socket 继承非阻塞模式导致后续请求失败的问题。

2026-10-08 已完成 macOS arm64 Release App 和 ZIP 打包。包内 gateway smoke、解压后 App 签名完整性、Release App 默认路径连接与 PCM WAV 分析均通过；长分析结果区域已支持滚动并经手动检查。三个项目的关键源码、10 个 FLL crate、品牌资源及包内必需文件已核对，见 `docs/develop/package-checklist.md`。最终包与自动文件清单、SHA-256 位于 `build/packages/`。

打包入口使用本次进程环境限定 Flutter arm64 构建，并在复制 Work/FLL 后重新 ad hoc 签名与校验。Work 并行错误路径测试的临时目录改用现有 UUID 生成器，避免时间戳碰撞。

2026-10-11，修复分支 `fix/windows-runtime-build`（PR #70）已完成 Windows x64 CI 与 ZIP 内容验收。代码提交 `883c616` 的 [Windows CI](https://github.com/zhouycheng/framelean/actions/runs/38110143981) 和 [macOS CI](https://github.com/zhouycheng/framelean/actions/runs/38110144182) 均通过。Windows 的 137 项 FLL 测试、4 项 Work 测试、Flutter 分析及测试通过；包内 gateway smoke 验证了 FLL 默认加载、PCM WAV 的 8000 Hz / 单声道分析和同一 Work PID 重连。

本轮修复了 MSYS2 Rust 工具路径、稳定 Win32 文件身份查询、静态库链接顺序、预览转换缓冲区对齐和输出刷盘句柄权限；完整 SDK 独立缓存，Windows 包携带 Visual C++ 运行库并检查必需文件。

已下载本次 Windows CI 产物，GitHub 产物摘要、ZIP CRC、38 个条目、核心二进制 x64 架构、DLL 依赖、FLL bootstrap 导出、四个字体与法律材料均核对通过。ZIP、文件清单、SHA-256 和核对报告位于 `build/packages/ci-883c616/`，详见 `docs/develop/package-checklist.md`。

当前 PR 尚未合入 `develop/next`。本轮按已确认范围完成 CI 和包内容验证；因暂无 Windows 设备，解压启动、桌面连接与 WAV 分析的手动验收暂缓。Developer ID 签名、公证及 Intel/Universal 2 尚未验收。当前 UI 仍是引擎诊断入口，下一功能阶段是打通首条用户媒体处理闭环。
