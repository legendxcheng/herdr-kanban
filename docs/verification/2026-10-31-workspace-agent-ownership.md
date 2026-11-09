# 普通终端 Agent 导致 Workspace 状态查询失败修复

日期：2026-10-31。用户反馈 `w2A` 的整批状态查询返回 `stage=snapshot / code=status.external_error`，确认修复普通终端被当成卡片任务的问题。

## 根因和修复

- `workflow::pane_owner` 既返回真实任务 ID，也用 `workspace-terminal`／`retired-execution` 标记普通终端与隔离的旧会话。此前 `agent_status::assign_agents` 把所有非空结果当作任务 ID，SQLite 查不到任务便返回 `Query returned no rows`，中断整个 Workspace 快照。
- 新增内部 `PaneOwner` 类型与 `pane_ownership`，区分 Task、WorkspaceTerminal、RetiredExecution。状态服务仅对 Task 查询任务和绑定；其余归属保留 `occupied_by`，不生成 `verified_task_id`。普通终端中手动运行 Codex 是允许的，不需要关闭终端或重建任务。
- 原 `pane_owner` 作为兼容入口保留既有占用标记。派发和生命周期调用不变，普通终端／隔离目标仍受占用保护。任务核验仍比较完整身份并拒绝跨项目、已归档和未决操作的状态归属。
- 映射层增加项目归属、终端占用、任务记录和绑定读取阶段；预期任务记录缺失归为 `status.database_row_missing`，绑定解析失败归为 `status.binding_read_failed`，其他本地读取错误使用对应分类。中英文详情同步，不记录原始数据库或 JSON 内容。
- 无数据库迁移、无绑定重写、无终端输入或自动重发，无状态查询超时／重试变更。

## 验证

- 红绿回归命令：`cargo test --offline --locked --lib agent_status::tests::non_task_agents_keep_ownership_without_breaking_workspace_status -- --exact --nocapture`。旧实现确实返回整批 `snapshot / status.external_error`，不是以无关断言失败代替复现。
- 修复后状态服务 9 项通过。新增矩阵覆盖本机／远端身份、普通终端／旧会话、正常任务 idle／working／blocked／done；实际 SQLite 占用表与公共服务调用参与，Herdr 使用测试替身。正常任务继续获得 `verified_task_id`，保护目标在后台快照和派发候选中仍标为占用，任务绑定保持原值。
- 新增数据库异常回归覆盖真实任务记录缺失和绑定 JSON 损坏，验证明确错误分类及不泄露 JSON 原文。
- 前端定向状态测试 24 项及 `npm run build` 通过（包含 TypeScript），保留既有 bundle 大小提示。
- 首轮默认并发完整 Rust：324 通过、3 失败、34 跳过。失败均位于未修改的标题 HTTP 模块：`connection_failure_is_transient_without_raw_diagnostics`、`shutdown_cancels_stalled_headers_or_body_and_preserves_retry_budget`、`http_failures_keep_summary_and_only_transient_errors_schedule_retry`。不能将此轮称为全绿，未修改这些用例或认定已查明波动根因。
- 串行完整复核：326 通过、1 失败、34 跳过，131.22 秒。剩余失败为未修改的 `title_deepseek::tests::shutdown_cancels_stalled_headers_or_body_and_preserves_retry_budget`，接收测试消息得到 `Disconnected`；全量测试仍不是全绿。
- `HERDR_ENV=1 cargo test --offline --locked --lib herdr::agent_discovery::tests::live_background_task_statuses_without_viewer -- --ignored --exact --nocapture` 通过：本机真实 Herdr 查询 9 条绑定任务返回 idle，在线项目无逐 pane 查询失败；一个已缺失的历史 Workspace 单列为不可用。只读本机数据库和 Herdr，没有打开终端或发送输入。

## 真实环境边界

- 诊断时 xc 上实际 helper 两次读取 w2A 成功（约 156ms／125ms），返回 p2=idle、p3=done；p3 位于 `kanban-terminal-4535c850-645c-4af2-b699-593bd961f7fc` 标签页。与本缺陷触发条件吻合。
- 未读取另一台看板电脑的数据库，未在该电脑验收修复后状态恢复；本机／远端矩阵属于模拟 Herdr 验证，不能替代真实 SSH 验收。
- 保留本次会话前已有未提交改动；完整测试及构建使用当前工作区。未安装或替换正在运行的程序。

## 安装包

- 正式脚本 `scripts/build-kanban.ps1 -NoPause -NoOpen` 构建成功。构建快照中 24 个改动／新增文件与工作区逐个哈希一致。
- 交付：`release/kanban/0.1.0/Herdr-Kanban-0.1.0-windows-x64-setup.exe`。
- SHA-256：`0e593395583976368e97d41c95872a418c825a493c2f05499c5d1cdd2c1e3eb0`；交付包、NSIS 原始包与 `SHA256SUMS.txt` 一致，`BUILD-INFO.json` 已更新。
- 构建日志：`release/kanban/logs/build-20261031-233402-8168.log`。安装包已更新，仍未安装或完成另一台电脑的真实 SSH 验收。
