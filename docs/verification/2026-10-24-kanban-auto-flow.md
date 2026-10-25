# Kanban 自动流转验证记录

日期：2026-10-24。规格：[GitHub #10](https://github.com/legendxcheng/ContextSwitcher/issues/10)。

## 已实现

- 拖入开发中自动分配同 workspace、同项目目录的可用 pane；没有则新建独立 tab。
- 首次执行发送已保存提示词；再次开发精确恢复 Codex 会话并等待输入。
- 完成按钮和拖入完成使用同一工作流：保存会话、关闭 pane、保留会话记录。
- 缺会话 ID 或关闭失败时卡片仍完成，保留目标并提供重试关闭。
- SQLite 持久化生命周期意图及会话，与原派发未知结果核实机制协作。
- 保留手动派发入口、既有终端布局、自动选择和其他未提交工作。

## 自动化结果

- Rust：28 通过，2 项既有 Herdr live 测试 ignored。
- 前端：40 通过；包含自动启动、重复拖拽、完成失败提示、恢复跳过旧 prompt 与手动入口兼容。
- 生产构建通过；仍有既有单 chunk 大于 500 kB 提示。
- 本次生命周期、Herdr 适配、Workflow 与 IPC 文件的 Rust 格式检查通过；后续共享工作区出现其他任务的持久化模块编辑，不将其计入本次验证结论。
- 服务测试以真实 SQLite、Herdr 替身验证对外行为。持久化用例导出临时数据库、关闭旧连接、重新打开数据库并执行迁移/恢复后恢复原会话。
- 并发跨项目任务获得独占 pane；缺 ID 不关闭；替换进程不误关；关闭超时后可启动其他任务；核实派发和恢复落库错误均不重发 prompt。
- 延迟恢复请求无法覆盖新操作预占的 pane。

## 静态能力核对与未验收项

本机 Codex 0.164.0，Herdr 0.8.0-preview.2026-08-04-d78e3d3b5126。读取 CLI help 与 bundled schema 确认 tab create、agent start 原生参数透传、pane close、agent_session 和 codex resume 参数。协议中 agent_session 可为空，这不证明运行时已配置正确上报。

本次没有在真实 Herdr 内创建、关闭或恢复测试 pane；没有对用户正在工作的 pane 发送探针。当前 Agent 不在 Herdr 托管 pane，未伪造 HERDR_ENV。测试替身和前端构建不能替代以下验收：

1. Windows 空闲 shell 的 foreground process group 与 shell PID 判定。
2. 空闲 Codex `/quit` 后回到 shell、旧上下文隔离、启动参数及默认权限保留。
3. 新会话及 resume 后 agent_session 上报的来源、延迟和 UUID 是否正确。
4. 实际完成 pane 消失；重新启动应用后恢复原对话且没有再次发送历史任务。
5. 真正的 Herdr 断开/重启、Codex 审批弹窗、工作中完成以及原生拖拽/终端切换。

若 agent_session 不上报，当前实现会保留 pane 并提示检查 Herdr Codex 集成；不会改写用户全局 Codex hooks/notify 或猜测最近会话。该环境依赖必须在真实验收中确认。
