# Work ↔ FLL C ABI v1

FLL 以 `framelean_fll` 动态库独立构建：macOS 为 `libframelean_fll.dylib`，Windows 为 `framelean_fll.dll`。Work 通过 `libloading` 在运行时加载；它不依赖 FLL crate，也不静态链接引擎。

## ABI 边界

- 唯一公开符号：`framelean_fll_get_api(requested_major, requested_minor, out_api)`。
- 当前版本：ABI `1.0`；bootstrap 返回版本化 `FrameLeanFllApiV1` 函数表。Work 检查主次版本和 `struct_size` 后才使用函数指针。
- Runtime 由 opaque `void*` handle 表示；Rust 对象、引用、trait object 或 Rust allocator 所有的字符串都不跨边界。
- 调用与结果使用有界 UTF-8 JSON 文档。当前 FLL 文档版本为 `1`，操作由单一 Work 适配点映射。
- FLL 分配的缓冲区只能通过同一 API 表中的 `buffer_free(pointer, exact_length)` 释放；Runtime 必须在卸载动态库前销毁。
- ABI 错误、Runtime 错误和 panic 以状态码返回；Work 对缺库、符号缺失、版本不兼容或初始化失败采取失败关闭。

ABI 的 Rust 事实位于 `fll/crates/framelean-ffi/src/lib.rs`。该文件是实现来源；若增补 API 表字段，只能追加字段并增加 minor version，不能重排既有字段。
