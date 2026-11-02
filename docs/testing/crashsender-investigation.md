# CrashSender 弹窗调查

2026-10-31 本机只读调查：

- 项目源码和 target 目录未搜索到 CrashSender 或 CrashRpt 组件。
- 重新启动 Debug 程序，窗口可正常显示，未复现报告的弹窗。
- 实际运行的 cleaner-ui 进程加载了微信输入法的系统组件：
  - `C:\Windows\system32\wetype_tip.dll`
  - `C:\Program Files\Tencent\WeType\WetypeCore_3.0.0.38\x64\wetype_tip_core.dll`
  - 同目录的 `CrashRpt1500.dll`
- 本机发现的报告程序为 `C:\Program Files\Tencent\WeType\3.0.0.38\CrashSender1500.exe`。
- 多个其他应用也加载了 WeType 的 CrashRpt；仅凭模块存在不能证明崩溃原因。
- 本次检查的 Windows Application 事件日志没有检索到 cleaner-ui/Rusten 的对应崩溃事件。

当前结论：提示与 WeType 的 CrashRpt 组件有明确关联线索，但未取得原始弹窗所属进程、异常栈或触发步骤，不能宣布根因已修复。
需要复现时记录触发操作、时间和 Rusten 是否退出，再关联弹窗所属进程及崩溃转储。
本轮未修改输入法、系统配置或屏蔽异常处理。

## 扫描中反复切换主题的复现

用户补充触发操作后，在真实 C 盘扫描中反复切换三种主题，复现进程退出。
标准错误明确报告线程栈溢出；ProcDump 捕获 `C00000FD.STACK_OVERFLOW`。
DbgEng 解析本地 PDB 后定位为 UFFS 的 `TreeTraversal::preprocess` 与
`aggregate_children` 递归，局部变量显示记录 `0x46ba` / `0x46b9` 交替出现。
因此此次已确认的根因是 MFT 目录图环导致无限递归，而非主题保存线程竞争。
WeType CrashSender 提示是错误报告阶段的线索，不能据此把根因归给输入法。

修复在 UFFS 共享统计遍历中增加当前祖先链检测，完成节点退出时清除标记，
保留跨不同路径重复访问硬链接的份额统计。子项链增加边界与最大步数校验。
补丁源码为 `vendor/UltraFastFileSearch/crates/uffs-mft/src/tree_metrics.rs`，保留上游 MPL 头。
定向回归 `crates/platform-windows/tests/uffs_cycle.rs` 验证双节点环终止、普通大小和硬链接份额。

原始转储仅保存在忽略目录 `target/crash-dumps`，未上传；栈解析在 `target/crash-stack.log`。

## 修复验证

- 定向环检测回归通过，原有普通文件与硬链接大小不变。
- 完整验收脚本退出 0，工作区 281 通过、0 失败、7 忽略；严格 Clippy 与格式通过。
- Debug 真实 C 盘扫描中原生切换主题 30 次，通过此前失败的索引阶段，枚举超过 79 万文件后正常取消。
- 强制重建 UFFS 的 Release 缓存后，Release 原生切换主题 30 次，扫描完成 792232 文件、100%、0 警告，没有复现栈溢出。
- 快速点击专项：2004 个样本文件扫描期间连续发出 90 次主题点击，不等待配置保存，扫描完成，随后三主题和重启恢复验证通过（`target/scan-theme-rapid-test.log`）。保存期间已有的禁用保护保留，因此不承诺每一次点击都切换颜色。
- 证据：`target/scan-theme-acceptance.log`、`target/scan-theme-release-build.log`、`target/scan-theme-stress/*.json`。

上述扫描只读，没有执行真实文件清理，也没有修改输入法或系统异常处理设置。
补充的 Release 单测因重新编译完整 Polars 依赖而主动停止，不计作通过；Release 构建及真实完整扫描验证均已完成。
