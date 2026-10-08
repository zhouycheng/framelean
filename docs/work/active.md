# 当前工作

`develop/next` 已建立三项目主线：Flutter `desktop-client/`、Rust Work `fengine/`、Rust 动态媒体引擎 `fll/`。当前实现的最小操作为 Work token 握手、FLL ping 和 WAV 媒体分析。

2026-10-07 已完成 macOS arm64 本机验证：FFmpeg 9.0 静态 libav SDK、FLL Release dylib、Work Release 可执行文件均已生成；架构与动态依赖检查通过，FLL 仅导出 `framelean_fll_get_api`。FLL 的 136 项测试和 Work 的 4 项测试通过。真实 Flutter gateway smoke 验证了引擎 ping、PCM WAV 的 8000 Hz / 单声道分析，以及新 Gateway 接回同一个 Work PID。

本轮修复了 runtime 构建入口遗漏 FLL Release 构建的问题，以及 macOS 接入 socket 继承非阻塞模式导致后续请求失败的问题。

2026-10-08 已完成 macOS arm64 Release App 和 ZIP 打包。包内 gateway smoke、解压后 App 签名完整性、Release App 默认路径连接与 PCM WAV 分析均通过；长分析结果区域已支持滚动并经手动检查。三个项目的关键源码、10 个 FLL crate、品牌资源及包内必需文件已核对，见 `docs/develop/package-checklist.md`。最终包与自动文件清单、SHA-256 位于 `build/packages/`。

打包入口使用本次进程环境限定 Flutter arm64 构建，并在复制 Work/FLL 后重新 ad hoc 签名与校验。Work 并行错误路径测试的临时目录改用现有 UUID 生成器，避免时间戳碰撞。

下一平台验收是 Windows 构建和包内 smoke。Developer ID 签名、公证及 Intel/Universal 2 尚未验收。旧媒体 UI、旧 SQLite 数据和 Alpha 全量执行协议都没有迁入；双平台发布包验收尚未完成。
