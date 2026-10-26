# Release 远端 helper 空响应修复

日期：2026-10-26。用户升级后，远端任务显示 `Remote response is incompatible`，无法打开 Herdr 查看窗口。

## 原因与复现

Tailscale 与普通 SSH 均正常。对同一远端主机比较已有 debug helper 和用户安装包对应的 release helper：

| 调用方式 | 返回码 | stdout | 结果 |
| --- | --- | --- | --- |
| 旧 debug helper，经原 PowerShell 单次请求启动 | 0 | 245 字节 | version 1 preflight 正常 |
| 已安装 release GUI helper，经原 PowerShell 单次请求启动 | 0 | 0 字节 | `machine.invalid_response` |
| 同一 release GUI helper，经 cmd 直接继承标准流启动 | 0 | 245 字节 | version 1 preflight 正常 |
| 同一 release GUI helper，经既有持久查询通道启动 | 0 | 359 字节 | version 2 正确校验截图中的绑定目标 |

`main.rs` 在 release 使用 Windows GUI subsystem。PowerShell 的 `& helper.exe` 非管道调用不等待图形程序完成；它可能成功退出，但 helper 尚未输出响应。新版连接会按当前 exe 哈希更新 helper，`remote_install::install` 的末尾 preflight 仍走这一旧入口，导致安装检查失败。前一轮 debug 桌面验收未覆盖这一差异。

`.scratch/remote-incompatible/probe.py` 运行原调用失败，添加 `--direct` 后对同一二进制和请求通过。未修复时，产品回归 `remote_transport::tests::release_gui_helper_returns_preflight_envelope` 在真实 SSH 上以 `machine.invalid_response` 失败；修复后通过。

## 修复与边界

`remote_transport::helper_command` 统一单次 JSON 与持久查询的 helper 启动：仅接受内容哈希文件名，使用 `cmd /d /s /c` 等待进程并原样继承 stdin/stdout。请求、路径和 session 仍放在 JSON 中，不拼入命令。保持原有主机身份、版本、超时及输出大小检查，不添加自动重发。

`search_connection` 复用这一入口。PowerShell 仍用于 bootstrap/安装校验脚本本身；没有替换 Herdr、重建任务绑定或发送 Prompt。

回归测试必须显式提供 `KANBAN_ACCEPTANCE_MACHINE_FILE` 和已部署的 `KANBAN_ACCEPTANCE_HELPER`，并检查 PE subsystem 为 GUI，防止误用 debug console 程序使测试失去覆盖。此测试默认 ignored；本次已显式执行通过。

## 验证

- 常规 Rust：200 项通过，20 项 opt-in ignored。
- 真实 SSH release GUI helper 回归：1 项通过。
- `scripts/build-kanban.ps1 -NoPause -NoOpen` 成功完成生产前端、Rust release 和 NSIS 构建。
- 新 release 程序在隔离 SQLite/WebView2 中连接截图中的任务，真实 IPC 进入 `Herdr open`；本版本 helper 自动更新并通过 preflight，记录哈希与 release exe 一致。未发送任何 Prompt。
- 同一实例的真实远端 Workspace 同步通过，文件查询返回 50 条、技能查询返回 30 条。桌面截图被其他应用遮挡，不作为视觉验收证据；通过依据是实际 release IPC、目标就绪状态与 helper 哈希检查。
- 对新打包并部署的 release helper 再次执行上述回归：1 项通过。
- 已关闭本次隔离验收实例；用户正在运行的安装实例与原数据库未被替换。

修复安装包：`release/kanban/0.1.0/Herdr-Kanban-0.1.0-windows-x64-setup.exe`。SHA-256：`f84ee436398b4be9e100e9014a0e3b5fab9ebcb05fb862158e056e7dd9502b7d`。本次完成 release 产物及桌面功能验证，未执行用户安装目录的覆盖安装。

诊断脚本与日志位于 `.scratch/remote-incompatible/`；使用隔离数据库，业务数据库只读复制。此处不将构建成功等同于安装后的功能验证。
