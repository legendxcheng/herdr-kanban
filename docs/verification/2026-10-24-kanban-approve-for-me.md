# Kanban Codex 默认 Approve For Me 验证

日期：2026-10-24。用户在会话中确认方案并授权开发；本次未发布新的 GitHub Issue。

## 实现

- `herdr_codex.rs` 的统一启动路径加入 `--approve-for-me`，新建与精确 UUID 恢复均生效，保留 `--no-daemon` 和任务目录。不修改全局配置；保留已有运行绑定时沿用该进程当前模式。
- 恢复命令识别兼容新旧两种精确 argv，未知参数、非法 UUID 和其他程序均不能进入补充分支。
- 模式为 workspace-write / on-request / auto_review，与[官方说明](https://learn.chatgpt.com/docs/sandboxing)一致。

## 真实回归发现及处理

首次真实生命周期回归在恢复时报 `flow.session_mismatch`。当前 Codex 0.164.0 开启自动审批后，进程会额外持有辅助会话写锁；原“只能持有一条写锁”的检查因此拒绝正常恢复。对照手动审核配置后只剩目标写锁；改变参数位置和关闭 compaction reuse 均没有解决，未将这些实验配置加入产品。

本次采用启动证据：仅在看板刚启动的精确 resume 进程中，采集包含目标 UUID 的两个写锁 ID，作为 `Target.codex_resume_locks` 持久化到已有 JSON。后续要求同一完整目标身份、精确 argv 和完全相同的锁集合；不对任意第二条锁放行，不在运行中的旧绑定上重采。旧 JSON 默认空集合，继续采用原唯一写锁检查；无数据库迁移或新 IPC 命令。

补测发现 Herdr 的 interactive_ready 可早于 Codex 打开全部写锁；采集最多等待 5 秒，失败保留待核实窗口。进程内切换到另一会话会改变锁集合，不能用旧 argv 和旧 ID 完成清理。

## 结果

环境：Windows、Herdr `0.8.0-preview.2026-08-04-d78e3d3b5126` / protocol 19、Codex CLI 0.164.0。

- `cargo test --offline --locked --lib`：99 项通过、6 项 opt-in live 默认跳过。覆盖新旧 argv、未知参数、非法 UUID、原唯一锁规则、辅助锁集合及切换后的新增/替换锁拒绝。计数包含共享工作区的其他功能测试。
- 显式运行 `live_tasks_keep_distinct_sessions_through_completion_and_resume`：通过，31.85 秒。真实 Workflow、SQLite、Herdr、Codex，无 CLI 替身。双任务独占 pane 和不同会话；实际进程参数均带 `--approve-for-me`；完成关闭对应 pane；数据库重开后精确恢复；恢复后再次关闭并重开数据库，持久化证据不丢失；不输入新消息即可再次完成。
- 同一 live 用例补测恢复 A 后在同一进程执行本地 `/resume B`：返回 `flow.session_mismatch`，保留 pane、清理待重试标志及保存的 A UUID。测试最后显式清理自己创建的窗口。
- 独立读取本轮两段真实对话：每段只有一条原始用户提示词，恢复占位文本未发送；记录的权限均为 on-request / auto_review / workspace-write。
- 专用权限检查窗口的新建与恢复 `/permissions` 均显示 `Approve for me (current)`。该检查使用真实 CLI；它与看板生命周期回归分别记录。
- `npm run build`：TypeScript 及生产构建通过，保留既有大于 500 kB chunk 提示。
- `cargo fmt -- --check` 及本次涉及文件的 `git diff --check` 通过。

脚本、失败及成功日志、权限菜单原文和对话统计位于 `.scratch/kanban-approve-for-me/`。专用测试 workspace 已清理，未操作个人工作 pane，未修改用户全局 Codex/Herdr 配置。

## 边界

本次没有重跑 Windows Tauri/WebView2 拖放、浏览器组件矩阵或安装包验收，也未替换运行中的应用。真实验证到 Rust Workflow、SQLite 及 Herdr/Codex；不能将其描述为本轮桌面 UI 验收。

启动证据基于当前 Windows 进程句柄和 Codex 写锁行为；未知版本或无法观测时仍保守拒绝补充分支。没有模拟外部自动审核服务的通过/拒绝，也没有测试一次实际越界操作的审批结果；本次确认的是默认选择的模式及其生命周期兼容性。
