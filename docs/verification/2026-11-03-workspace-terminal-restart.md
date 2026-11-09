# Workspace 普通终端重启恢复验证

日期：2026-11-03。用户报告 xc 重启电脑后，game-h5 的 OpenTerminal 显示 `Target identity changed; repair the binding`，Reconnect 无效，并明确授权按核实原标记后恢复绑定的方向修复。

## 故障证据与失败回归

- 通过 `view_image` 读取用户本地原图，确认这是 Workspace 普通终端，错误区域只有重连入口。
- xc 的 Herdr 日志在北京时间 2026-11-03 07:35:57 记录恢复 15 个 Workspace。只读核对 game-h5 保存记录与实时库存：Workspace `w9`、Tab `w9:tF`、pane `w9:p27` 相同，terminal 从 `term_65ef933dbc0c6fc` 变为 `term_65f539b8a4e131e`；原专属 Tab 创建标记仍在。
- 未改源码前，用真实库存调用原始 `shell_live`，完整和单 pane 的最小数据都得到 `Err("binding.identity_changed")`；仅在诊断数据副本更新 terminal ID 即通过。
- 新增原始 CLI 适配器与 SQLite 的 `restored_workspace_shell_reconnects_and_persists_without_creating_or_sending` 回归，修复前显式运行失败于 `binding.identity_changed`。前端新增 Workspace 身份异常回归，修复前显示通用“请修复绑定”而失败。

```powershell
cargo test --offline --locked --manifest-path src-tauri/Cargo.toml --lib restored_workspace_shell_reconnects_and_persists_without_creating_or_sending -- --nocapture
npm test -- src/TerminalPanel.test.tsx
```

## 最终实现

- 保留普通终端和任务终端的原严格身份检查；只在普通终端旧绑定不匹配时，按原操作 UUID 的专属创建标记核实恢复。
- 标记在全 session 唯一、同一电脑/session/Workspace/Tab/pane、旧 terminal 全局不存在、新 terminal/pane 唯一且无占用冲突时，事务更新保存的 terminal 身份。
- 更新以原创建操作、原目标 JSON 和 ready 状态为条件，保留初始 cwd。取消、过时记录、任务/隔离记录/其他普通终端占用均阻止写回。
- 旧目标和创建标记都确认不存在才允许替代创建。标记移动、重复、Tab 中多 pane、旧 terminal 仍在其他位置、库存不可读等情况保留拦截；未知创建不重放。
- 普通终端错误使用专用中英文说明和“核实并重连 / Verify and reconnect”。沿用现有 `workspace_terminal_open`、helper `cli_output` 及原生查看器；无数据库迁移或远端协议变化。

## 自动验证

- `cargo test --offline --locked --manifest-path src-tauri/Cargo.toml --lib workspace_terminal -- --test-threads=4`：终端相关 15 项通过；原始 CLI 替身仅实现只读清单命令，创建或输入命令会令测试失败。
- 新回归覆盖原位置 terminal 更新及重复打开不再创建、标记缺失/重复、Tab/pane/Workspace 移动、旧 terminal 移到其他位置、候选 terminal 重复、多 pane、任务/隔离/其他 Workspace 占用、取消、原创建操作已变、库存不可读及 Workspace 默认目录变化。
- `npm test -- src/TerminalPanel.test.tsx src/board-tests/workspace-terminal.test.tsx src/board-tests/terminal-layout.test.tsx`：16 项通过，包含普通终端专用错误、“核实并重连”及只按 Workspace Project ID 路由。
- `npm test -- --maxWorkers=4`：42 文件、318 项通过。
- `npm run build`：TypeScript 与 Vite 生产构建通过，保留原有大 chunk 提示。
- `cargo test --offline --locked --manifest-path src-tauri/Cargo.toml --lib -- --test-threads=1`：334 通过、1 失败、35 ignored，用时 136.16 秒。唯一失败为未修改的 `title_deepseek::tests::http_failures_keep_summary_and_only_transient_errors_schedule_retry`：HTTP 错误模拟返回 `title.network`，而断言期望具体 HTTP 错误码。单独显式复查仍失败；`title_deepseek.rs` 与 `title_deepseek_tests.rs` 相对 Git 没有改动。未将全量 Rust 标记为通过，也未在本次普通终端修复中改动标题模块。
- 本次三个完整 Rust 文件的 `rustfmt --check --edition 2024 --config skip_children=true` 及 `git -c core.whitespace=cr-at-eol diff --check` 通过。未重排共享工作区其他既有改动。

## 真实本机与 SSH 只读核验

显式运行 `live_workspace_terminal_restoration_read_only`。该验收适配器只委托真实 CLI/helper 的只读查询，不实现创建或发送输入能力；持久化在内存 SQLite 中完成，不更新用户数据库、关闭 shell、重启 Herdr 或发送命令。

1. 本机 xc：使用上述 game-h5 的真实重启前保存记录。实际严格检查先失败，再调用完整 Workflow 恢复、核对保存结果并重复打开，全部通过（0.34 秒）。
2. Windows SSH devpc：使用真实远端专属 Tab 与 pane 清单，在隔离数据库中模拟旧 terminal ID。通过实际机器配置、身份校验及 SSH helper 控制连接执行恢复、保存和重复打开，全部通过（1.97 秒）。这验证真实 SSH 查询链路与本次恢复逻辑；没有重启远端服务。

本次输入快照保存在本机 `.scratch/terminal-restore/real-local.json` 与 `real-ssh.json`，不作为通用测试数据提交。可复查的显式入口：

```powershell
$env:KANBAN_SHELL_RESTORE_FIXTURE='G:/herdr-kanban/.scratch/terminal-restore/real-local.json'
cargo test --offline --locked --manifest-path src-tauri/Cargo.toml --lib live_workspace_terminal_restoration_read_only -- --ignored --nocapture
# 将输入文件改为 real-ssh.json 可复查真实 SSH 链路。
```

## 安装包交付（2026-11-03 10:16）

- 通过正式入口 `build_kanban.bat -NoPause -NoOpen`，隔离源码快照、锁定前端依赖恢复、TypeScript/Vite、Rust Release 及 NSIS 构建成功。保留既有大 chunk 和未使用代码提示。
- 采用当前工作区快照，包含既有未提交改动；`workspace_terminal.rs`、`herdr_workspace_terminal.rs`、新增 CLI 回归、`herdr.rs`、`workflow.rs`、`TerminalPanel.tsx` 及两份语言文件共 8 项与打包副本的 SHA-256 一致，生成的前端产物包含普通终端恢复提示。
- 安装包：`release/kanban/0.1.0/Herdr-Kanban-0.1.0-windows-x64-setup.exe`，4,948,819 字节。
- SHA-256：`6e51998d6a1eca55bb0b5a8a01e83084209a6a8a80fdff8ffdb8a30e4a266f7e`。交付文件、原始 NSIS 产物与同目录 `SHA256SUMS.txt` 三方一致。
- 构建日志：`release/kanban/logs/build-20261103-101113-22444.log`。

## 验收边界

- 已生成新安装包；未替换或安装用户当前桌面版本。
- 尚未用新版本完成实际电脑/Herdr 服务重启后的 Tauri、WezTerm 原生窗口完整交互。真实只读 CLI/helper 验证和模拟前端通过不代表该桌面验收通过。
- 恢复只关联 Herdr 已重建的 shell；不能恢复重启前已结束的命令、进程或环境。外部操作与最终核验之间仍可能发生竞态，原生查看器继续做严格身份检查，失败后可重新核实。
