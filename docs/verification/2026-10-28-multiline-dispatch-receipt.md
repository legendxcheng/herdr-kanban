# 多行提示词与延迟回执修复验收

日期：2026-10-28。

## 故障与复现

用户的分支 C 任务在本机 Herdr `w9:p1W` 执行，pane、terminal 和进程身份与派发记录一致。Kanban 在北京时间 15:52:57.275 发起派发，15:53:08.752 记录启动失败；对应 Codex 会话在 15:53:21.894 记录用户消息。保存正文包含两处 LF，实际用户消息没有 LF；移除保存正文的两个 LF 后才相同。原始状态轮询与用户键盘事件没有完整记录，不能进一步判断原接收延迟是否全部来自启动或期间有人工操作。

旧实现使用 `pane send-text` 发送原始字节，Codex 会将裸 LF 当作按键事件，导致多行内容合并。`codex_receipt` 正确地拒绝这份不同正文；不应通过去掉换行来放宽成功判据。

新增真实回归 `workflow::tests::live::live_multiline_start_has_exact_receipt_and_one_submission`，在专用 Workspace `kanban-acceptance-multiline-20261028` 内走真实 `Workflow::start_task`，使用独立内存 SQLite 和每次新建的 `kanban` pane。

- 旧实现：`w2B:p2`，会话 `01a1818b-3daa-74d1-8090-6f39d8f4ec90`。启动返回成功，但中文、多行、空行被拼成一行；等待 60 秒仍无精确正文回执，回归失败。仅凭状态或固定回复无法发现此问题。
- 修复后普通多行案例：`w2B:p3`，会话 `01a1818c-d0fa-72f1-b4e8-3793a30953eb`；08:07:37.180 UTC 的真实用户消息与保存正文逐字相同，回归通过。
- 修复后中文、多行、空行、Tab 与末尾 `$kanban-dev` 案例：`w2B:p4`，会话 `01a1818f-0f84-7a62-b932-af0e5548c9ac`；08:10:06.141 UTC 的真实用户消息逐字一致，完整回归在 14.04 秒通过。两次成功测试均验证仅一条派发记录，并在测试会话结束后关闭各自 pane。

## 实现

- `herdr_codex_submit.rs` 对含 CR、LF 或 Tab 的正文添加 bracketed paste 开始／结束边界，单次传输到原目标；数据库正文不包含边界，后续 Right、Escape、一次 Enter 保持原有顺序。
- 原 10 秒状态确认超时后，以 500ms 间隔、最多 30 秒／61 次探测补查精确会话回执。单次 CLI 请求仍受既有独立超时约束，因此整体墙钟时间可能超过窗口。
- 补查复用原身份、操作有效性与回执校验；命中后 Workflow 再核对并提交。迟到状态变化不单独确认为成功；无证据、失效操作或身份变化仍保留不确定结果，不重发任何输入。
- 自动启动与手动派发共用此边界。没有新 IPC、迁移、旧任务自动重置或未知结果自动重放。

## 自动化验证

先运行 `cargo test --manifest-path src-tauri/Cargo.toml --offline --locked --lib herdr::codex_submit::tests -- --test-threads=4`，新增的多行传输和 23 秒迟到回执两项在旧实现上失败，修复后相关 11 项通过。另覆盖补查到期、补查期间操作失效、身份读取失败以及正文／Enter 各一次；时间延迟采用可控时钟推进，不宣称复现了真实账号初始化慢的全部原因。

完整 `cargo test --manifest-path src-tauri/Cargo.toml --offline --locked --lib -- --test-threads=4`：257 passed、0 failed、27 ignored。包含既有回执重新核验、进程替换、迟到恢复、事务回滚及任务重置保护；共享工作区已有其他未提交功能，计数包含它们。保留既有 terminal 模块未使用成员警告。

`npm run build`：TypeScript 与 Vite 构建通过，保留既有大于 500 kB 的 chunk 提示。本次没有前端代码修改，未额外运行前端测试套件。

提交前将仅包含本次 4 个文件／改动块的暂存树导出到 `.scratch/multiline-commit-20261028`，独立运行完整 Rust 测试：242 passed、0 failed、25 ignored。确认本次修复不依赖共享工作区其他未提交改动；测试数量与上述共享工作区不同是代码快照范围不同所致。

## 边界

真实验收覆盖本机 Herdr／Codex、完整自动启动调用链及 SQLite 成功状态（in_progress / valid / sent / succeeded），严格回执要求同时间范围内唯一完整正文。尚未执行本次 SSH 或原生 WebView2 拖放验收；没有以这些后端真实测试替代桌面点击验收。

未修改用户故障任务或向原窗口发送输入。历史换行已丢失的任务仍需原有“核实派发结果 → 确认已发送”，不能由严格正文校验自动恢复。未安装或替换运行中的应用，未推送 Git。

## 重新打包

按用户后续要求，通过 `powershell -NoProfile -ExecutionPolicy Bypass -File scripts/build-kanban.ps1 -NoPause -NoOpen` 完成 Windows x64 Release／NSIS 构建，日志为 `release/kanban/logs/build-20261028-161434-9624.log`。脚本从当前工作区创建 `release/kanban-source` 快照，因此安装包包含工作区已有的其他未提交功能；Git 提交仅包含本次两个 Rust 文件、本文和结构文档对应段落。

安装包：`release/kanban/0.1.0/Herdr-Kanban-0.1.0-windows-x64-setup.exe`。SHA-256：`9c88806dc8f1528272b8bd6f5b76174535b4a111f622d3a80250fe23c90e7915`，与同目录 `SHA256SUMS.txt` 一致。本次两个 Rust 文件在暂存树、工作区和打包快照中的字节完全一致。打包成功不代表已安装或完成本次安装向导／桌面验收。
