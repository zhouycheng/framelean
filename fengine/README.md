# FEngine / Work

FEngine 是 Flutter Client 与 FLL 媒体引擎之间唯一的本机 Work 服务。它是独立 Rust 项目，不直接依赖 `fll` crate；启动时从自身可执行文件旁加载 `framelean_fll` 动态库。

当前真实实现提供 loopback 随机端口与 token、长度帧 JSON v1、跨客户端断开重连、FLL 健康检查与媒体分析转发。Work 不创建子 Worker 进程。媒体 Runtime 和执行调度由 FLL 管理。

## 构建与运行

```sh
cargo check --manifest-path fengine/Cargo.toml --locked
cargo run --manifest-path fengine/Cargo.toml -- serve --state-dir /path/to/framelean-engine
```

运行需要同目录的 `libframelean_fll.dylib` / `framelean_fll.dll`；开发时可用 `FRAMELEAN_FLL_LIBRARY` 指定绝对路径。桌面端负责按需以 detached 模式启动 Work，并从 endpoint 文件读取端口与 token。
