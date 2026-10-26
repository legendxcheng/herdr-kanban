# 空 Workspace 刷新闪烁修复（2026-10-26）

## 原因与实现

真实桌面在空 `game-h5` Workspace 中出现标题、工具栏和三列看板短暂替换为 Loading 的画面；侧栏与空终端保留。修复前截图采样在 21.828 秒和 23.828 秒捕获消失，随后约 0.094 秒恢复，与 `useTaskFlowUpdates` 的 2 秒轮询一致。

原 `load()` 每次设置 `loading=true`，渲染使用 `loading && tasks.length === 0`，因此已成功加载的空列表也被当成首次加载。`KanbanBoard` 现使用仅在组件首次查询期间为真的 `initialLoading`，最新有效请求结束后关闭；后台刷新不再将其重置。首次失败沿用错误通知并结束占位，重试也保留看板。App 既有 Project ID key 负责项目切换时重新初始化；请求代次与迟到响应隔离保持原有规则。

不修改轮询频率、IPC、数据库或原生终端管理。

## 自动化验证

- 先执行 `npm.cmd test -- src/board-tests/refresh.test.tsx`：旧代码 3 项失败、1 项通过；失败均捕获刷新时 `.board-loading` 替换已展示看板。
- 修复后 `npm.cmd test`：29 个文件、173 项全部通过。新增 4 项覆盖首次空列表、跨多个轮询周期的慢响应、焦点刷新、失败恢复与新数据展示、删除最后一张任务、新项目首次加载及旧响应隔离、首次失败后的重试。
- 回归检查保留标题、工具栏、三列的原 DOM 节点及新建入口，不仅检查最终列表结果。测试使用模拟 IPC，与下述桌面证据分开。
- `npm.cmd run build`：TypeScript 与 Vite 生产构建通过，保留已有超过 500 kB 的 chunk 提示。

## 真实桌面观察

当前运行的 Tauri/WebView2 开发实例使用 localhost:1420，前端修改通过 Vite 热更新生效。通过 Windows UI Automation 调用侧栏真实 Workspace 按钮，进入空 `game-h5`，确认任务总数为 0、三列均为空；没有创建或删除用户任务。

连续观察 30 秒，共采样 408 帧，未捕获标题、工具栏或三列消失。固定看板区域的内容像素最少 551，修复前 Loading 帧为 0 或 126；判定阈值为 200。输入法浮层消失造成少量像素变化，不计为产品闪烁。结束后切回 `herdr-kanban`。

本地证据位于 `.scratch/flicker-observe/`：修复前 `previous-21.828.png`、`frame-21.828.png`、`frame-21.922.png`、`result.json`；修复后 `empty-after-first.png`、`empty-after-last.png`、`empty-after-result.json`。中途对有任务项目的采样不作为空 Workspace 验收证据。

桌面采样覆盖当前空 Workspace 的连续刷新，不代表所有负载下逐帧无闪烁。慢请求、失败与删除最后一项由组件回归覆盖；未在真实用户数据库执行这些破坏性场景。未重测 Herdr 派发、安装包或重新打包安装。

## 后续重新打包（2026-10-26）

按用户要求执行 `build_kanban.bat -NoPause -NoOpen`，隔离源码快照、锁定依赖恢复、前端构建、Rust Release 与 NSIS 打包成功。版本保持 0.1.0，输出 `release/kanban/0.1.0/Herdr-Kanban-0.1.0-windows-x64-setup.exe`。打包快照中的 KanbanBoard 与修复源码哈希一致；安装包 SHA-256 为 `adca4dab5351e5da9303364394bcbe33dd92ab25640b7c11aede5bb65c78907b`，已与 `SHA256SUMS.txt` 核对。日志为 `release/kanban/logs/build-20261026-115425-20600.log`。保留既有 bundle 体积和 `Profile::prepare` 未使用警告；此次仅生成安装包，未安装或替换正在运行的应用。
