# Todo 右侧编辑面板验收（2026-10-26）

## 行为

- Todo 卡片点击、标题 Enter/Space、铅笔入口均在原右侧 Herdr 区域打开 TaskEditPanel，选中卡片高亮；保留可拖动宽度。
- 标题、提示词、优先级复用 TaskForm，图片、文件/技能引用继续使用原 PromptEditor。保存后保持打开，后台刷新不替换草稿或抢走选择。
- 有修改时，切换任务、关闭面板、切换/隐藏当前 Workspace、启动或派发前，可继续编辑、放弃或保存后继续。保存失败停留原任务，图片导入和保存进行中不切走。
- 进入 Todo 时卸载终端查看组件，返回进行中任务重新连接。仅关闭查看连接，不完成任务、不停止底层 pane、不自动发送提示词。
- 新建及其他状态任务编辑保留居中弹窗。未保存草稿仍只存在内存，不提供应用退出/刷新后的草稿恢复。

## 自动检查

当前共享工作区包含并行进行的侧栏及引用搜索变更，以下数量代表当次完整快照，不全属于本功能：

- 前端 28 个测试文件、169 项用例全部通过。一次运行中 Windows Vitest worker 以 3221225477 异常退出；重跑完整套件通过，没有把异常运行记作通过。
- 新增 `src/board-tests/todo_editor.test.tsx` 的 9 项回归覆盖：保存并保持面板、键盘选择、刷新保留草稿、取消/放弃/保存后切换、保存失败、终端连接释放与返回、不误派发、拖入开发中前保存、保存期间保护、Workspace 切换、选中任务删除、图片导入与连续保存租约、概况导航后的 inert 清理。
- 更新原任务表单和自动标题测试，使 Todo 铅笔入口断言右侧面板；仍验证不覆盖后台生成标题。
- TypeScript/Vite 生产构建通过；保留既有大于 500 kB 的 chunk 提示。
- `git -c core.whitespace=cr-at-eol diff --check` 通过；保留仓库现有 CRLF 文件格式。
- 本次不修改 Rust 或数据库结构，未新增后端测试。

## 浏览器交互和布局

真实 Edge + React/Tiptap，使用 `.scratch/todo-editor/index.html` 的隔离模拟 IPC 数据，未访问用户任务数据库：

- 点击 Todo 后显示右侧表单，编辑标题、保存并切换后可读回。
- 未保存切换确认、继续编辑、放弃修改、切回进行中界面通过。
- 中英切换保留未保存标题，确认按钮与文案随语言更新。
- 1280×800 布局截图正常。900×600 CSS viewport 下右侧面板 x=600、width=300、height=600；底栏 bottom=600，保存按钮可见，正文独立滚动。
- 截图：`.scratch/todo-editor/browser-1280.jpg`、`browser-900.jpg`。浏览器尺寸覆盖已恢复。

## 原生桌面限制

尝试使用独立 `KANBAN_DATA_DIR`、WebView 用户目录和 `kanban-acceptance-todo-editor` Herdr session 验收，没有重启用户 default server 或替换正在运行的应用。

测试会话拒绝后续客户端访问，返回 elevated server / administrator terminal / Access denied；未启用放宽客户端权限的启动选项。测试 Tauri 副本实际启动并显示界面，但 WebView2 未提供可连接的验收调试端点，因此未完成真实 SQLite 保存读回、原生输入及 Herdr 窗口遮挡/恢复验收。不能以浏览器模拟结果或既往原生验收代替本次结果。

验收副本已关闭；清理记录位于 `.scratch/todo-editor/native/cleanup.json`。未发送模型提示词，未重新打包或安装。
