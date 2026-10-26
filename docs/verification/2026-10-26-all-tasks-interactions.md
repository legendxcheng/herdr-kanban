# All Tasks 页内交互验收（2026-10-26）

对应 [规格 #26](https://github.com/legendxcheng/herdr-kanban/issues/26) 与子任务 #27–#34。普通看板与 All Tasks 共用 `TaskBoard`；跨项目操作保留任务真实 Project 和电脑身份，编辑、终端和生命周期操作都在汇总页完成。

## 自动化验证

| 层级 | 命令／证据 | 结果 |
| --- | --- | --- |
| 完整前端回归 | `node_modules\.bin\vitest.cmd run --maxWorkers=2` | 32 个文件、229 项通过 |
| 类型检查 | `node_modules\.bin\tsc.cmd --noEmit` | 通过 |
| 真实 SQLite 汇总投影 | `cargo test --offline --locked --manifest-path src-tauri/Cargo.toml task_summary::` | 3 项通过；完整交互记录与普通列表在生命周期状态下逐字段一致 |
| 任务持久化与批量操作 | 同上，过滤 `tasks::` | 10 项通过，包括重开数据库、删除级联、归档限制、快照范围及逐项失败 |
| Rust 格式 | 对 `tasks.rs`、`commands.rs`、`task_summary_tests.rs` 执行 `rustfmt --check --edition 2024` | 通过 |
| 桌面开发构建 | `node_modules\.bin\tauri.cmd build --debug --no-bundle`，`CARGO_BUILD_JOBS=2` | TypeScript、Vite 与 Windows Tauri 开发可执行文件构建成功 |

提交前将仅含本次 23 个文件／改动块的暂存树导出到 `.scratch/all-tasks-staged-verify`，排除共享工作区其他未提交代码。在该副本再次执行完整前端回归（229 项通过）、`npm run build`（类型检查和生产前端构建通过）及完整 Rust 常规测试 `cargo test --offline --locked --manifest-path src-tauri/Cargo.toml --lib`（202 项通过、19 项需要专用环境的测试按默认忽略）。原生验收单列如下，不将这些忽略项计为真实环境通过。

前端测试在 App／Tauri IPC 边界模拟服务，不能代替原生终端验收。新增 `summary_interactions.test.tsx` 14 项与迁移后的 `summary.test.tsx` 13 项覆盖：页内编辑及保存、草稿保护、正确项目派发、新建归属、启动／完成、跨机器状态隔离、未知结果不重发、删除防复活、隐藏项目、跨项目批量部分失败、旧范围响应、保存前旧轮询、慢读取期间点击、远端查询容量耗尽及迟到派发准备。

先验证失败再实现／修复。完整回归曾发现中英文占位符顺序不一致，修正后完整重跑为 229 项通过。Windows 回归使用仓库既有 `--maxWorkers=2` 约定。默认 release 编译遇到 `cl.exe 0xc0000005`；本次成功产物为 debug，不作为发行安装包验收。

## 双轴代码审查

- Standards：修复旧范围回调污染新范围，以及远端查询占满导致本机状态读取等待的问题；复审无遗留阻断项。
- Spec：修复保存后派发读取旧提示词、轮询使等待中的点击失效，以及切换任务后迟到结果打开旧派发弹窗的问题；复审无遗留阻断项。

所有修复都有对应先失败后通过的行为测试。范围、读取代次和交互选择代次分别保护查询结果与用户意图；本机、远端 Agent 查询使用独立容量。

## 真实 Tauri／WebView2 与本地数据库

使用独立 `KANBAN_DATA_DIR`、独立 WebView2 用户目录和专用测试进程，不替换已安装应用。通过 CDP 操作真实 WebView2 控件，保存和变更走实际 Tauri IPC；SQLite 直接查询用于核对持久化。离线夹具不连接真实 Herdr。

| 操作 | 实际结果 |
| --- | --- |
| 打开远端 Todo，修改标题并保存 | 右侧编辑器保持打开、All Tasks 保持选中；数据库标题已更新 |
| 跨任务切换／隐藏所属 Workspace | 显示草稿确认；继续编辑保留输入；放弃后才切换 |
| 新建 | 先选择远端 Workspace；持久化 `project_id` 正确 |
| 拖拽完成任务到待办 | 实际拖拽事件、卡片列和 SQLite 状态一致 |
| 跨两个 Workspace 批量归档 | 两项成功、卡片移除，数据库归档时间已写入 |
| 单项删除 | 原确认对话框、卡片移除，数据库记录删除 |
| 1280px／900px 窗口、中英文 | 已检查截图；无根页面横向溢出，窄窗口沿用看板内部横向滚动 |

键盘触发与普通看板交互继续由前端回归覆盖。此轮真实桌面用鼠标操作上述流程，未单独执行输入法组合键验收。

本地证据：`.scratch/all-tasks-native/result.json`、`all-tasks-1280.png`、`all-tasks-900.png`、`all-tasks-english.png`；脚本 `.scratch/all-tasks-native.py`。这些是本机验收产物，不作为仓库公共附件。

## 真实本机／SSH Herdr

使用新建专用 Workspace：本机 `kanban-acceptance-all-tasks-7982f19-local`（`w24`），SSH 电脑 devpc 上 `kanban-acceptance-all-tasks-7982f19-remote`（`wH`）。仅测试三条专用任务，提示词只要求回复标记，禁止执行命令或修改文件。

| 任务 | 目标 | 标记 |
| --- | --- | --- |
| local_a | 本机 `w24:p2` | `SUMMARY_7982F19_LOCAL_A` |
| local_b | 本机 `w24:p3` | `SUMMARY_7982F19_LOCAL_B` |
| remote_a | devpc `wH:p2` | `SUMMARY_7982F19_REMOTE_A` |

三项均从 All Tasks 通过实际拖拽进入开发中。本机两项正常完成启动；远端首轮停留在 `dispatch.verify_required: flow.not_idle`，终端中提示词仍在输入框，未自动重发。只读确认目标后，对已有输入按一次 Enter；远端返回对应标记后，通过 All Tasks 的“核实派发结果”填写证据并确认已发送，恢复为有效绑定。不能把这次人工恢复计作远端自动提交一次成功。

原生输入通过所选专用 WezTerm 客户端的真实 socket 写入 `INPUT_7982F19_LOCAL_B`／`INPUT_7982F19_REMOTE_A`，读取终端内容确认后清空未提交输入；没有调用模拟 IPC 或重新派发任务。该项验证实际终端输入传输与目标选择，不代表人工键盘／输入法验收。

按 local_b → remote_a → local_a → remote_a → local_b 完成五次页内切换：原生终端均出现对应标记且未出现另外两项任务标记，电脑身份正确，All Tasks 始终选中。远端终端最大化／恢复成功。切换前后的 `dispatched_at`、启动 `operation_id` 与绑定目标保持一致，未重发或新启动任务。

最后逐条点击“完成”，三项均持久化为 `done` 且 `cleanup_required=false`。本机和远端测试进程使用独立数据库与专用 Workspace；安装中的 Kanban 未被关闭或替换。

本地证据：`.scratch/all-tasks-live/result.json`、`fixture.json`、`local_b-native-input.txt`、`remote_a-native-input.txt`；脚本 `.scratch/all-tasks-live.py`。`fixture.json` 保存真实任务、电脑、Workspace、pane 与原生输入验证记录。全部完成后逐一核对 Workspace 名称、ID、允许窗格集合和空闲状态，两个专用 Workspace 已关闭；记录为 `cleanup.json`。远端完成后仍有测试 pane，因此最终通过专用 Workspace 清理回收，没有将数据库 `cleanup_required=false` 等同于所有远端进程已经退出。

## 边界

构建和桌面验收在保留工作区既有开发改动的条件下执行，包括另一条 Herdr 启动／远端链路开发；本次提交仅包含 All Tasks 相关改动。没有数据库迁移。没有构建正式安装包、安装／替换用户应用或推送远端仓库。

图片、文件和技能引用沿用已有表单及派发路径，前端现有回归覆盖其组件行为；本次新增真实 Herdr 提示词为纯文字，没有另做远端图片上传和交互式 SSH 认证验收。
