# 后台任务状态与未知原因验证

日期：2026-10-31。规格为本次会话用户确认的范围：不打开任务即可获得状态；刷新不清空已有结果；查询失败明确过期，并区分绑定异常。未调整任务业务状态、优先级规则或增加通知/自动完成。

## 已实现

- 前台每三秒查询所有当前范围的 Workspace；焦点切换复用在途查询，不清空最近观察或反复丢弃有效响应。
- 新 `herdr_agent_snapshot` 返回已核验 Agent 与逐 pane 失败；原派发候选 IPC 不变。后台读取列表后，对未识别状态或进程检查失败的 Agent 复查一次，完全不打开/聚焦终端、不发送输入。
- 查询失败与超过 15 秒的观察不继续显示旧实时颜色。卡片明确区分查询中、查询失败、状态过期、绑定异常、会话未找到和未识别状态。上次状态仅作为过期历史提示。
- machine/session/workspace/pane/terminal/进程及任务绑定保护继续生效；查询失败不会自动修复绑定，也不会重发提示词。
- SSH 使用同版本 helper 的 `agent_status_targets` 只读方法；无数据库迁移。
- 状态专用 CLI 子查询限时 1 秒，沿用项目级并发池、单 Workspace 内顺序检查；不会改动启动、派发等操作原有超时。快照携带采样年龄，前端仍以 IPC 开始时间作为保守上界，SSH 返回/本机消息排队延迟不能把旧样本变成新鲜状态。

## 红绿回归

1. `npx vitest run src/board-tests/agent_status.test.tsx`：新增“焦点刷新保留未选中任务状态”在旧逻辑得到 unknown 而非 working，修改后通过。初次失败因未结束的测试 Promise 影响后续用例；最终测试全部结算请求。
2. 新增“查询失败标明过期和上次状态”：旧逻辑找不到“查询失败”，实现后通过。
3. 新增“反复切回窗口保留同一轮后台查询”：旧逻辑保持 working、丢掉 idle 响应，修复后通过。
4. 后端 `background_snapshot_keeps_per_pane_failure_distinct_from_missing_agent`：原行为返回零条失败，期望一条；接入带诊断的发现后通过。
5. CLI 边界 `background_discovery_recovers_unknown_list_entry_without_opening_or_focusing`：列表 unknown、单 Agent idle 时，原逻辑仍返回 unknown；有界复查后通过。命令替身仅接受 workspace list、agent list/get 和 pane process-info，其他调用会被记录为失败；另覆盖 working/done、持续 unknown、终端被替换与进程未知。
6. 审查后新增“成功→单 pane 失败→恢复”：先复现原状态历史丢失，再保留失败 pane 的过期历史，其他任务继续刷新。
7. 慢 pane CLI 替身每次进程查询阻塞 4 秒；状态专用查询两次各在 1 秒截止，同组正常 pane 返回 idle，整批采样年龄小于 5 秒。
8. “IPC 延迟 20 秒、服务端采样年龄 100ms”回归先复现旧 done 被当成新鲜完成；保守时间上界修复后保持过期。

## 当前验证结果

- 定向看板状态测试 21 项通过；含汇总看板的此前一轮 30 项通过；TypeScript 检查通过。
- 后端状态服务 5 项通过；CLI 边界矩阵通过。
- 完整前端：42 个文件、311 项测试通过；`npm run build` 通过（包含 TypeScript），保留现有大 chunk 提示。
- 最终 Rust 默认并发完整测试：`cargo test --offline --locked --lib --quiet`，314 项通过、34 项环境依赖测试跳过，18.20 秒。此前完整串行复核也为 314 通过、34 跳过。
- 中间实现尝试增加 Workspace 内 4 个 worker；默认并发完整测试两次在 `ucrtbase.dll` 以 `0xc0000409 / STATUS_STACK_BUFFER_OVERRUN` 退出。排除新增慢查询测试时通过，完整串行也通过。移除这层额外 worker、保留原项目级并发和 1 秒子请求截止后，最终默认并发全套通过，慢查询回归仍通过。没有把失败运行算作通过，也未声称已查明 CRT 异常的底层原因；最终代码不含该额外 worker。
- Standards 审查：0 项遗留问题；两项维护性意见（重复条件、IPC 导航）已处理。Spec 审查：单 pane 历史丢失与慢 pane 影响两项发现已修复并复核；追加延迟送达回归通过。
- 上述完整测试在共享工作区运行，已有的短多行派发改动保留但不纳入本次提交。转储只确认中间版本异常落在 `ucrtbase!abort`；旧构建符号已被后续构建覆盖，未据此推断具体应用代码根因。

## 真实本机只读验收

运行：`cargo test --offline --locked --lib herdr::agent_discovery::tests::live_background_task_statuses_without_viewer -- --ignored --exact --nocapture`。

测试以 SQLite 只读方式读取现有本机任务，使用真实 Herdr 0.9.5 和本次 Rust 查询实现；未打开终端，也未切换 pane 或发送输入。9 条绑定身份匹配的任务直接返回 8 条 idle、1 条 working。旧 contextSwitcher Workspace 缺失返回 `binding.workspace_ambiguous`，不会伪造为等待输入；另有旧任务进程已变化，未获得 verified_task_id。首次遍历遇到这个已缺失的历史 Workspace 中断，随后验收将明确的 Workspace 缺失单列报告，其他错误仍令测试失败。

## 尚未覆盖

- 截图中同一张卡片点击前后的现场复现；目前已复现的是清空状态与焦点使请求失效两条确定性路径。
- 真实 SSH 的新 helper 方法、断线恢复及同一任务状态转换。
- 新构建的原生 WebView2 界面、真实审批等待和未读 done 转换。
- 冷 helper 或很慢的路由首轮可能超过 15 秒而显示过期；后续继续轮询，不用较短的服务端采样年龄掩盖整条响应链的延迟。大量异常 pane 时也不承诺每三秒完成整批查询。
- 未打包、安装或替换用户运行中的应用。需要运行包含前后端改动的新构建后生效。
