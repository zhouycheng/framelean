# P1 基础队列验证记录（HTTP v2 历史基线）

日期：2026-10-30。本记录对应 HTTP v2、数据库版本 1 的基础队列实现，保留当时的验证证据。当前 HTTP v3、数据库版本 2 的实现与复现步骤见 [本轮验证记录](v3-validation.md)。P0 基线为 a50edd13d599a4913f2215cac6ea37a96e4d1865。

## 当时的验证证据

- Rust 3 项测试通过：数据库创建与重复打开、结构异常诊断、外键和单活动尝试约束、迁移事务回滚；Clippy all-targets 无警告。
- Flutter 静态分析与状态投影测试通过；设置真实素材及隔离目录后，真实 Dart 客户端完成分析、入队、断开重连、文件结果检查和正式退出。
- macOS Release 打包成功，包内 probe 成功；20 个媒体依赖已打包，无开发目录动态库引用。
- 包内进程集成脚本通过 11 组检查，见 [机器结果](p1-macos-results.json)：请求去重和冲突、混合分析、等待项取消与重试历史、重启暂停、真实压缩解码与中文路径重名保护、运行中崩溃、发布成功对账、发布缺失/冲突、Host 崩溃及运行项取消、源变化、输出创建失败、不支持编码后继续处理正常素材。
- 原生 P1 界面已启动检查，确认连接后台、显示暂停状态和队列操作；正式退出成功。完整原生批量交互尚未逐项人工验收，真实客户端流程由 Dart 集成测试覆盖。

发布提交间隙测试直接构造隔离测试数据库与文件状态，用于确定性验证恢复规则；不等同于实机断电测试。输出失败检查使用输出目录在入队后被移除的场景，尚未实测磁盘耗尽。

## 历史复现步骤

以下命令需要配套的 HTTP v2 实现；`p1_integration.py` 保留作历史夹具，当前版本请运行 `v3_integration.py`。原基线从仓库根目录运行：

```sh
bash scripts/build/macos.sh
cargo test --manifest-path fengine/Cargo.toml
cargo clippy --manifest-path fengine/Cargo.toml --all-targets -- -D warnings
python3 scripts/test/p1_integration.py \
  --engine desktop-client/build/macos/Build/Products/Release/framelean.app/Contents/Resources/runtime/framelean-engine \
  --library desktop-client/build/macos/Build/Products/Release/framelean.app/Contents/Resources/runtime/libframelean_fll.dylib \
  --sample .workspace/p0-media/bbb-30s.mp4
```

真实客户端测试从 desktop-client 目录运行：

```sh
FRAMELEAN_ENGINE="$PWD/build/macos/Build/Products/Release/framelean.app/Contents/Resources/runtime/framelean-engine" \
FRAMELEAN_TEST_SAMPLE="$(dirname "$PWD")/.workspace/p0-media/bbb-30s.mp4" \
FRAMELEAN_RUNTIME_DIR="$(mktemp -d)/runtime" flutter test
```

素材来源、授权与哈希沿用 [P0 素材记录](p0-validation.md)。P0 /v1 脚本属于历史版本，本记录对应的接口为 /v2；FLL ABI 仍为 1。

## 当时的验收边界

Windows 构建和实测尚未执行；新版清单在 p0-windows-checklist.md。尚未测试真实断电、磁盘耗尽及大规模历史性能。快照返回全部历史；大量记录时需分页。当前从空库建立版本 1，不读取归档客户端数据库；未来版本迁移需要新增升级测试。当前没有公开发布、公证或完整分发许可审计。
