# 当前任务单 pane 显示验收（2026-10-25）

## 行为与实现

选择看板任务或重连时，`native_client_wire::Position` 在 Kanban 专用客户端连接上发送 `pane.zoom`，明确使用 `mode: on`。Herdr 同时聚焦目标 pane 并将它单独显示；不使用 toggle，不关闭其他任务，也不改变任务绑定、提示词或会话存储。

就绪条件为：endpoint generation 1 支持 pane.zoom；请求 ID、boot_id、目标 pane、成功回执及 shell 快照一致；快照证明目标是焦点，且所属 Tab 已 zoom，或该 Tab 确实只有一个 pane。Herdr 单 pane 的 `single_pane` no-op 属于成功；仅 focus 成功不足以显示窗口。

连接就绪后继续观察 shell 快照。后台 `pane split --no-focus` 会清除 Herdr 的 Tab zoom；若当前仍聚焦绑定任务，则发送一次新的 `on` 请求并等待回执和快照，期间隐藏查看窗口。请求串行、每次使用新 ID，恢复限时 15 秒，重复快照不重复发送；失败关闭查看连接。官方渲染和输入帧仍原样转发。

用户主动在 Herdr 内切换到其他 pane/workspace/tab 时仍允许导航，不持续抢回焦点；重新点击不同任务或重连会重新定位。Herdr 0.9.5 的 zoom 和同 Tab pane 焦点是共享状态，因此另一个客户端若显示同一 Tab，也会受影响。关闭查看窗口不会恢复共享 Tab 的分屏或关闭 Agent。

## 自动检查

- `cargo test --offline --locked --lib`：共享工作区当次 150 passed、12 ignored，包含同时进行的其他功能改动。新增/调整的协议回归共 6 项通过。
- 协议回归覆盖：显式 on、单 pane no-op、焦点存在但未 zoom 不就绪、回执和快照两种顺序、分片响应、重复快照、旧请求回执隔离、新增分屏后的恢复、失败回执、错误目标和能力不兼容。
- `npm test`：136 passed；`npm run build`：TypeScript/Vite 通过，保留既有大 chunk 提示。
- `cargo build --offline --locked --no-default-features` 通过；本次 Rust 文件格式检查和限定范围的 `git diff --check` 通过。

## 真实 Herdr / WezTerm

使用独立 session `kanban-acceptance-pane-zoom`、独立 APPDATA 和数据目录 `.scratch/task-pane-zoom/`，Herdr 0.9.5。新增 opt-in 测试 `native_terminal::connection::tests::live_client_zoom` 显式执行通过：

1. 单 shell pane 可就绪，随后后台拆出第二个 pane 自动恢复 zoom。
2. p1/p2 切换、重复连接已 zoom 的 pane、重连均得到正确焦点和 zoom 状态。
3. 旧连接被新选择取消后，最终选择保持正确。
4. 关闭所有查看连接后，两条原 pane 仍存在。

该用例要求测试 session 名以 `kanban-acceptance-` 开头，并预先只有 `w1:p1` 一个 shell pane；运行后会留下测试用第二个 pane，由测试会话清理。

## 原生看板链路

使用本次编译的 debug 可执行文件隔离副本、真实 Tauri/WebView2、SQLite、WezTerm 和两个空闲 Codex Agent。测试卡片在隔离数据库中预置真实 Agent 身份，未发送模型提示词。

- 实际点击任务 A/B，确认 Herdr layout 的 focus/zoom；从真实 WezTerm `get-text` 读取的画面只包含对应任务的未提交标记。
- 经 WezTerm 原生输入送入 A 的标记未进入 B；切换后 B 标记未显示在 A 的画面中。
- 看板保持选中 A 时，后台新增第三个 pane，自动恢复 A 的 zoom；随后仅关闭测试新增 pane。
- 快速 B→A 选择以及 WebView reload 后均重新显示 A。
- 清除未提交标记后关闭看板；查看进程退出，两条 Agent 的 terminal_id 不变。

证据：`.scratch/task-pane-zoom/desktop-result.json`、`task-a-screen.txt`、`task-b-screen.txt`；脚本 `desktop_acceptance.py`。脚本首次使用文件入口时外部 CLI 子进程报访问拒绝，最终以 Python stdin 加载同一脚本完整执行通过；没有放宽 server 权限。桌面区域截图被其他前台应用覆盖，不用作视觉通过证据，单 pane 画面由真实终端缓冲区和 layout 交叉确认。

验收结束后停止了本次隔离 Herdr server；未重启用户 default server，未提交模型请求。没有生成/安装 NSIS 包；物理鼠标、跨屏 DPI、IME 和同 Tab 双客户端争抢不在本次完整验收范围。
