# Rusten

Rusten 是 Windows C 盘空间分析与可恢复清理工具。MVP 默认离线，普通用户进程负责扫描和界面，管理员助手只处理登记的系统操作。

## 架构

- `domain`：路径、文件身份、规则和安全准入；
- `application`：扫描编排、清理计划、执行复核和恢复端口；
- `platform-windows`：NTFS/Windows API、Shell 回收站和管理员客户端；
- `persistence`：配置、操作日志、备份索引和报告文件；
- `apps/cleaner-ui`：GPUI 主界面；
- `apps/cleaner-cli`：离线扫描、规则预览、报告导出和策略导入命令；
- `apps/admin-helper`：按需启动、处理一个请求后退出的提权助手。
- `FastFileScanner`：统一扫描入口；默认使用固定提交的 UFFS MFT 驱动，权限、卷或索引失败时回退现有 Win32 枚举器。

## 验证

```text
cargo test --workspace
cargo check --workspace
```

CLI 默认只读并输出 JSON；`export` 写入报告，`preview` 额外生成规则候选。清理执行仍需经 UI/应用层复核。策略命令会使用内置 Ed25519 信任根验证完整 envelope，`--yes` 不能绕过签名校验：

```text
cargo run -p cleaner-cli -- scan --root C:\\ --max-entries 1000000
cargo run -p cleaner-cli -- preview --root C:\\
cargo run -p cleaner-cli -- export --output rusten-report.json
cargo run -p cleaner-cli -- apply-policy --policy policy.toml
```

真实 NTFS 性能和权限门禁见 `docs/mvp-acceptance.md` 与 `tests/fixtures/README.md`。
