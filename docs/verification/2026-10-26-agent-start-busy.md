# Agent 启动窗格占用时序修复验收

日期：2026-10-26。

## 原始问题与证据

创建卡片已成功，自动启动复用 idle Codex pane，退出旧会话后立即调用 `agent start`。Windows 进程查询与 Herdr Agent 注册状态更新存在短暂差异，启动返回 `agent_pane_busy`。原实现将准备阶段一切错误记为 unknown，核实又要求 pane 消失，导致明确拒绝启动也无法直接重试。

用户报告的本机 default / game-h5 / `w9:pS`：Herdr 日志于 `2026-10-26T04:11:25.408Z` 拒绝启动，`04:11:25.518Z` 才记录旧 Codex 变为 shell。只读数据库确认生命周期为 unknown、绑定为 reserved、本任务没有 dispatch 操作。任务正文仍保留。

## 实现

- `herdr_cli.rs` 严格识别完整 agent.start / agent_pane_busy JSON 错误，保留 stdout 错误信封，兼容旧 stderr 错误记录。
- `herdr_codex.rs` 仅在明确拒绝后重查同一窗格并重试；150ms 间隔、10 秒重试窗口，单次 CLI 调用仍受原有超时控制。核对 terminal、shell PID 与就绪 shell 身份，允许退出过程中旧 Codex 注册状态短暂滞后。
- `lifecycle.rs` 将持续明确拒绝转为 failed 并释放本次 reserved 绑定。事务复核操作与绑定快照、无本次派发及其他未决派发；保留 pane、正文及已有恢复会话。旧 unknown 卡片通过原有核实入口走同一逻辑。
- 其他错误保持 unknown；任何一次启动返回成功后均不重复启动，任何派发不确定结果均不自动重发。

## 自动化验证

先加入回归，运行 `cargo test --manifest-path src-tauri/Cargo.toml --offline --locked --lib pane_busy -- --nocapture`：3 项失败分别复现不重试、失败仍为 unknown、旧卡片核实要求关闭 pane；未知/替换目标保护通过。

修复后的同一组：8 passed、1 ignored。覆盖实际 CLI 进程替身返回 stderr/stdout 明确拒绝后成功、持续拒绝有界结束、首次/重试后连接失败不重复、terminal/shell 替换不重试、非完整错误不误判；真实 SQLite 工作流覆盖释放预占后只发送一次、旧 unknown 恢复、恢复会话保留及有派发记录不能释放。

完整后端：

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --offline --locked --lib
```

181 passed、0 failed、19 ignored。保留既有 `terminal.rs` 未使用字段/方法警告；ignored 为显式 opt-in 验收，不属于已通过常规测试。

相关前端：

```powershell
npm.cmd test -- src/board-tests/background_start.test.tsx src/board-tests/dispatch.test.tsx
```

2 文件 / 29 测试通过，属于模拟 Tauri IPC 的组件回归。

## 真实 Herdr / Codex

使用独立 `kanban-acceptance-pane-busy-20261026` workspace（w22），目录为本仓库，所有创建均 `--no-focus`。已有 Herdr server 需要管理员连接权限；以相应权限运行测试，无服务重启或配置修改。

```powershell
$env:KANBAN_LIVE_BUSY_WORKSPACE='kanban-acceptance-pane-busy-20261026'
$env:KANBAN_LIVE_CWD='G:\herdr-kanban'
$env:KANBAN_HERDR_SESSION='default'
cargo test --manifest-path src-tauri/Cargo.toml --offline --locked --lib live_pane_busy_reuses_idle_codex_without_sending_prompt -- --ignored --nocapture
```

1 passed，耗时 24.01 秒。同一个 `w22:p2` 完成 4 次启动，terminal 身份由测试连续相等断言核验，shell PID 始终 47964，Codex PID 依次为 59552、13224、66288、23112。后三次都在旧 Codex 退出后收到原始 `agent_pane_busy`，随后自动重试成功；这不是仅模拟相似故障。

其中一轮服务日志：

- `04:21:20.404Z`：agent.start 明确拒绝。
- `04:21:20.516Z`：旧 Codex 注册状态清除，回到 shell。
- `04:21:20.628Z`：重试 agent.start。
- `04:21:20.649Z`：启动请求被接受，后续真实 Codex 就绪检查通过。

测试不发送业务提示词，只执行准备流程所需的 `/quit`。结束时测试关闭自己的 pane 并确认不存在，随后关闭本次创建的 w22 workspace，未操作用户工作 pane。

## 边界与使用

未执行本次原生 WebView2 点击/拖放、远端 SSH 故障注入或安装包验收；未重新打包或替换运行中的应用，也未修改用户实际数据库。应用更新到本次代码后，旧错误卡片点击“核实启动结果”可恢复为“重试启动”，随后由用户明确重试；核实动作本身不发送正文。
