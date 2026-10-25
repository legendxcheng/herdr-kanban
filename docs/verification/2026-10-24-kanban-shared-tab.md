# Kanban 共用 Herdr Tab 验证记录

日期：2026-10-24。

## 行为与范围

用户确认每个工作区共用一个 `kanban` Tab，任务继续独占 pane 和 Codex 会话。首次需要时创建 Tab，后续任务复用其中可用终端或分屏；自动分配不再搜索其他 Tab。旧任务的有效绑定保持原位置，正常完成关闭后再启动采用新规则。手动派发和绑定修复不改变。

`shared_tab.rs` 负责 Tab 查找和布局，`pane_allocation.rs` 负责同 workspace 分配互斥、创建清单持久化和不确定目标隔离。新增 `pane_allocations` 表以兼容方式初始化，原有任务、会话、绑定不迁移。名称歧义显示可读错误；创建响应丢失不通过 Tab 名猜测任务，不自动重复发送。

## 自动化检查

- `cargo fmt --check` 通过。
- `cargo test --offline --locked --lib`：80 项通过、6 项 opt-in live 测试默认跳过。包含新增的创建响应丢失、SQLite 文件重新打开后核实、创建回执落库失败、未决 A 不阻碍新任务 B、B 仍运行时核实 A，以及既有删除时迟到请求隔离。
- `npm test -- --run`：84 项前端测试通过。它们使用模拟 IPC，不代表原生桌面验收。
- `npm run build`：TypeScript 检查及生产构建通过，保留既有大于 500 kB chunk 提示。

以上基于包含其他未提交功能的共享工作区；计数不是仅本次变更的测试数。

## 真实 Herdr / Codex

环境：Windows，Herdr `0.8.0-preview.2026-08-04-d78e3d3b5126`。使用独立 workspace `kanban-acceptance-shared-90742aa4`（`w1A`），测试目录 `.scratch/kanban-shared-tab/acceptance`；任务存储使用独立临时 SQLite。没有操作个人工作 pane。

显式运行 `live_tasks_keep_distinct_sessions_through_completion_and_resume` 通过（29.67 秒），覆盖：

1. 两个任务通过真实 Workflow 并发启动，共用唯一 `kanban` Tab，pane ID 与会话 UUID 均不同。
2. 同目录的原始 shell 位于其他 Tab，未被自动占用。
3. 完成 A 后 B 的 pane 和共用 Tab 仍存在；完成 B 后共用 Tab 随最后一个 pane 关闭。
4. SQLite 关闭并重开后，任务在重新创建的 `kanban` Tab 恢复原会话；恢复不重发原提示词，完成后正常清理。

显式运行 `live_shared_tab_reuses_shell_and_rejects_duplicate_labels` 通过（1.18 秒），覆盖：

1. 仅共用 Tab 内的空闲 shell 成为自动分配候选。
2. 已有 Tab 时分屏生成不同 pane，Tab 数量不增长。
3. 存在两个同名 `kanban` Tab 时，查询候选及创建均报告歧义。
4. 关闭其中一个测试 pane 后另一个保留；全部关闭后 Tab 移除。

测试 Tab 和 Agent 已由测试清理，隔离 workspace 在完成验证后关闭。运行入口位于 `session_isolation_tests.rs` 和 `shared_tab.rs`；复跑需新建以 `kanban-acceptance-` 开头的空工作区并设置 `KANBAN_LIVE_WORKSPACE`、`KANBAN_LIVE_CWD`。

## 验证边界

本次真实验证到 Workflow、SQLite、Herdr 与 Codex 生命周期，没有执行新一轮原生 WebView2 拖放、终端输入与窗口切换验收。未重新打包、安装或重启用户正在运行的 Kanban，现有旧 Tab 不自动合并。不同数据库的多个应用实例同时操作同一 Herdr workspace、外部用户在分配期间移动布局，以及 Herdr 服务重启未覆盖。

创建响应丢失且出现无法归属的外部新增 pane 时采取保守核实，不自动采用该 pane；其他任务仍可创建独立 pane 继续启动。
