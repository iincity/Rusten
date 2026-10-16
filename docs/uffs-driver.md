# UFFS 文件搜索驱动

## 边界

- 上游固定提交：`5892fc895de253a7b578b570b5f9a451732e1cec`；源码位于 `vendor/UltraFastFileSearch`。
- Rusten 只使用 `uffs-mft` 的 MFT 索引能力，不引入 UFFS daemon、client、core 查询、网络更新或 MCP。
- `crates/platform-windows/src/filesystem/uffs_driver.rs` 将 UFFS `MftIndex` 转换为统一 `ScanEvent`，按路径深度补齐目录顺序，并继续执行 Rusten 的路径边界、重解析点、身份和清理前复核。
- `FastFileScanner` 是 UI/CLI 的唯一扫描入口；UFFS 无法打开卷或权限不足时回退 `WindowsFileScanner`。

## 构建策略

UFFS 上游依赖 Polars，依赖树明显大于 Rusten MVP。当前 `uffs` 为默认 feature，主工程直接依赖固定提交的 `uffs-mft`，因此构建要求 nightly。UFFS 无法打开卷、权限不足或索引失败时回退 `WindowsFileScanner`；正式发布前仍需在管理员和普通用户权限下完成性能与安全回归。

UFFS 原始许可证和 `LICENSES/` 文件随源码保留；集成分发遵循 MPL-2.0。
