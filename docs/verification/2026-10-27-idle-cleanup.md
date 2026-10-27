# 新建 pane 与确认清理验收（2026-10-27）

规格：[GitHub #41](https://github.com/legendxcheng/herdr-kanban/issues/41)。实现以 `a62854f5` 为基线，提交仅包含本功能；工作区已有的任务重置、本机服务启动等改动未纳入。

## 最终行为

- 自动启动新任务始终在所属电脑/session/Workspace 的唯一 `kanban` Tab 创建新 pane；不复用旧空闲 shell/Codex。保留有效绑定及精确恢复、不重发和未知启动保护。
- 单 Workspace 的 NewTask 旁增加清理小按钮；All Tasks 不提供该入口。先预览，再显示电脑、Workspace、范围及数量，默认聚焦取消。取消/Escape 不发送关闭，重复确认不重放。
- 后端排除所有 InProgress 绑定，包括 idle/done、等待用户输入的 Codex；同时保护 reserved、未决生命周期/派发/创建及清理操作。执行再次核对 Tab、terminal/进程身份和实时空闲状态。
- 非 InProgress 历史绑定必须先保存精确会话及目录；只有确认整个 session 中原 terminal 消失，才释放匹配绑定，任务状态保持。关闭未知结果写本地日志，可重开弹窗核实，但不重发关闭。
- 普通 shell 全部保留。当前 Herdr API 不能可靠证明 shell 的命令已经结束：即使没有子进程，PowerShell `Start-Sleep` / `Read-Host` 和 cmd 内置命令仍可能运行。中英文弹窗明确说明此限制；不使用屏幕文字或 PID 相等替代证据。

## 自动化验证

使用只含本次待提交内容的独立导出 `.scratch/issue41/isolated/`，排除共享工作区其他未提交代码的依赖。

| 检查 | 结果 |
| --- | --- |
| TypeScript 与生产前端构建 | 通过，保留已有大 chunk 提示 |
| 独立快照 Rust release 内嵌前端构建 | `cargo build --release --features tauri/custom-protocol --offline --locked` 通过 |
| 前端全量测试 | 35 文件、243 项通过 |
| Rust 全量 `cargo test --lib --offline --locked` | 238 项通过、24 项环境依赖/真实测试默认忽略 |
| 新增清理行为测试 | 真实 SQLite + Herdr 边界，包含固定快照、晚到 InProgress 绑定、身份/Tab 变化、重复确认、unknown 跨数据库重开、历史绑定保存及未知关闭阻止重启 |
| 实际 CLI 适配器测试 | 单个 pane 失效不阻断其余候选；shell、working、blocked、unknown、launch_pending 均排除 |
| 清理按钮组件测试 | 二次确认、默认取消、Escape、零候选、重复提交、All Tasks 无入口、切项目丢弃迟到响应、历史 unknown 只读核实 |

TDD 记录：新建策略、清理快照/保护、unknown 重开、数据库查询错误和失效单 pane 均观察到失败后修正。全量测试发现的旧绑定 JSON 兼容问题改为直接按清理日志中的任务归属检查，归档回归恢复；自动分配减少一次候选查询后，删除期间副作用测试的读取屏障相应调整。

一次并发高负载的前端全量运行出现已有 Workspace 隐藏测试 `root.inert` 时序失败；该文件在本次快照和基线上各自 8 项单跑通过，限制为 4 workers 的最终全量 243 项通过。独立 Rust 构建曾与尚未退出的真实测试争用同名 Windows 可执行文件，待该进程完成后重跑；最终结果如表。不把这些中途失败记为通过。

## 真实 Workflow、Herdr 与桌面

两侧均使用独立 SQLite、专用 Workspace、无业务内容的测试任务。测试提示词只要求回复固定文本，不执行工具或修改文件。每侧预置普通 shell、停在 `cmd /c pause` 的 shell、执行 `Start-Sleep -Seconds 180` 的 PowerShell、未绑定空闲 Codex、其他 Tab，以及两张 InProgress 且已等待输入的 Codex 卡片。

| 路径 | 最终 Workspace | 结果 |
| --- | --- | --- |
| 本机 Workflow + SQLite + Herdr | `w2A` / `kanban-acceptance-issue41-f508c386` | 51.34 秒通过；两任务获得不同新 terminal；清理仅关闭未绑定空闲 Codex；受保护对象存活 |
| SSH `devpc` 同路径 | `wP` / `kanban-acceptance-issue41-6740f0d4` | 75.71 秒通过；远端 helper 在所属机器核验，结果同本机 |
| 本机真实 Tauri/WebView2 | 同上本机 Workspace | Windows UI Automation 调用真实按钮；默认焦点取消，取消后仍为 1 个候选；确认结果关闭 1、跳过 0、失败 0 |
| SSH 项目的真实桌面入口 | 同上远端 Workspace | 同样验证范围、默认取消、取消不关闭、确认仅关闭 1 个未绑定 Codex |

桌面测试使用带内嵌生产前端的可执行文件、真实 Tauri IPC 和实际 Herdr，不注入模拟 IPC。窗口未在前台时系统拒绝了早期坐标点击/焦点检查；调整为激活测试窗口后使用原生 UI Automation，最终两侧通过。测试资源都属于本次隔离 fixture。

桌面完成后另核对真实 pane 清单及 SQLite：两个 InProgress 任务的有效绑定仍在，每任务仍只有一次派发；普通 shell、pause、进程内命令所在 pane 均在；桌面候选 terminal 已不存在；每库两条确认关闭记录（Workflow 一条、桌面一条），无 claimed/closing/unknown 残留。日志、截图与逐项清单位于 `.scratch/issue41/` 和 `.scratch/issue41-acceptance-{local,ssh}-final/`。

核对完成后，按 fixture 中的稳定 ID 和完整专用名称二次匹配，关闭本轮及中途创建的测试 Workspace（本机 `w27`、`w28`、`w29`、`w2A`，远端 `wN`、`wP`）；未使用用户 Workspace 做清理测试，也未关闭用户业务会话。

## Standards

双轴审查没有发现文档规范硬性违规。两条非阻塞建议中，资格判定已从 `owner` 改名为 `eligible_owner`；清理状态仍沿用本仓库 SQL/Rust 字符串方式，未额外重构整个生命周期类型体系。

## Spec

初审发现普通 shell 误判风险和单个失效 pane 阻断预览，均修复并复核。补充检查了清理占用对任务重启、派发和绑定变更的保护；剩余阻塞发现 0。规范轴 0 条硬性违规、1 条保留的非阻塞类型建议；规格轴 0 条未解决发现。

## 边界

- 普通 shell 的可靠空闲识别需要上游提供额外证据；本版本保守保留。
- 就近核验与关闭之间仍可能发生外部 Herdr 操作；没有跨进程原子关闭保证。
- unknown/断线、数据库重开和精确恢复不重发主要由自动化回归验证；本次未人为切断真实 SSH，也未重跑真实精确 resume 的全部历史验收。
- 新建 pane 的真实验收从 Workflow 入口执行；桌面验收集中于清理按钮与确认流程。未执行安装包安装、未替换用户运行中的应用，未修复或重试用户原失败卡片。
