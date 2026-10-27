# 任务模型与思考强度验收

日期：2026-10-27。规格 [#35](https://github.com/legendxcheng/herdr-kanban/issues/35)，实施 #36–#40；基线 `28d49b41c6d8ea81c02146f6b65a47f9dd7c67b6`。

## 实现与数据边界

- 新建及安全可编辑的 Todo 支持任务模型/思考强度，默认字段不传 CLI 覆盖。已建立会话、绑定或待核实操作时配置只读。
- 最近一次成功创建的整组设置与任务在同一 SQLite 事务提交，跨 Workspace、All Tasks 和应用重启保留；失败/取消/编辑/删除不改此偏好。
- 目标执行环境提供模型目录，按 machine_id/cwd 缓存，可刷新；目录不是账户授权保证。继承的不支持组合保留原值并提示；启动前再校验，错误不自动降级。
- 受理启动时冻结配置，新建及精确恢复通过独立 argv 传递。保留进程直接重连；不注入聊天切换命令，不修改全局 Codex 配置；独立自动标题模型保持原配置。

## 自动检查

服务边界采用真实 SQLite 与可控 Herdr；覆盖创建/重开、原子失败、最近偏好、编辑锁、省略更新、配置冻结、失败不分配 pane、精确恢复不重发、手动派发限制、目录缓存/失败和目录隔离。实际 CLI 适配器另用可控子进程记录 argv，保持原有就绪、身份变化及明确拒绝回归。

前端使用真实共用表单和模拟 IPC，覆盖继承、编辑自身配置、dirty 保护、无效强度、跨执行项目迟到目录隔离，以及原有表单、语言、All Tasks 和引用回归。完整前端 33 文件 / 233 项通过；Rust 常规 222 项通过、23 项 ignored；TypeScript 检查及 `npm run build` 通过，保留既有大 chunk 提示。

在前后端全套同时运行时，Rust 六个既有后台时序用例触发 3 秒测试等待超时（`rust-full-concurrent.log`）；未改变产品或等待阈值，单独以 `--test-threads=4` 执行完整 Rust 套件全部通过。此前未新增最后一项目录回归时，默认线程全套 221 项也曾通过。

本地日志：`.scratch/task-model-spec/{frontend-full,rust-full,build}.log`。环境依赖测试在常规测试中保持 ignored，以下真实验收单独执行；不将模拟 IPC 结果视为真实模型生效证明。

## 真实 Windows 桌面

使用新编译 debug Tauri/WebView2、独立 `.scratch/model-acceptance-desktop/kanban.db`，通过界面实际操作并读取同一 SQLite 核对：

1. 新建默认 `gpt-6-luna/high`，改为 `low` 创建后任务及最近偏好同时为 `low`。
2. 再次新建改为 `high` 后取消，偏好仍为 `low`。
3. 退出并重开该测试进程，仍继承 `gpt-6-luna/low`。
4. Todo 修改强度触发离开保护，“继续编辑”保留草稿；保存任务为 `high`，最近新建偏好仍为 `low`。
5. 切换另一个 Workspace 及从汇总看板选择 Workspace 新建，均继承同一 `gpt-6-luna/low`。

结果和截图：`.scratch/model-acceptance-desktop/result.json`、`new-task.png`、`restarted.png`、`edit-saved.png`、`all-tasks-inheritance.png`；已查看截图。脚本位于 `.scratch/task-model-spec/{native_ui,cross_workspace}.py`。初次调试端口 9596 无监听；切换独立 profile/19837 后可用。早期脚本使用不匹配的精确按钮名称导致超时，修正定位后全流程通过。

## 真实 Herdr / Codex 本机与 SSH

使用 Herdr 0.9.5、Codex CLI 0.164.0，专用 Workspace/任务、独立数据库和一条不调用工具的最小提示词。实际新进程 argv 包含 `--model gpt-6-luna -c model_reasoning_effort="high"`；只读核对各自 Codex rollout 的首条 `turn_context`，模型均为 `gpt-6-luna`、effort 均为 `high`。

| 执行位置 | 精确会话 ID | 首次/恢复配置 | 完成再恢复后的派发次数 |
| --- | --- | --- | --- |
| 本机 | `01a17aac-46c6-7181-b3c7-b449d39f053d` | `gpt-6-luna/high` | 1 |
| SSH devpc | `01a17aae-7bee-7c41-8cca-a31d059a5d1e` | `gpt-6-luna/high` | 1 |

每侧均执行首次启动、完成、原会话精确恢复及再次完成；实际记录新建与恢复进程参数，原会话 ID 不变，恢复未再次发送业务提示词。SSH helper 通过既有内容寻址部署路径更新；未替换或重启用户 Herdr 服务，测试最后仅关闭自己创建的任务 pane。

可复用 opt-in 测试：`src-tauri/src/model_acceptance.rs`，需要显式隔离目录、可信 cwd 和可选已验证 SSH profile/helper 路径。证据分别位于 `.scratch/model-acceptance-local/` 和 `.scratch/model-acceptance-ssh/` 的 `fixture.json`、`session.txt`、`started-process.json`、`resumed-process.json`、`result.json`。本机首轮脚本过早读取延迟到达的会话 ID，调整等待后在原任务继续验收，无重发；SSH 一轮通过。

## 交付限制

已验证生产前端构建和 Windows debug 程序；未生成本次 NSIS 安装包、未安装或替换用户正在运行的应用。未逐个调用目录内所有模型验证账户权限，运行期无权限/CLI 错误仍由原生终端显示。测试截图及原始日志留在本机 scratch，不纳入 Git。
