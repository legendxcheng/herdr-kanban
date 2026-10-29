# Codex 启动更新提示自动 Skip

日期：2026-10-30。

## 问题与修正

本机任务“炼器失败后提升成功率列”在启动阶段遇到 Codex `Update available · 0.165.0 → 0.166.0`，Herdr 检测规则为 `startup_update`，返回 `cli:agent:start / agent_not_ready`。现场数据库无派发记录、`dispatched_at` 为空，生命周期保留 unknown。用户授权遇到更新提示自动选择 Skip。

`herdr_cli.rs` 精确解析启动错误信封，将 `agent_not_ready` 与可重试的 `agent_pane_busy` 区分。前者进入原进程的就绪检查，不重复启动，也不释放未知启动的预占。

`herdr_codex.rs` 仅对本次启动的 Codex，在 terminal、原 shell、前台进程及 Agent 名称一致、blocked 且 launch_pending 的条件下处理更新页。需要同时满足 Herdr 的 `startup_update` 检测及可见页的 `Update available ·` / `enter continue · esc skip` 提示；发送前复核身份、状态序号和任务有效性，向该 Agent 名称发送一次 Esc。之后仍要求同一进程 idle/done、interactive_ready 且不再 launch_pending，才进入原会话核验和派发流程。

其他弹窗、缺少 Esc 提示的旧版更新页、身份替换、任务取消及查询失败均不自动回答。按键响应不确定或跳过后持续阻塞时保留原失败/待核实处理，不重发 Esc、agent.start 或正文。适用于新建和精确恢复共用的启动适配器；没有数据库或 IPC 变更，不修改全局 Codex 更新设置。

## 自动化验证

- 新增真实 CLI 适配器边界回归，PowerShell fixture 模拟 Herdr 的启动阻塞、检测结果和可见更新页。`cargo test --offline --locked --lib startup_update_skips_once -- --nocapture` 在修正前复现同一 `agent_not_ready`；修正后通过。
- 覆盖启动返回错误与先返回成功后出现更新页两种时序、只启动一次/只发一次 Esc、其他弹窗和无 Skip 提示拒绝、名称/terminal/进程替换、写入前取消、按键结果不确定不重发、跳过后仍阻塞有界退出、精确启动错误信封。
- `cargo test --offline --locked --lib -- --test-threads=4`：285 passed、0 failed、28 ignored，耗时 30.35 秒。随后只新增一项默认 ignored 的真实更新页验收测试，单独执行通过；其余常规测试未改变。既有 terminal dead-code 警告仍存在。
- 三个修改 Rust 文件的 `rustfmt --edition 2024 --check` 及 `git diff --check` 通过。仓库其他文件已有格式差异，本次未纳入格式化变更。

## 真实本机验证

使用 Herdr 0.9.5、独立 `kanban-acceptance-startup-update-20261030` Workspace，不发送业务任务正文。CLI 记录器只记录命令种类及按键目标，不更改请求。

第一次设置 Workspace 环境 PATH 未覆盖交互 shell 的解析结果，实际启动 Codex 0.166.0，无更新页；“必须发送一次 Esc”的验收断言失败，不能作为更新 Skip 已通过的证据。该测试 Workspace 已清理。

随后在第二个独立测试 shell 内临时指定本机保留的 0.165.0 路径，并以 `codex --version` 确认。未修改系统 PATH、安装入口或用户配置。执行：

```text
cargo test --offline --locked --lib live_startup_update_skips_once -- --ignored --nocapture
```

结果：1 passed，9.24 秒。捕获真实 `agent_not_ready`，随后自动 Esc 成功，同一 `w28:p1` / `44512:103180` 达到就绪。记录中 `agent start` 恰好一次，`agent send-keys kanban-update-check esc` 恰好一次，无 `agent prompt` 或 `pane send-text`。测试完成后关闭测试窗格，所属最后一个 Tab/Workspace 随之消失。

本地原始命令记录：`.scratch/startup-update-live/trace.txt`；首次未触发更新页的记录为同目录 `trace-no-update.txt`。业务失败 Agent 在实施前已不存在，本次没有恢复、重发或修改该业务任务。

## 验收边界

真实测试验证启动更新页自动跳过和同进程就绪，未发送模型任务、未验收完整桌面拖拽派发/精确 resume/SSH 更新页。后续派发的不重发规则由既有 Rust 回归覆盖。未重新打包或替换正在运行的安装版；旧 unknown 卡片不会因源码修正自动恢复。
