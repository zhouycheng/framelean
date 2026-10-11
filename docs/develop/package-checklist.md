# 三项目文件与打包验收清单

## 本次状态

2026-10-08，`develop/next` 在本机 macOS arm64 完成三个项目的 Release 构建和组合打包。最终 ZIP 包含 Flutter App、Work 可执行文件和 FLL 动态库。三个项目保持独立构建入口，最终交付为一个桌面 App。

本清单按“源码关键文件 → 构建产物 → 最终包内容 → 实际运行”核对。文件存在只表示结构完整；构建、加载和 smoke 的结果分别记录。

## 1. 源码关键文件

以下路径相对于仓库根目录。本次已检查这些文件存在且非空。

| 项目 | 必需文件或目录 | 本次结果 |
| --- | --- | --- |
| Flutter 前端 | `desktop-client/pubspec.yaml`、`pubspec.lock` | 存在 |
| Flutter 入口与网关 | `desktop-client/lib/main.dart`、`lib/engine_gateway.dart` | 存在 |
| Flutter smoke | `desktop-client/test/engine_gateway_smoke_test.dart` | 存在 |
| 平台工程 | `desktop-client/macos/Runner.xcodeproj/project.pbxproj`、`desktop-client/windows/CMakeLists.txt` | 存在；Windows x64 CI 构建通过（PR #70） |
| 品牌资源 | `desktop-client/assets/app_icon/`、`assets/fonts/alibaba_puhuiti/` | 5 个 PNG、4 个非空字体文件存在 |
| Work 构建入口 | `fengine/Cargo.toml`、`Cargo.lock` | 存在 |
| Work 服务与动态适配 | `fengine/src/main.rs`、`src/fll.rs` | 存在 |
| Work 错误路径测试 | `fengine/tests/startup_failure.rs` | 存在；缺库与 ABI 不匹配测试通过 |
| FLL workspace | `fll/Cargo.toml`、`Cargo.lock` | 存在 |
| FLL 内部模块 | `fll/crates/framelean-{core,media,analysis,environment,decision,ffmpeg,ffi,pipeline,plugin,runtime}/` | 10 个 crate 的 `Cargo.toml` 与 `src/lib.rs` 均存在 |
| FLL ABI 构建配置 | `fll/crates/framelean-ffi/build.rs` | 存在 |
| 接口契约 | `protocol/v1/README.md`、`protocol/fll-abi/v1/README.md` | 存在 |
| macOS 构建入口 | `scripts/build/build_runtime_macos.sh`、`package_macos.sh` | 存在，实际执行成功 |
| 法律材料 | `LICENSE`、`legal/NOTICE.md`、`legal/THIRD_PARTY_NOTICES.md` | 存在 |

## 2. 构建产物

| 项目 | 本机产物 | 本次结果 |
| --- | --- | --- |
| Flutter | `desktop-client/build/macos/Build/Products/Release/FrameLean.app` | Release 构建成功；Runner 为 arm64 |
| Work | `build/runtime/macos-arm64/framelean-engine` | Release 构建成功；arm64 可执行文件 |
| FLL | `build/runtime/macos-arm64/libframelean_fll.dylib` | Release 构建成功；arm64 动态库 |
| 静态 libav SDK | `build/dependencies/ffmpeg/macos-arm64/` | 本机构建成功；引擎不依赖外部动态 libav |

FLL 的 136 项测试、Work 的 4 项测试和 `flutter analyze` 通过。无引擎路径参数的普通 Flutter 测试会跳过 gateway smoke；打包脚本随后显式传入包内 Work 路径，真实 smoke 通过，不能将前面的跳过计为通过。

## 3. 最终 App 与 ZIP

ZIP：`build/packages/FrameLean-macos-arm64.zip`。

下列路径相对于 ZIP 内的 `FrameLean.app/`，本次已核对存在且非空：

| 所属项目或材料 | 包内路径 | 本次结果 |
| --- | --- | --- |
| Flutter Runner | `Contents/MacOS/FrameLean` | 存在、可执行、arm64 |
| Flutter AOT 程序 | `Contents/Frameworks/App.framework/Versions/A/App` | 存在 |
| Flutter runtime | `Contents/Frameworks/FlutterMacOS.framework/Versions/A/FlutterMacOS` | 存在 |
| 字体声明与字体 | `Contents/Frameworks/App.framework/Versions/A/Resources/flutter_assets/FontManifest.json` 与 `assets/fonts/alibaba_puhuiti/` | 声明与 4 个字体存在 |
| 应用图标 | `Contents/Resources/Assets.car` | 存在 |
| Work | `Contents/Resources/framelean-engine` | 存在、可执行、arm64 |
| FLL | `Contents/Resources/libframelean_fll.dylib` | 存在、arm64；只导出 `framelean_fll_get_api` |
| 应用身份 | `Contents/Info.plist` | `com.justin.framelean`；沿用版本字段 `1.2.1` / build `8`，未定义新发布版本 |
| 法律材料 | `Contents/Resources/legal/LICENSE`、`NOTICE.md`、`THIRD_PARTY_NOTICES.md` | 存在；法律材料目录随包复制 |
| App 完整性 | `Contents/_CodeSignature/CodeResources` | runtime 复制后重新 ad hoc 签名，完整性校验通过 |

每次 macOS 打包自动生成两个旁置文件：

- `FrameLean-macos-arm64.zip.files.txt`：ZIP 中的完整条目清单，包含目录、符号链接及 macOS 元数据。
- `FrameLean-macos-arm64.zip.sha256`：该次 ZIP 的 SHA-256，用于核对包是否发生变化。

构建产物、ZIP、清单和哈希均在 Git 忽略目录中。源码、SDK、旧数据库、测试 fixture 和 endpoint/token 不作为应用资源打入 ZIP。

## 4. 实际运行验收

| 验收 | 本次结果 |
| --- | --- |
| ZIP 压缩数据完整性 | `unzip -tq` 通过 |
| 解压后 App 签名完整性 | `codesign --verify --deep --strict` 通过 |
| 包内 Work/FLL 默认加载 | 清除 FLL 路径覆盖，Flutter gateway smoke 通过 |
| 引擎 ABI | ping 返回 ABI v1.0；FLL bootstrap 导出存在 |
| WAV 分析 | 真实 libav 返回 `pcm_s16le`、8000 Hz、单声道 |
| 新客户端重连 | gateway smoke 确认连接到同一个 Work PID |
| Release App 手动操作 | 从最终 ZIP 解压启动，无 Work/FLL 路径覆盖；连接和 WAV 分析通过 |

复查时从仓库根目录运行：

```sh
scripts/build/package_macos.sh
unzip -tq build/packages/FrameLean-macos-arm64.zip
shasum -a 256 -c build/packages/FrameLean-macos-arm64.zip.sha256
```

每次新增运行依赖时，先更新包内必需文件和实际加载检查，再生成新的清单与哈希。手动 App 验收须从当次 ZIP 解压启动，避免误用旧 Debug App 或开发环境覆盖路径。

## 5. Windows x64 CI 包

2026-10-11，PR #70 代码提交 `883c616` 的 [Windows CI](https://github.com/zhouycheng/framelean/actions/runs/38110143981) 通过 Rust 测试、Flutter 分析/测试、Release 构建及包内 gateway smoke。Windows FLL 的 137 项测试和 Work 的 4 项测试均通过，包括文件身份、真实媒体转换与输出防覆盖用例。

下载并核对的 ZIP 位于 `build/packages/ci-883c616/FrameLean-windows-x64.zip`，包含 38 个条目。GitHub artifact 的 SHA-256 摘要匹配，内部 ZIP 的 CRC 通过；ZIP 本身的 SHA-256 为 `6d543e8830d2a1030ea123a48455d88bc282aa11ab1dcb6c5669f942fa970bd0`。

| 检查对象 | 本次结果 |
| --- | --- |
| `FrameLean.exe`、`framelean-engine.exe`、`framelean_fll.dll`、`flutter_windows.dll` | 存在、非空、PE x64 |
| FLL ABI 导出 | 仅 `framelean_fll_get_api` |
| Work/FLL DLL 导入 | 无外部动态 libav、libgcc、libstdc++ 或 libwinpthread |
| Visual C++ 运行库 | CMake 自动安装；Runner 所需 `msvcp140.dll`、`vcruntime140.dll`、`vcruntime140_1.dll` 均随包携带 |
| Flutter 数据 | `data/app.so`、`data/icudtl.dat`、资产清单存在且非空 |
| 字体 | 四个 Alibaba PuHuiTi TTF 存在且非空，与 FontManifest 声明一致 |
| 法律材料 | LICENSE、COPYING、NOTICE、THIRD_PARTY_NOTICES、SOURCE_OFFER 均存在且非空 |
| 包内 smoke | 默认加载 FLL，PCM WAV 8000 Hz / 单声道分析及同一 Work PID 重连通过 |

旁置 `.zip.files.txt`、`.zip.sha256` 和 `FrameLean-windows-x64.audit.json` 保存完整文件清单、包哈希和核对报告。下载、报告和二进制均位于 Git 忽略目录中。本轮验收范围为 CI 和包内容；Windows 桌面手动验收待有设备后完成。

## 验收范围

macOS arm64 开发验证包使用 ad hoc 签名。Developer ID 签名、公证和 Intel/Universal 2 尚未验收。Windows x64 CI 与包内容验收已通过，桌面手动验收暂缓。当前 UI 为引擎诊断入口，未完成产品媒体处理工作流。
