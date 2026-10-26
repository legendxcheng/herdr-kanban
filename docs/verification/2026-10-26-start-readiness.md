# 新任务复用 Codex 时的启动就绪修正

日期：2026-10-26。延续远端启动修正，用户报告新版本仍出现 `The pane is not ready or is working; launch was stopped`。用户明确选择“保留现状，只修版本”：两张业务失败卡片、绑定和远端进程均不修改，不重发其提示词。

## 用户失败证据

- 两次启动：17:12:48 和 17:13:33 受理；后台分别 8.690、8.867 秒返回 `flow.not_idle`，最后阶段均为 starting_agent，没有任何业务派发记录。
- 两次都选择已有空闲 Codex 的 pane。只读查询显示同一 terminal 和 shell 下已有新进程、interactive_ready=true；新 Agent 名称分别对应本次操作的 `kb-<operation-prefix>`，说明新启动已发生，失败不能解释为“尚未启动 Agent”。
- 原实现把 `agent.start` 命令成功与首次查询立即交互就绪等同。新增 CLI 边界回归让成功启动后的前两次查询返回 launch_pending / 非就绪，旧实现稳定返回同样的 `flow.not_idle`：`cargo test --lib successful_launch_waits_for_the_same_codex -- --nocapture` 修正前失败，修正后通过。
- 前一轮性能验收预占所有旧 pane，验证的是新建 pane 路径，没有覆盖本次复用已有 Codex 的场景；此前结论不代表复用路径已通过。

## 修正

成功启动后只读等待同一窗口、原 shell 和新 Codex 进程达到交互就绪。等待不再次调用 agent.start，不发送任务提示词；每轮检查任务有效性，观察到新 Codex 后固定进程身份，换 terminal/进程/其他 Agent 拒绝继续。仍保留有界超时和 unknown 保护。

## 旧 Agent 注册清理的第二个时序问题

真实当前安装版在同一项目目录复用时再次捕获 `herdr.invalid_response`。隔离记录器保留的末尾请求顺序为：pane get 仍返回 agent=codex / unknown；agent get 返回同一 workspace/pane/terminal、idle 和 interactive_ready，但已无 agent 字段；process-info 返回 shell_pid=foreground_process_group_id。CLI 都成功，错误来自 Kanban 把后续缺失的 agent 字段当作无效回复。

以捕获形态新增 `pane_agent_cleared_between_reads...` CLI 回归，旧逻辑稳定返回相同 `herdr.invalid_response`；修正后先核对两份读取的目标身份，再让已无 Agent 的快照通过进程检查证明 shell 就绪。另有替换 terminal 拒绝回归。没有把任意 JSON/网络错误当成可忽略的过渡状态。

## 验证结果

- Rust 全量：211 passed / 20 ignored。延迟就绪、注册清理两条回归分别由红转绿，等待或交接时替换 terminal 的回归拒绝继续；已有 busy 明确拒绝重试、未知结果不重发、派发回滚和删除守卫通过。
- 真实正式版、独立数据库、专用 `kanban-acceptance-reuse-*` Workspace：先启动并完成验收提示词，然后删除测试卡片（保留 pane），让下一张测试卡复用同一 pane 的旧 Codex。未使用业务卡片作测试。
- 当前安装版在独立验收目录四次成功；在实际项目工作目录中前两次成功，第三次复用捕获 `herdr.invalid_response` 且派发数为 0。带只读元数据记录的复测在第五次启动再次捕获同一失败，取得上述注册清理证据，不能称旧版整套验证通过。
- 首轮修正版在实际项目目录完成 4 次成功（首次新建、随后 3 次同 pane/terminal、不同 Codex 进程），耗时 12.597、16.986、17.491、14.873 秒；每次仅一条 sent 派发，原生终端连接成功。测试提示词明确禁止工具和文件读写。专用 Workspace 与隔离应用已清理。

诊断脚本与原始记录位于 `.scratch/start-latency/failure/`。额外 SSH 记录器仅用于隔离诊断，记录命令类别、就绪状态和进程身份，不记录提示词内容；不加入产品代码。

### 最终组合修正版的真实验收

最终正式构建在实际项目目录中，通过独立数据库和专用 Workspace 连续启动 8 次；不带额外 SSH 记录器。首次新建终端，随后 7 次复用相同 pane/terminal，每次获得不同的 Codex 进程身份。8 次生命周期均 succeeded，每次恰有一条 sent 派发，8 次原生 Herdr 视图均连接成功。耗时从调用 start_task 到查询到生命周期完成计量，不等同于模型完成回答或视图显示耗时。

| 轮次 | 场景 | 启动完成耗时（秒） |
| --- | --- | --- |
| 1 | 新建 | 11.608 |
| 2 | 复用 | 16.260 |
| 3 | 复用 | 15.307 |
| 4 | 复用 | 16.965 |
| 5 | 复用 | 15.402 |
| 6 | 复用 | 14.693 |
| 7 | 复用 | 14.238 |
| 8 | 复用 | 15.286 |

证据：`.scratch/start-latency/failure/reuse-fixed-project-final/result.json`、同目录 `data/start-timing.jsonl`。测试卡片、专用远端 Workspace 和隔离应用均已清理；未用业务卡片或其 pane 作验收。该样本验证本次两条时序修正，不代表所有网络环境、恢复会话或后续模型响应性能均已覆盖。

按用户要求，本次工具未关闭两张业务失败卡片的远端进程、解除其状态或重发提示词。验收中途只读比较确认原卡片、生命周期和零派发记录均与初始快照一致，记录于 `preserved.json`；结束时主应用已有外部变更，原 task 行已不存在、原 unknown 生命周期仍保留，因此不把最终业务数据库描述为完全未变化，也不恢复或覆盖这些外部操作。

### 构建与交付

- TypeScript/Vite、Rust release、NSIS 构建成功；日志 `release/kanban/logs/build-20261026-173609-62444.log`。Rust 全量结果为 211 passed / 20 ignored；上面的真实验收单独执行，不以忽略测试计作通过。
- 实测正式 exe SHA256：`9b6fdca27aca8e9c95f4b2665f480c610d236d22d12c618fdd235f3ee2a8997c`。
- 安装包：[Herdr-Kanban-0.1.0-start-readiness-fix-windows-x64-setup.exe](../../release/kanban/0.1.0/Herdr-Kanban-0.1.0-start-readiness-fix-windows-x64-setup.exe)，与构建脚本的标准名称安装包内容一致，另存文件名便于区分之前同版本号的包。
- 安装包 SHA256：`1ccd98f045ad5606ad6138eb36b6df1e0498b953de9a9081a88788549aa96ddd`。本次未通过工具安装覆盖用户正在使用的应用，也未执行 Git 提交或推送。
