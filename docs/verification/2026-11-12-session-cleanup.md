# Session 资源跟踪与全局清理验证

规格：[GitHub #53](https://github.com/legendxcheng/herdr-kanban/issues/53)，子任务 #54–#60。

实现已进入审查与验收阶段，尚不能将本记录视为完整资源循环验收通过。

## 已执行

- TypeScript 检查、生产前端构建通过。保留既有大 chunk 提示。
- Session 清理专项 13 项通过；生命周期专项此前 103 项通过、2 项手动验收跳过，后续完整回归结果待写入。
- 隔离 SQLite 重开证明关闭回执丢失后只核实原目标，不重放关闭。
- 隔离真实 Tauri 桌面同时预览本机和 SSH session，InProgress 会话被保留，取消无关闭行为；截图及日志在 `.scratch/session-lifecycle-implementation/live/`。
- 真实资源循环已完成本机／SSH 各 2 轮：无绑定 Agent、完成任务。每轮目标 Pane 和已采样进程归零，InProgress 和普通 shell 对照保留。第三轮在验收脚本启动 Agent 时超时，已只读确认目标存在；尚未把该轮计为通过。

## 证据与边界

测试仅操作专用 `kanban-cleanup-20261112` Herdr session 和 `.scratch/session-lifecycle-implementation/live/kanban.db`。生产任务库与用户默认 session 未用于清理。测试辅助 SSH profile 专用于该 session。

循环经真实桌面按钮及正式 IPC 进入真实工作流、SQLite、Herdr 和远端 helper；任务归属由隔离 SQL fixture 建立，Agent 由真实 Herdr 启动。该证据不等同于首条提示词完整启动链路或真实对话恢复验收。独立进程观察使用本机 psutil 及 SSH Get-Process；资源证据中的进程创建时间来自正式关闭路径。

全量回归、最终版本桌面复测、20＋20 轮资源统计及代码双轴审查仍在进行。未打包安装或发布。
