# FrameLean Flutter 客户端

从仓库根目录运行 `bash scripts/build/macos.sh`，获得含 Worker/FLL 的完整应用。单独 `flutter run` 时，需要通过 `FRAMELEAN_ENGINE` 指向已打包的 Worker。

界面通过 `ProcessingModel` 接收 Worker 快照，`EngineGateway` 负责本地认证、HTTP 命令、SSE 和重连。任务状态仅由 Worker 决定。

`flutter test` 运行模型测试；真实进程测试需要设置 `FRAMELEAN_ENGINE`、`FRAMELEAN_RUNTIME_DIR`、`FRAMELEAN_TEST_SAMPLE`，参见根目录验证说明。
