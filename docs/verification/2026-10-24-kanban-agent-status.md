# 卡片 Agent 状态与等待反馈置顶验收

日期：2026-10-24。规格：[GitHub #15](https://github.com/legendxcheng/ContextSwitcher/issues/15)，用户已确认看板组件／模拟 IPC、真实 SQLite／模拟 Herdr，以及单列真实桌面验收的边界。

## 已实现

- 卡片标题和顶部 Agent 栏共用状态圆点及中英文提示；运行蓝、等待橙、本轮结束绿、空闲灰、未知空心灰，未启动／会话关闭另有说明。
- 开发中列 blocked 稳定置顶，其他列及组内保留后端优先级／创建时间／ID 顺序；状态恢复后回到原顺序。
- 当前项目可见时 3 秒刷新、焦点刷新；请求合并、隐藏暂停、失败降级及迟到结果隔离。
- 后端复用 Agent 发现 IPC，增加完整 target 和 verified_task_id，验证身份及未决操作；保留 occupied_by 的独占约束语义。前端再核对当前绑定。实时结果不写库，不改变任务业务状态，不修改终端连接身份。

## 自动化结果

- `npm test`：67 项通过。覆盖各状态及未来未知值、可访问提示、等待组内稳定排序、恢复顺序、清理／生命周期未决、匹配身份替换、离线恢复、前台轮询、后台暂停、慢请求不重叠、编辑使旧请求失效、切项目迟到响应，以及重排后保持选择和不新增终端打开请求。
- `npm run build`：通过，包含 TypeScript 检查；保留既有大于 500 kB 的 chunk 提示。
- `cargo test --offline --locked --lib`：58 项通过，3 项既有 live 测试未执行。新增服务测试使用真实内存 SQLite 和模拟 Herdr，覆盖状态读取无业务写入、所有身份字段变化、reserved／pending／unknown／archived 排除、查询失败和 Agent 缺失，并断言 Herdr 查询期间不持 SQLite 锁。
- `cargo fmt --check`：通过。增量 diff 使用 `core.whitespace=cr-at-eol` 检查通过（仓库 Windows 文件含 CRLF）。
- 独立目录的 Windows debug 桌面构建通过；没有替换或重启用户正在运行的看板实例。

## 真实桌面及混合验收

使用 `.scratch/kanban-agent-status/data/kanban.db` 与独立 WebView2 数据目录，启动新构建，通过 WebView2 CDP 执行验收脚本。真实 Tauri IPC、Rust、SQLite 与本机 Herdr 查询确认：三个验收卡片分别显示 working、idle、idle，任务归属匹配。本次查询不发送提示词或关闭 Agent/pane。

为确定性检查审批等待及状态切换，后半程在前端 IPC 适配边界覆盖 Herdr 返回的 status；其余读取仍通过真实后端。验收实例的终端打开请求在该边界被拦截，避免测试操作影响个人 pane。覆盖 blocked 置顶、恢复 working 后正常排序、done／idle／unknown 显示、选中任务保持、无新增终端打开、无输入或派发调用。橙点实测 rgb(245,165,36)，蓝点 rgb(96,165,250)，直径 8px；900×600 卡片高度 80px、无卡片内容溢出，列区域沿用横向滚动。

脚本和证据：`.scratch/kanban-agent-status/verify_native.py`、`native-results.json`、`native-live.png`、`native-blocked.png`、`native-900.png`。测试准备中曾因按路径／子串选中错误项目、Tauri invoke 只读导致模拟注入失败；最终脚本改用项目精确名称及 Vite API 模块拦截，最终验证结果以 native-results.json 为准。

## 未覆盖

- 真实 Herdr 审批界面触发 blocked 及反馈后恢复，真实 done 的未读／已读变化；这些切换目前只有自动化和桌面模拟状态证据。
- 本次没有对真实终端执行输入／重连回归，相关“不额外打开”是模拟边界的调用计数证据。
- 未打包安装或替换用户运行中的旧后端。使用新功能需要启动包含本次 Rust 改动的新构建；仅前端热更新无法补齐旧后端缺少的身份字段。
