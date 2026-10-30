# SSH Prompt 提交中断恢复与阶段诊断

日期：2026-10-30。规格：[GitHub #44](https://github.com/legendxcheng/herdr-kanban/issues/44)，标签 `ready-for-agent`。用户已授权实现，并确认提交/工作流测试入口、SSH 故障与真实独立窗口验收边界。

## 诊断证据与范围

只读现场：devpc 的 `w3:p1S` 显示完整鱼蛋判定提示词，Codex idle 且未报告会话 ID。该启动时段仅见一次按键请求，正常 `w3:p1T` 启动有三次。日志缺少按键名/目标，旧 pane 对应操作记录未找到，不能将具体旧故障唯一归因于某次查询失败。

当前代码存在可确定复现的路径：正文写入后，状态/进程查询的临时错误使后续 Right/Escape/Enter 提前终止。新增两项行为回归在修复前均失败：临时断连直接返回 `machine.connection_failed`；持续故障只观察两次便结束，而非完成有界恢复。未向旧业务窗口写入或补按 Enter。

## 实现

- `herdr_codex_submit` 对完整状态观察共享最多两次重试、每次间隔 300ms。仅四个明确临时错误码可重试；每次重新检查当前任务、操作和完整目标身份。busy、取消、重置、身份变化、无效协议均停止。原单次请求时限继续有效；两次重试不是整个提交的 600ms 总截止时间。
- `remote_control` 仅允许精确形状的 `agent get`、`pane process-info --pane` 观察在临时传输失败后被提交观察器显式恢复。旧连接销毁，新连接复核 profile、冻结机器与 helper、主机/session/请求身份；写入回执丢失和身份/协议错误保持锁定。手动派发建立同样的操作级连接，自动启动沿用既有作用域，不覆盖失败状态。
- 正文、Right、Escape、Enter 均不重发。本轮不修改正文编码，保留 bracketed paste、严格完整正文回执和状态确认语义。共享工作区原先未提交的单 CR 换行传输及本机回执接线修复也被保留，未纳入本次 SSH 恢复提交。
- `dispatch-input.jsonl` 逐步记录请求、传输确认、失败及观察恢复，关联操作和目标身份。只保留稳定错误码，不含正文、标题或外部错误正文；2 MiB 后下一条重置，写日志失败不影响业务。requested 是步骤发起，acknowledged 是传输确认，均不单独代表模型执行。
- 通过两个中英文错误码区分尚未发起 Enter、Enter 已发起但结果未确认。未知结果继续保留占用和核实入口，错误保存到已有派发 evidence；自动启动使用原生命周期 error，手动派发使用原弹窗反馈。无数据库迁移或新增 IPC。

## 自动化验证

```text
cargo test --manifest-path src-tauri/Cargo.toml --offline --locked --lib observation_ -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --offline --locked --lib -- --test-threads=4
cargo check --manifest-path src-tauri/Cargo.toml --offline --locked --lib
npx vitest run src/i18n.test.ts src/DispatchDialog.test.tsx
npm run build
```

- 首条命令在原逻辑上 2 项失败；修复后的提交测试覆盖每个中间观察点、共享重试预算耗尽、重试期间取消/忙碌/身份变化、各输入步骤失败及不重放。
- 最终 Rust 常规套件：303 passed、0 failed、32 ignored，33.37 秒。流式子进程测试分别验证 EOF、800ms 慢回复超时、重新连接、错误 session/请求 ID 不恢复及写入回执丢失不恢复。工作流验证阶段错误持久化、unknown 占用及重复派发被拒绝；诊断测试验证立即写入、作用域隔离和正文脱敏。
- 前端实际匹配并运行 `i18n.test.ts` 一份文件，6 项通过；没有名为 `DispatchDialog.test.tsx` 的匹配测试，不能据此声称派发组件验收通过。
- `npm run build`（TypeScript + Vite）通过，保留既有大 chunk 警告。没有重跑全量前端套件。
- 非测试构建 `cargo check --lib --offline --locked` 通过，保留既有 dead-code 警告，确认正式构建不依赖测试故障注入代码。
- 差异检查使用仓库 CRLF 约定：`git -c core.whitespace=cr-at-eol diff --check` 通过。

## 真实 SSH 验证

devpc、Herdr 0.9.5、Codex 0.166.0。独立 Workspace `kanban-acceptance-ssh-submit-20261030`（`wN`），独立磁盘 SQLite，各次新建 pane 和操作 ID，固定回复技能 `kanban-submit-fixture`。调用真实 `Workflow::start_task`、CLI、SSH 控制 helper 和 Codex。沿用目标已部署 helper，本改动不要求新的 helper 方法。上述真实多行验收使用包含原有未提交单 CR 换行修复的共享工作区；本次单独提交 SSH 恢复部分，不应把完整正文通过结论直接归于缺少该换行修复的独立提交快照。

| 用例 | 远端 pane | 精确正文会话 | 结果 |
| --- | --- | --- | --- |
| 路径开头的短中文多行、空行 | `wN:p3` | `01a18c24-3973-71e1-9f8d-1aa8a0ff0c34` | 完整正文、一次回复、正文/Enter 各一次 |
| 中文/空行/Tab/末尾技能，Right 后丢弃一次观察回复 | `wN:p4` | `01a18c24-b7fc-7272-8052-1bae6200d5a8` | `after_right` 重试一次，重新连接后完成，无重放 |
| 超过 11K 字符多行及末尾技能，Escape 后丢弃一次观察回复 | `wN:p6` | `01a18c26-fd92-74f1-8fb2-04554e86cd2b` | `before_enter` 重试一次，重新连接后完成，无重放 |

每例都检查 `in_progress / valid / sent / succeeded`、一条派发记录、同一会话唯一完整正文回执，以及远端会话日志恰好一次 `task_complete`，回复严格为 `KANBAN_RECEIPT_OK`。同时核对输入日志中四个输入阶段各一次 requested/acknowledged。故障注入只丢弃测试连接中一个已完成的只读回复，再由正常错误路径关闭该 SSH 子进程；这证明真实 SSH 重建和后续提交，不等同于关闭物理网络。慢查询超时与未知写入由真实本地流式子进程夹具覆盖。

入口：`workflow_tests/ssh_submit.rs` 的 opt-in `live_ssh_submission_recovers_read_disconnect_without_replaying_input`。使用显式机器配置、Workspace、cwd、证据目录环境变量；`KANBAN_LIVE_CASE` 可选择 0/1/2，未设置时运行三例。故障注入只存在于测试构建，正式应用没有入口。

验收脚本先修正了两个夹具问题：设置项目电脑前须移除夹具任务，以遵守 `machine.project_in_use`；用户消息应从 `response_item` 计数，不能依赖当前 Codex 未写出的 `user_message` 事件。最后长文本日志统计曾超过 Windows 命令行长度上限，改用 SHA-256 匹配后仅重跑第 2 号长文本例，通过（42.83 秒）；此前第 0、1 号例已通过。因此三例证据来自两次执行，不声称最终三例一次全跑通过。

本地证据位于 `.scratch/ssh-submit-live/`：

- 短文本：`89cff23e-07a0-4541-b4f7-0da2565f23fe/`
- Right 后断连：`d7762e62-e529-4fcc-9dde-6d29a01d12ab/`
- Escape 后断连：`3b6f48c2-82c9-4ec4-b5c2-b9300810b94b/`

各目录包含数据库、输入日志、会话计数和结果。成功用例各自关闭测试 pane；夹具失败遗留的两个已完成测试 pane，在复核 Workspace 标签、全部 pane/terminal 身份及 idle/done 后，随专用 `wN` Workspace 一并关闭。远端测试目录和会话证据保留，业务窗口未操作。

## 交付边界

用户随后授权 Git 提交。仅暂存本次 SSH 恢复的 15 个文件/改动块，原有多行输入及本机回执接线修复保持未提交。将精确暂存树导出到 `.scratch/ssh-submit-commit/source/` 独立检查：Rust 常规套件 301 passed、0 failed、31 ignored（32.81 秒）。该结果证明本次独立提交可以编译并通过常规回归；真实多行验收的组合依赖边界仍如上所述。

未执行原生 WebView2 操作或安装包验收；未打包安装、未替换正在运行的 Kanban。修复需要后续构建部署才对安装版新派发生效。没有自动恢复历史 unknown 任务，也没有宣称已唯一确定原业务故障的根因。
