# FrameLean 开发工作流

## 核心规则

- `develop/next` 是长期集成线；旧 `main` 与 `develop/v1.3.0-alpha` 冻结归档。
- 代码位于三个独立项目：Flutter `desktop-client/`、Rust Work `fengine/`、Rust 动态引擎 `fll/`。协议契约位于 `protocol/`。
- 改动按当前事实验证。Alpha 功能、测试或发布文档不能替代 `develop/next` 的实现和验收证据。
- 保留用户已有工作；不重置、覆盖、暂存、提交、推送、打 tag 或发布，除非用户明确要求对应 Git 外部动作。

## 分支和工作区

日常改动从最新 `develop/next` 创建短生命周期的 `feature/*`、`fix/*`、`chore/*`、`docs/*`、`release/*` 或 `hotfix/*` 分支，完成后经 PR / MR 合回集成线。不要在冻结归档线开发。若当前目录有其他未完成工作，优先使用 `worktrees/<task-name>` 隔离。

## 依赖方向

```text
desktop-client → protocol/v1 → fengine → protocol/fll-abi/v1 → fll
```

- Flutter 仅通过 `EngineGateway` 调 Work，不启动 FFmpeg/FFprobe CLI，也不直接加载 FLL。
- Work 不静态链接 FLL crate；运行时动态加载同目录 `framelean_fll`。
- FLL 使用进程内 libav。应用 UI、设置和应用本地数据属于 Flutter；媒体运行时和处理属于 FLL。

## 本地验证

Flutter：

```sh
cd desktop-client
flutter pub get
dart format --output=none --set-exit-if-changed lib test
flutter analyze
flutter test
```

Rust Work 的 framing 和错误边界可独立检查：

```sh
cargo check --manifest-path fengine/Cargo.toml --all-targets --locked
cargo test --manifest-path fengine/Cargo.toml --locked
```

FLL 使用各目标平台的静态 libav SDK。macOS 和 Windows runtime 构建脚本会准备/检查依赖、构建 FLL 与 Work、核对架构和动态 libav 依赖：

macOS 构建依赖包括 Xcode/Clang、CMake、Make、NASM 和 `pkg-config`。Homebrew 中后两者的 formula 名为 `nasm` 和 `pkgconf`；按工作站供给流程安装并刷新本机清单。

```sh
scripts/build/build_runtime_macos.sh
# Windows：在 MSYS2 MINGW64 shell
bash scripts/build/build_runtime_windows.sh
```

三项目真实闭环由 `desktop-client/test/engine_gateway_smoke_test.dart` 验证；需要并排构建的 Work 与 FLL，通过 `FRAMELEAN_FENGINE_BINARY` 提供 Work 路径。该测试启动 detached Work、请求 ping、分析临时 WAV、断开并重连，再执行测试关闭。

macOS runtime 构建后，在仓库根目录执行真实 smoke：

```sh
RUNTIME="$PWD/build/runtime/macos-$(uname -m | sed 's/x86_64/x64/')"
cd desktop-client
env -u FRAMELEAN_FLL_LIBRARY FRAMELEAN_FENGINE_BINARY="$RUNTIME/framelean-engine" \
  flutter test test/engine_gateway_smoke_test.dart --reporter expanded
```

smoke 清除开发用 FLL 路径覆盖，确保 Work 从自身同目录加载真实动态库；它校验 WAV 的 PCM 编码、采样率、声道数和重连前后的 Work PID。

平台包入口：

```sh
scripts/build/package_macos.sh
# Windows：先从 MSYS2 MINGW64 构建 Rust runtime，再运行：
./scripts/build/package_windows.ps1
```

打包必须确认 Runner 可执行文件、Work 和 FLL 为目标架构；包内存在法律材料；Work 默认从自身同目录加载 FLL；最终包内 smoke 成功。缺少动态库、ABI 不匹配或运行时初始化失败必须报告错误且停止启动。

macOS arm64 打包仅在本次构建进程设置 `FLUTTER_MACOS_ARM64_ONLY=true`，不修改全局 Flutter 配置。runtime 复制后重新 ad hoc 签名并通过 `codesign --verify --deep --strict`。ZIP 通过压缩数据检查，旁置 `.zip.files.txt` 完整条目清单和 `.zip.sha256` 哈希。源码和包内文件核对见 `docs/develop/package-checklist.md`；Release App 手动验收从最终 ZIP 解压启动。

## 文档与发布

架构、数据、协议、打包、用户行为和可复用工程事实变化时更新 `docs/`、`CONTEXT.md` 与相关项目 README。正式版本再更新根目录 `CHANGELOG.md`；日常改动不写 `No Release` 流水。

当前没有已验收的公开 Release 流程。构建包不等于已确定版本、签名、公证、更新协议或公开发布。用户明确授权前不创建 tag、不发布、不推送。
