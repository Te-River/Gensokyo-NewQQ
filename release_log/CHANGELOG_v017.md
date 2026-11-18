# Changelog — Release017

> 自 Release016（Release 016 Rust）以来的所有变更。

---

## 🐛 修复

### `cargo xtask build --all` 缺工具链目标改为 `[skip]`（2026-11-18）

Windows-gnu 宿主上，矩阵中的 macOS / MSVC / Android 目标此前会先 `rustup target add`（下载 rust-std）再硬编失败（如 ring 的 C 代码被 mingw gcc 拒绝 `-arch`），并把失败计入整体结果（exit -1）：

- **修改**：构建循环前增加工具链可用性闸门（纯函数 `target_skip_reason`，`xtask/src/build.rs`）：宿主非 macOS 时跳过 darwin 目标、找不到 `link.exe` 时跳过 MSVC 目标、无 NDK 时跳过 Android；linux 目标继续走 zig 自动引导（`.tools/` 缓存命中即零下载）
- **影响**：`--all` 汇总输出 `N 个产物 / M 个跳过 / 0 个失败`，缺工具链目标不再触发 rustup 组件下载；显式 `--target <triple>` 不受闸门影响（仍会真实尝试）
- **测试**：xtask 32 条单测全绿（含两条新增闸门判定用例）

---
