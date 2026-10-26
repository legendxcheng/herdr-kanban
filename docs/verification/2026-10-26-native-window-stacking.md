# 原生 Herdr 窗口层级修复（2026-10-26～27）

> 此轮交付未覆盖“主窗口真正获得系统前台”的场景，用户随后反馈点击后仍被遮挡。当前修正与验收以[2026-10-27 前台切换修复](2026-10-27-native-window-activation.md)为准；本文保留上一轮证据和不足，不代表本问题已完整解决。

## 现场与诊断边界

用户报告运行中的安装版 Kanban 右侧 Herdr 偶尔消失。只读 Win32 采样确认：Kanban HWND 338000、查看窗口 HWND 41689562，后者 `IsWindowVisible=true`、矩形 `(3207,76)-(3834,1028)`，但在主窗口下方；连续 50 次区域命中检查均返回 Kanban 主窗口。排序参照交替落在隐藏的 IME、Sogou_TSF_UI 窗口，其中包含终端自身的附属窗口。

旧实现把 `GetWindow(main, GW_HWNDPREV)` 直接作为 `SetWindowPos` 的参照。可见顺序已经正确时，重新插入隐藏附属窗口会触发多余排序；新增真实 Win32 回归在旧实现下因此失败（排序计数从 2 增至 3），修复后通过。普通原生窗口及独立 WezTerm 的简化场景没有稳定复现最初的长期遮挡，不能据此把输入法宣布为唯一根因。后续观察中原安装实例已更换查看窗口并恢复显示，该恢复不是本修复已部署的证据。

## 实现

- `native_terminal_windows::WindowSync` 使用 EnumWindows 读取当前顺序，忽略隐藏辅助窗口和其他隐藏查看窗口；保留目标查看窗口自身，跳过其 owner 链下的附属窗口，避免把终端插在自身 popup 后面。主窗口的可见对话框仍作为遮挡参照。
- 按实际可见层级判断是否需要调整。终端或其附属窗口获得前台时，主窗口跟随到终端下方；其他情况把终端放到主窗口上方的适当位置。
- 仍使用 `SWP_NOACTIVATE`、普通 Z 序、无 owner/SetParent/TOPMOST。增加 `SWP_NOOWNERZORDER`，检查 SetWindowPos 返回值并限频记录错误。
- 每个查看窗口保留独立重试状态。异步请求后由后续监视循环重新观测，最多连续 3 次、间隔至少 50ms，然后冷却 1 秒；恢复正常、隐藏或前台改变会重置重试。错误/未收敛日志最多每 5 秒一条。移动与尺寸更新不受层级冷却阻塞。
- 无数据库迁移、任务身份或 Herdr 输入协议变更。用户已有未提交启动、远端连接等改动保留。

## 已执行验证

- `cargo test --offline --locked --manifest-path src-tauri/Cargo.toml --lib`：213 项通过、22 项 opt-in ignored。
- `npm test -- --maxWorkers=2`：229 项通过。
- 单独执行 `native_terminal::windows::tests` 的两个 ignored 原生测试：2 项通过。真实桌面线程覆盖隐藏及嵌套附属窗口、稳定顺序不重复排序、可见宿主对话框、外部窗口遮挡、显隐、最小化恢复、不抢焦点、无 owner/TOPMOST；另启动真实独立 WezTerm，10 轮强制压到宿主后均恢复。测试拥有并回收自己的窗口/进程。
- `scripts/build-kanban.ps1 -NoPause -NoOpen`：TypeScript、生产前端、Rust release 和 NSIS 打包通过。沿用已有 chunk 大小及未使用代码警告。

上述全量检查和安装包基于当前共享工作区，并非只含本次修复的隔离提交快照。

当轮安装包路径（后续构建会覆盖）：`release/kanban/0.1.0/Herdr-Kanban-0.1.0-windows-x64-setup.exe`。

SHA-256：`cba6053d307d6e6dab0f5af0522c003ea8b89b389c0f066aa86cb2dec0c8d13f`。

## 实际 release Kanban + Herdr 验收

使用本次 release exe、独立 SQLite/WebView2 数据目录及专用 `kanban-acceptance-stacking-*` Herdr session。两个 Codex TUI 仅作为空闲终端夹具，没有提交 Prompt 或模型任务。通过真实页面点击及 Win32 检查验证：

- 两个任务/Workspace 往返切换，真实查看窗口始终排在主窗口上方，且无 TOPMOST。
- 12 次主动将查看窗口压到主窗口后，等待 250ms 后均已恢复；同步没有改变当时的前台窗口。
- 插入隐藏 owned 辅助窗口后，以及再次压低查看窗口后，都恢复正确顺序。
- 实际新建任务弹窗打开时隐藏终端，取消后恢复；终端最大化/恢复、应用最小化/恢复通过。
- 创建独立外部窗口覆盖主窗口和终端，确认同步不会把终端提到该窗口上方。

最后的屏幕区域命中来自另一个用户应用，而非 Kanban 主窗口；截图也存在该外部遮挡，不计入完整视觉验收。原生层级检查与真实应用交互结果已单独记录。应用日志有一条未收敛冷却记录，后续检查通过；不据此承诺所有瞬时帧都无差异。

测试脚本及结果：`.scratch/native-stacking/acceptance.py`、`results.json`。最初沙箱运行未建立可用 WebView2 页面，最终实际应用验收在获准的沙箱外执行；测试 shell 中 Codex 入口未被 PATH 解析，夹具改为明确可执行文件路径启动。以上均为测试夹具调整，没有更改用户 shell/Herdr 配置。最终测试应用和专用 Herdr server 已关闭，用户原安装实例 PID 33304 保留。本次没有向原任务输入任何内容。

构建快照中的 `native_terminal.rs`、`native_terminal_windows.rs` 与最终源码 SHA-256 一致，安装包校验和已重新核对。真实 SSH 连接、NSIS 覆盖安装不在本次验收范围。

## 验证限制

未覆盖所有物理输入法候选操作、不同 DPI 的跨屏拖动、虚拟桌面和所有第三方置顶工具。当前安装实例未覆盖安装或重启；源码修复和新安装包不代表用户当前进程已经更新。独立跨进程窗口仍允许短暂同步帧差，不能承诺任意系统负载下零遮挡。
