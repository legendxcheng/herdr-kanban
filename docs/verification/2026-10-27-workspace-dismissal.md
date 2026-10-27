# Workspace 创建记录移除验收（2026-10-27）

## 范围与实现

失败和待核实的创建记录提供“移除记录”，经应用内二次确认后从侧栏移除。确认框显示电脑与名称，默认取消获得焦点，支持 Esc/遮罩取消和焦点恢复；明确保留已有工作区、任务和 Herdr 进程。

`dismiss_workspace_operation → ProjectService::dismiss_operation` 只从 SQLite 解析所属电脑并使用对应创建/恢复互斥锁，不构造本地/SSH 执行适配器。取得锁后重新检查状态，只允许 failed/unknown；已成功或仍 pending 拒绝。schema 17 的 `workspace_operation_dismissals` 保存时间戳，原记录不删除，重复移除幂等。创建/核实检查移除标记，阻止旧操作 ID 再次写入外部资源。列表查询排除已移除记录；同步恢复普通候选发现，仍须显式“添加已有”才能显示新发现项目。

前端成功后记录已移除 ID，过滤本机与远端全部列表快照，防止慢请求重新显示旧条目；失败保留原条目，可重新确认。实际移除不关闭任务、终端或 Herdr 服务。

## 自动化验证

共享工作区同时存在 Herdr 启动、任务退回等其他进行中修改。首次共享验证曾因缺少进行中的 `herdr_startup_tests.rs` 阻塞，部分前端测试也受新增启动调用的模拟配置影响。因此以提交 `75fee484a96b4460c9183af8bad73cee10099433` 加本次文件/改动构造隔离副本，未撤销或包含其他功能。副本、构造脚本及清单位于 `.scratch/workspace-dismiss/`。

- `npm test`：34 个文件、238 项通过。新增 5 项覆盖确认取消/Esc/遮罩/焦点、重复点击与失败重试、pending 不提供移除、英文文案，以及离线立即移除和迟到快照过滤。
- `npm run build`：TypeScript 与 Vite 生产构建通过，保留既有大 chunk 提示。
- 桌面发现并修复遮罩点击抢走恢复焦点后，隔离副本及共享工作区的 3 个相关前端文件、22 项测试均通过；隔离副本生产构建再次通过。
- `cargo test --offline --locked --lib`：228 项通过，23 项环境相关测试忽略。新增 5 项覆盖绑定冲突保留已有 Project/Task/外部 Workspace、离线/无机器配置移除、并发成功后拒绝移除、写入失败回滚，以及旧库升级与文件关闭重开。
- 本次改动的 `git -c core.whitespace=cr-at-eol diff --check` 通过，沿用仓库 CRLF。

前端测试使用模拟 IPC，Rust 测试使用真实 SQLite 与 Herdr 替身；这些结果不等同于真实桌面或 SSH 验收。

## 桌面与交付边界

隔离副本构建新的 Windows debug 宿主，使用独立 APPDATA、KANBAN_DATA_DIR、WebView2 profile、前端端口与 `kanban-acceptance-workspace-dismiss` Herdr 会话。真实应用经 Tauri IPC 操作 SQLite 和本机 Herdr；专用数据中注入失败/未知操作记录，不接触个人项目。最终结果及中英文截图位于 `.scratch/workspace-dismiss/desktop-1793066552/`，脚本为 `.scratch/workspace-dismiss/desktop.py`。

- 创建真实 Herdr Workspace 并添加本地项目/任务，注入指向同一 Workspace 的 unknown 创建记录；点击核实，真实返回“该工作空间已关联其他项目”。
- 取消按钮、Esc、遮罩均保留记录，默认聚焦取消，取消后回到移除按钮；确认时背景 inert，取消后恢复。
- 确认移除冲突记录后侧栏立即消失，SQLite 项目和任务仍在，真实 Herdr Workspace 仍存在。
- 停用远端电脑的 unknown 记录通过纯本地 IPC 移除，英文确认框正确标明电脑；没有发起 SSH。
- 停止专用测试 Herdr 服务后仍能移除本机 failed 记录；关闭并重启测试应用，三个移除标记仍存在，原始三条创建记录和已有任务保留，两个电脑的待核实列表均为空。
- 中英文最终截图关闭过渡动画后检查，无文字溢出或按钮遮挡。

首次受限桌面启动未能开放 WebView2 调试端口，随后通过自动审批在独立桌面环境完成。首次真实取消检查发现遮罩 mousedown 的默认聚焦会覆盖焦点恢复，已在共用确认框中用 `preventDefault` 修正，并经新一轮真实操作通过。中途测试脚本曾误将 `server stop` 的文本响应按 JSON 解析，修正后完整流程通过。

本次未验收真实 SSH 连接上的服务操作、已有运行中 Agent 的生命周期或安装升级；远端覆盖的是停用电脑记录的本地移除。未生成新的安装包，未替换用户运行中的应用。测试结束后已关闭专用测试应用、前端服务与 Herdr 服务。
