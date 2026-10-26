# Workspace 选择性显示与 SSH 接入界面验证

日期：2026-10-26。规格：[GitHub #14](https://github.com/legendxcheng/herdr-kanban/issues/14)，子任务 #15–#20。验证基于本轮工作区代码；未提交、未打包安装，也未替换用户运行中的看板。

## 已实现

- schema 13 独立保存显示偏好；本地及远端支持隐藏、搜索/多选添加已有、新建并同步 Herdr。首次和外部新增不默认全选；重加按稳定身份保留原记录。
- 隐藏不删除项目、任务、会话、绑定或 Herdr 资源，离线可用；隐藏当前项只释放本 App 查看连接。未决任务和 Workspace 操作保留核实入口。
- 批量添加本地事务原子提交；目录属于执行电脑；未知创建沿用核实而不重发，重复核实不覆盖后续隐藏。
- Local/Remote 与连接状态分开显示，接入成功后直接回到目标电脑的候选列表。
- 原生授权有操作 ID、明确结果、终端关闭判定及阶段错误；新连接检查通过前不能作为执行目标。
- 修正远端 Workspace 创建失败恢复入口漏传 machine_id，以及远端接入后的刷新提前关闭添加弹窗。

## 自动化结果

- 前端：25 个文件、151 项测试通过；新增 App/弹窗行为覆盖搜索多选、失败草稿、迟到候选、远端新建、接入检查门槛、授权关闭、侧栏入口完整衔接、同步期间隐藏和远端恢复路由。
- Rust：166 项通过，17 项既有真实环境测试保持 ignored。新增真实 SQLite 的迁移/重开、显式空选择、隐藏保留任务及会话、批量回滚、跨机同 ID 隔离、离线隐藏和创建核实不覆盖隐藏。授权观测器用独立实际子进程验证精确结果身份、成功、失败和无结果退出。
- TypeScript 与前端生产构建通过，保留既有大块体积提示；独立名称的 Tauri 桌面可执行文件构建通过。原调试程序被占用，因此未覆盖原可执行文件。
- 普通前端测试使用模拟 IPC；Rust 服务测试使用可控 Herdr 适配器；这些结果不替代下述真实环境证据。

## 真实 Tauri／Herdr／SSH 结果

使用 `.scratch/workspace-display-spec/desktop` 下的专用程序、全新数据库及 WebView2 配置，CDP 端口 19284。没有使用或修改用户的任务数据库。远端使用既有 `devpc` profile，正式连接流程部署并校验当前版本的辅助程序。

- 本地真实 UI 新建专用 Workspace，确认 Herdr 中存在；隐藏后 Herdr 仍存在，任务仍可读取。
- 页面重载保留隐藏；通过添加已有恢复同一 Project ID，原任务仍在。
- 从侧栏添加入口进入远端接入，既有 profile 检查通过后自动显示目标电脑候选，没有自动选入全部远端 Workspace。
- 真实 UI 在远端创建专用 Workspace；远端目录手填，本机文件夹选择按钮不出现。隐藏、重新添加后仍为同一远端 Project。
- 本地与远端所选列表在页面重载后保留，本机仍可正常同步；看板和终端显示所属电脑。
- 独立桌面程序退出、重新启动后，两端所选 Project ID 均保留。另在真实 Tauri WebView 中以 CDP 设置 1000×650 视口检查中英文、长名称/路径、添加弹窗和电脑标识；这属于 WebView 布局检查，不等同于系统窗口拖拽或原生终端画面检查。
- 两端各启动一条专用 Codex 验收任务，只要求回复标记，不执行工作目录命令。隐藏后仍发现同一个 Agent process/pane，重新添加仍绑定原 pane；两端派发记录均为 1，没有重新发送提示词。实际目标为本地 `w21:p2`、远端 `w6:p2`。

操作脚本及证据：`.scratch/workspace-display-spec/desktop-acceptance.py`、`live-preservation.py`、`final-desktop.py`，以及 desktop 目录下的 `result.json`、`live-result.json`、`final-result.json`、fixture、日志和截图。专用 Workspace 名称以 `kanban-acceptance-selection-` 开头；没有向用户工作 pane 发送测试探针。

验收后通过正式 `complete_task` 完成两条专用运行任务，两端均无 warning，并退出独立验收程序。专用 Workspace 和隔离数据保留用于复查；用户正在运行的 Kanban 未停止。

## 验收边界

- 首次密码授权未执行人工输入验收；本次真实 SSH 接入复用了已保存连接。授权状态的等待/退出/成功/失败由组件及实际子进程测试覆盖，不据此宣称新的密码/公钥安装已做真实验证。
- 断线及迟到响应主要由自动化覆盖；没有断开整机网络、重启远端 Herdr server，或变更用户 SSH/防火墙配置。
- 当前支持范围仍为 Windows x64。未构建安装包或执行安装升级；父 Issue 不自动关闭。

## 追加修复：早期多机数据库缺列

用户连接 Devpc 时报告 `no such column: helper_name`。只读检查实际 `%APPDATA%/herdr-kanban/kanban.db` 确认迁移记录已到 13，而 machines 表仍是早期 schema 12，仅含 id 至 disabled 八列。此前字段只追加在 schema 12 的首次建表语句中，已有版本标记导致升级时跳过；前一轮全新数据库及当前结构生成的迁移夹具未覆盖这份早期结构。

- 新增 schema 14，在事务内检测并补齐 helper_name / host_identity，保留已有字段值。旧连接需要通过正式检查后才获得辅助程序和主机身份元数据。
- 新回归测试先复现与截图完全相同的查询错误，再验证缺两列、分别缺一列、已有两列以及重复初始化；原机器 ID、任务、项目和隐藏选择均保留。
- 最新 Rust 全套：167 passed、17 ignored；没有修改前端行为。本轮未重新执行前端测试。
- 通过 SQLite backup API 保存故障数据库副本，在隔离目录启动当前 Tauri 程序。迁移后对比 projects、tasks、task_bindings、task_sessions、project_visibility、navigation_preferences 内容完全一致；真实 UI 点击 Connect 成功进入 Devpc 的 Workspace 选择，共读取 5 个 Workspace。证据为 `.scratch/machine-schema-fix/verify.py`、`result.json`、`connected.png`。
- 用户正在运行的是开发模式，代码变更触发既有 watcher 自动重载；只读复查实际数据库已到版本 14，两个缺列已补齐。未手工修改用户数据库，未停止用户窗口或修改 SSH 凭据。
