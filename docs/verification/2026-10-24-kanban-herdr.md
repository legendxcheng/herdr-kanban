# Kanban 独占 pane 与右侧终端验收

日期：2026-10-24。规格：[任务独立 pane 与右侧交互终端](../superpowers/specs/2026-10-24-kanban-herdr-task-pane.md)，来源 #4，实施拆分 #5–#9。

## 实现与版本

- 三列等宽收窄，默认终端 300 CSS px；列最小 200px，终端最小 280px，分隔线支持拖动及方向键。
- SQLite 跨项目唯一绑定、持久化派发操作 ID、未知结果核实、旧绑定显式修复。
- React xterm + Rust/Tauri Channel + Herdr JSONL 控制流，连接、输入、resize、输出确认和安全释放均走真实进程。
- 实测 Windows Herdr `0.8.0-preview.2026-08-04-d78e3d3b5126`，server protocol 19。
- 此版本 Windows `agent attach` 实际返回 `direct terminal attach is not supported on Windows yet`。因此改用同版本源码中存在且已实测的 `terminal session control <pane> --cols N --rows N`。该接口提供可写终端，非只读 agent read。未使用 takeover。

## 自动化检查

| 检查 | 结果 |
| --- | --- |
| `npm test` | 25 项通过；真实 React/API 封装，模拟 Tauri/xterm 外部边界；包含连接等待期间 resize 与语言回退回归 |
| `npm run build` | TypeScript 与 Vite 通过；主 JS 约 993.81kB，gzip 292.41kB，存在 chunk 超过 500kB 的提示 |
| `cargo test` | 11 项通过；2 项需要隔离 Herdr 的 live 测试默认 ignored |
| `cargo fmt --check`、`cargo build` | 通过 |
| 两个 ignored live 测试显式运行 | 在专用 Herdr workspace 中通过派发/独占、ANSI 输出/输入/resize/安全 detach；不把默认跳过计作通过 |
| `python -m pytest tests -q` | 收集被工作区已有 `tests/test_explorer_helper.py:181` 语法错误阻断；该文件被本机 `.git/info/exclude` 排除，最后修改于 2025-07-14，本次未修改 |
| Git 跟踪的 Python 测试 | 对 `git ls-files tests/test_*.py` 返回的 14 个文件执行 pytest：110 项通过；不含上述本机额外文件 |

Rust 覆盖：跨项目冲突、并发抢占、同一操作去重、有效绑定再派发、明确失败释放、未知结果恢复、发送后 SQL 失败、重复迁移/重开、旧共享修复不重发、身份变化拒绝、待建立连接取消，以及数据库实例锁。

## 真实 Tauri / Windows Herdr 检查

使用专用 Project、`KANBAN_DATA_DIR` 指向的独立 SQLite、独立 WebView2 用户目录，以及 `kanban-acceptance-20261024` workspace。验收操作只作用于该 workspace 的测试 Agent，不向个人工作 pane 发送内容。通过 CDP 操作实际 Tauri WebView；后端 invoke、SQLite、CLI 和 Agent 均未替换为 mock。

| 场景 | 实际观察 |
| --- | --- |
| 派发 A/B | UI 分别选择 p1/p2；B 的选择器将 A 的目标禁用；实际库中为两个独占绑定 |
| 输入与快捷键 | A 输入并 Enter 后收到 NATIVE_INPUT_A；B 输入草稿仅出现在 B，退格可删除；`n` 没有抢走终端焦点 |
| 快速 A→B→A | 最终标题、画面和可输入目标为 A；没有 pageerror |
| 切换项目 | 切项目立即清空旧选择和 xterm，返回原项目仍需明确选择；快速切项目后只连接最后选择的任务 |
| 布局 | 1280×800：三列约 209.33px、终端 300px；900×600：三列各 200px、看板内部 scrollWidth 624px，页面无横向溢出、终端可见；1600×900：三列各 316px、终端 300px |
| 调整宽度 | 方向键将终端加宽 24px，终端继续工作；独立控制流实测 frame 尺寸从 80×24 变为 100×30 |
| 控制器中断与重连 | 只结束本应用的控制器子进程，UI 显示断开；重新连接恢复同一 Agent，无重新派发 |
| 看板进程重启 | 强制退出专用看板进程后，A/B 继续运行；重开后绑定及查看可恢复 |
| 状态与删除 | 后端直接改为 in_progress 被拒绝；UI 标记完成、删除清理选择和连接，底层 Agent 保留 |
| 旧共享记录 | 两个旧 in_progress 任务保留相同历史目标和快照；先显示待修复，无可输入终端；分别修复 p1/p2 后连接，旧快照 LEGACY_SHOULD_NOT_SEND 未进入任一 Agent |
| Agent 替换 | 在测试 Agent 输入 /quit，UI 断开；同 pane 启动新 Agent 后，普通重连因身份变化被拒绝；显式修复后连接成功且不重发 |
| pane 被关闭 | 关闭隔离测试 pane p2 后显示断开；选择 p1 任务仍可正常连接 |

本地复现脚本、原始 JSON 和截图保存在 `.scratch/kanban-herdr-task-pane/acceptance/`，不提交包含机器路径、运行数据或生成程序的文件。`native-check.cjs`、`native-exercise.cjs`、`native-lifecycle.cjs`、`native-replacement.cjs` 是本次隔离 fixture 的验收脚本，不应直接对个人数据库运行。

验收结束后已停止专用看板进程，并明确关闭本次创建的测试 workspace；这属于测试环境清理，与产品的 detach 行为分开。个人 workspace 和既有开发服务未关闭。

## 明确未覆盖的范围

- 125% 缩放使用实际 WebView 的 device metrics 模拟；尚未切换 Windows 系统 DPI 或跨不同 DPI 显示器验证。实际系统 DPI 切换仍需人工补验。
- 没有重启共享 Herdr server，以免中断用户工作；已覆盖本应用控制器中断、Agent 退出/替换和 pane 丢失。
- 已运行开发版 Tauri 原生程序与生产前端构建，尚未在独立安装包/干净机器上验收。
- 本次验证普通文本、粘贴式输入、Enter、退格和 /quit；未触发会执行文件修改的 Agent 权限确认。独立 pane 隔离终端上下文，不保证隔离文件目录或 worktree。

## 维护入口

新增 Herdr 版本后，先在专用 pane 验证 JSONL 控制流和安全 release，再执行 live 测试；不能仅凭帮助文本中存在 agent attach 判定 Windows 可用。所有 UI 模拟结果与原生验证保持分开记录。

## 双轴代码审查

以开发前提交 `f13362f` 为基线，按 code-review 技能并行审查 Standards 与 Spec。

- Standards 初审：1 项 ADR 违反（写死中英文目录）、2 项重复逻辑建议。已用自动目录收集及统一语言回退修复 ADR 问题，并合并 workspace 解析、pane 归属查询/校验。
- Spec 初审：1 项实现缺陷（连接期间 resize 丢失）。已在连接成功后显式发送最新尺寸，并增加迟到连接回归测试。其余验收覆盖缺口保留在上文，不作为已通过。
- 两路分别复核修复提交后，Standards 剩余原发现 0 项，Spec 剩余代码缺陷 0 项；环境验收缺口仍保留。

## 技能引用与 Windows Codex 自动提交修复

2026-10-24 补充：用户报告技能被展开为正文，以及输入框停在 `[Pasted Content …]`。

- 前端实际路径回归先失败：`resolveMentions` 将技能节点变成整份 SKILL.md。现在发送与预览均保留 `$技能名`；文件引用继续展开。
- 本机 Herdr `0.8.0-preview.2026-08-04-d78e3d3b5126` + Codex `0.164.0`，专用 workspace 中复现 11,682 字符停在粘贴输入框。CLI 的成功响应没有证明需求已开始。
- 对照本地 Herdr Git 对象：已安装提交 `d78e3d3b5126` 的 `src/app/api/agents.rs` 固定延迟 300ms 发 Enter；较新提交 `43b36b68` 使用非字符按键结束 Windows Codex 的粘贴缓冲。看板兼容处理为发送正文、再次核验身份、发送 Right + Enter；不重复调用 agent prompt。
- `pane send-text` 成功时没有 stdout，这是其 CLI `send_ok_request` 的正常行为；仅此命令接受空成功响应，其余 JSON 响应校验保留。
- 正文发送后，即使后续 CLI 启动失败或身份改变，也保留 unknown 和预占，不自动重放正文。回归覆盖这些部分成功情况。
- 真实 Rust Workflow → Herdr → Codex 的 ignored 测试已加强：发送超过 11K 字符的提示词，建立终端连接，并等待本次唯一标识的实际 Agent 回复。不是仅断言 CLI 成功或数据库状态。
- 本次验证使用 `kanban-acceptance-prompt-20261024`、`kanban-acceptance-submit-20261024` 隔离 workspace，不向用户既有任务 pane 发送输入。

前端组件测试与 Rust 单元测试分别覆盖技能预览/派发和失败隔离；真实长文本提交另行通过 live 测试。未重新执行全套原生 UI 生命周期、其他 Agent 或安装包验收。

本次最终结果：前端 28 项通过，Rust 14 项通过（2 项 live 默认跳过），加强后的长文本 live 派发测试显式运行通过；前端生产构建、`cargo build --offline --locked`、`cargo fmt --check` 通过。前端保留原有大 chunk 提示。测试 workspace 已关闭，用户现有任务未重发。
