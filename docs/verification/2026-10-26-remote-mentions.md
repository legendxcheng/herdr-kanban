# SSH 项目提示词引用搜索验收（2026-10-26）

规格：[GitHub #21](https://github.com/legendxcheng/herdr-kanban/issues/21)，由用户调用 to-spec 发布，标签 ready-for-agent。用户已确认开发及真实表单、专用任务派发的验收范围。

## 发现与修改边界

- 用户报告位置为看板新建/编辑任务提示词框，不是右侧终端的原生 Codex 输入框。
- 当前代码已有 Project → 执行电脑 → SSH 辅助程序的远端文件/技能查询。本次隔离桌面副本读取真实 fish-oasis 项目时，`@AGENTS` 约 2.38 秒、`$oasis-qa` 约 2.88 秒返回且可选择；没有复现持续不可用，不能把等待缺乏提示认定为用户现场唯一根因。
- 已确认原组件将等待、空结果和异常都隐藏为没有候选框，远端技能每次输入立即查询。修改集中在候选控制器、独立反馈样式和中英文文案。
- 新增搜索中、无匹配、错误原因和原地重试。远端 @/$ 使用 120ms 防抖，同一编辑器内按 Project 串行查询、跳过过时队列项；项目切换和本机搜索不受旧项目挂起影响。关闭或改词后忽略旧响应/错误。缺少文件搜索目录时提供提示。
- 不修改 Rust、IPC、数据库或引用格式；技能仍发原生 `$技能名`，文件仍从所属执行电脑读取正文。没有将计划或测试结果写入其他功能的已有改动。

## 自动检查

- `npm.cmd test -- src/remoteMentions.test.tsx`：修改前 6 项中 5 项失败，捕获缺少状态反馈、空结果、目录提示和远端连续查询未合并；修改后 6 项通过。
- `npm.cmd test -- src/remoteMentions.test.tsx src/fileDiscovery.test.tsx src/skillDiscovery.test.tsx`：12 项通过。
- 新回归使用真实 TaskFormDialog/PromptEditor/Tiptap，通过 DOM 粘贴、键盘/鼠标选择、表单保存及重挂载验证引用；仅 IPC 为替身。覆盖失败重试、文件与技能保存/展开、同路径不同项目、远端未返回时切换本机、过时错误、关闭后迟到响应及最新查询合并。
- 首次默认并发全量运行出现一项保存失败后焦点断言失败，以及 completed_bulk 测试 worker 原生退出码 3221225477。没有将该轮记为通过；随后 `npm.cmd test -- --maxWorkers=2`：28 文件、167 项全部通过。
- 收尾时共享工作区已有其他任务新增回归，最终同命令重跑为 28 文件、169 项全部通过；生产构建再次通过。最后仅更新本验收记录。
- `npm.cmd run build`：类型检查和生产构建通过，保留原有大 chunk 提示。
- 本次没有后端改动，未重复运行 Rust 全量；真实 IPC/SSH 路由通过以下桌面验证覆盖。

## 真实桌面与远端执行

使用当前 debug 二进制的独立副本、独立 WebView2 数据目录以及用户数据库的 SQLite backup 副本，运行当前 Vite 前端；没有替换或重启用户原应用。测试数据位于 `.scratch/remote-mentions/`。

1. 新建表单实际输入 @ 和 $：等待状态可见，真实远端文件/技能候选可选；Enter、Tab 和鼠标选择通过。
2. 两种引用由实际表单保存，经页面重载后在 Todo 编辑面板读回。完成后的编辑表单再次搜索技能并用 Enter 选择通过，未保存的测试编辑已取消。
3. 在复制数据库中临时将对应机器 helper_name 置空，触发真实后端的 machine.setup_required。@ 和 $ 均显示“搜索失败 / 请先完成这台电脑的接入”，草稿仍在。恢复原值后点击重试，实际 SSH 搜索成功；脚本 finally 复核恢复配置。这是受控配置故障，不是整机断网验收。
4. 900×600 WebView 视口中错误框及重试按钮在边界内；中英文空结果、Escape 只关闭候选并保留表单、连续输入后取得当前远端匹配均通过。截图已人工查看。
5. 专用远端 Workspace `kanban-acceptance-selection-remote-337347` / `w6`，任务 `23f4db02-8ffa-4cbc-8316-0e90f842987b`，派发操作 `e599ff3e-73e9-495c-a19f-a747b1647695`，实际 pane `w6:p3`。
6. 任务快照包含远端文件 `REMOTE-REFERENCE-7c92` 与原生 `$kanban-acceptance`，没有展开 SKILL.md 正文。远端输出 `mentions-result-344fcd40.txt` 为 `REMOTE-REFERENCE-7c92 / SKILL-READ / NO-IMAGE`。会话 `01a17611-77d7-7c51-b2af-8b818fb126d7`；正式完成接口成功、warning 为 null。
7. 收尾只读数据库核对：该任务 done、sent 派发记录恰好 1 条、绑定 0 条、helper 元数据已恢复；只关闭本次隔离应用副本，保留证据及远端验收产出。

证据：`.scratch/remote-mentions/live.py`、`live-result.json`、`feedback.py`、`feedback-result.json`、`before-file.png`、`before-skill.png`、`after-file.png`、`after-skill.png`、`reopened.png`、`edit-search.png`、`failure-file.png`、`failure-skill.png`、`empty.png`。

## 保留边界

- 原始现场持续无法补全未复现；当前真实远端搜索可用，已修复能够确认的反馈和查询调度缺口。
- 自动用例验证请求合并次数，真实桌面验证连续输入及最终候选；没有把真实桌面结果计作精确 SSH 子进程计数测试。
- 没有测试整机断网、远端服务重启或 Linux/macOS；没有重建安装包或执行安装升级。
- 队列不能取消已开始的 SSH 查询；下一次同项目查询可能等待当前查询结束，期间显示搜索中。不同编辑器之间没有共享缓存或全局串行限制。
