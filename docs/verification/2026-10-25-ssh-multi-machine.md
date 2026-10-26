# SSH 多机看板验证（2026-10-26 更新）

对应规格：legendxcheng/herdr-kanban #2；子票 #3–#13。数据库仍在看板所在电脑，每台执行电脑运行自己的任务。

## 实际环境

- 本机 Windows，Herdr 0.9.5；远端 `ssh devpc` 为 Windows 10 x64，用户 Administrator。
- 已由用户在独立 SSH 终端输入密码，完成专用 ED25519 公钥授权；第二次输入成功。应用不收集密码。
- Herdr 已保存 profile `a3beff764e7c3b46c11d7a1201f94cd2`，remote session `default`。
- devpc 已安装 CLI 0.9.5，当前运行 server `0.9.2-preview.2026-09-29-8e78f929d8f0`。未升级或重启现有 server。
- 辅助程序部署到远端 `.herdr-kanban/helpers/<hash>.exe`，SHA-256 校验后发布。主机身份文件在每次 JSON 请求前核实。

## 已验证

- 机器注册幂等、停用后历史身份保留、外部 profile 身份变化拒绝。
- schema 11 数据库升级到 12 并重新打开：旧 Project、任务绑定、会话、unknown 派发及 workspace 排除记录仍归 Local；不重放。
- 同 session / workspace / pane ID 的两台电脑独立导入、创建和绑定；远端失联不把本机项目标成缺失。
- 同一路径通过执行环境读取不同正文；模拟远端读取期间数据库锁已释放。自动启动使用受理的文档。
- 图片完整内容和摘要校验、缺失探测、已校验内容复用、破损内容拒绝。
- 真实 SSH 辅助程序 hello/preflight、workspace 列表、项目创建和改名、项目文件/技能搜索、Git 分支查询成功。
- 真实 WezTerm + Herdr 客户端通过官方 `remote-client-bridge`：在专用 `kanban-acceptance-ssh-renamed` workspace 的 `w4:p1` / `w4:p2` 连续切换四次，每次收到 pane.zoom 回执和目标快照；关闭连接后两个 pane 均存活。此项不是完整 Tauri 界面验收。
- React/模拟 IPC：远端请求挂起或失败时，本机仍可刷新，远端卡片保留；接入失败不报告成功；密码不进入 WebView；接入 session 明确传递。
- 前端全量 24 文件、142 项测试通过；TypeScript 与生产构建通过。Rust 全量 159 通过、0 失败、17 忽略；图片准备失败回归专项 4 项通过。生产构建保留原有大块体积提示。

- 真实独立 Tauri/WebView2：本机及 devpc 分组、中英文电脑管理、官方已保存连接显示、远端新项目目录输入、停用后重开保留远端项目、本机仍可同步均通过；使用复制的隔离数据库，没有停用用户实际连接。证据在 `.scratch/ssh-multi-machine/desktop-acceptance/result.json` 和同目录截图。
- 审查发现并修复六项边界：附件失败孤立引用、可变 profile 路由、显式刷新旧响应覆盖、定时刷新作废慢响应、离线未决操作缓存、终端所属电脑标识。两个审查轴均已静态复核修复。
- 重建并部署辅助程序后，真实 SSH CLI 读取验证不再依赖可变 profile；身份核验后的原生桥接四次 pane 切换再次通过。

## 真实任务闭环（2026-10-26）

用户确认后，将 devpc Herdr 的新终端默认 shell 改为现有 WindowsApps 独立 PowerShell 7.6.8，保留 WezTerm。配置备份为 `%APPDATA%/herdr/config.toml.kanban-shell-backup-20261025235416`，`server reload-config` 返回 applied；没有重启 server 或关闭用户已有任务。

专用验收数据库 `.scratch/ssh-multi-machine/acceptance-run2`，远端 Workspace `w5`，目录 `.herdr-kanban/acceptance-8d00b101c3874f9093caf1785ae4b6be`。首次启动被 Codex 更新菜单、目录信任提示阻挡，均保留 unknown 且未发正文；跳过当前版本更新并信任仅此自建目录后，确认没有派发记录、关闭对应测试 pane，再通过正式核实入口复位，未直接修改任务状态。

实际 Codex 0.163.0 已通过 view_image 读取远端 PNG、读取项目 Skill，并写出 `REMOTE-REFERENCE-7c92 / SKILL-READ / red`。正式完成流程成功，随后恢复时发现 npm 的 node.exe 包装器不持有会话写锁。新增对唯一直接 codex.exe 子进程的严格检查；不可查询或歧义时仍拒绝确认，保留原有精确命令和启动锁集合验证。

修复后 `live_remote_exact_resume_after_environment_repair` 通过：恢复原会话 `01a17581-bd4f-7b31-8ed0-a635f91a71dc`，目标 `w5:p6`，派发记录仍为 1。没有重新发送原提示词。此闭环分阶段完成，不把首次环境准备失败的测试记为通过。

真实 Tauri + WezTerm 组合验收通过（隔离数据库 `.scratch/ssh-multi-machine/desktop-combined`）：本机任务读取 LOCAL 标记并产出文件，与远端任务切换；两端未提交输入草稿互不串入；最大化仍显示 devpc 标识；仅终止本验收窗口的 SSH 连接后出现重连入口，本机仍能同步；重连、页面重载连接恢复及跨电脑概况均通过。记录见同目录 `result.json`。截图受另一用户窗口遮挡，窗口内断言与实际原生终端文本检查通过。

原 `acceptance-run1` / `w4` 的旧 shell 失败记录保留，未自动重试。

## 测试入口

- `cargo test --lib remote_acceptance::live_remote_task_images_references_complete_and_resume -- --ignored --nocapture`：需显式 `KANBAN_ACCEPTANCE_MACHINE`、`KANBAN_ACCEPTANCE_HELPER`、包含 ssh-multi-machine/acceptance 的 `KANBAN_DATA_DIR`。
- `cargo test --lib live_remote_native_client_switch_and_reconnect -- --ignored --nocapture`：复用上述隔离目录中的 machine.json / fixture.json，仅允许专用验收 workspace。
- 普通测试不建立 SSH 连接；live 测试均默认忽略。

## 保留的边界

- 首期接入 Windows x64；不宣称跨平台远端支持。
- 未知派发结果不重发；恢复精确会话不重新解析正文或上传旧附件。
- 停用连接保留历史并关闭本看板查看窗口，远端 Agent 继续运行。附件缓存保留，不随断线/移除而删除。
- Herdr 同一 tab 的 zoom 和终端尺寸仍可能共享；客户端导航通过专用连接，不调用全局 focus。

## 提交范围与剩余覆盖

按用户最终指示，将 SSH 功能与工作区原有的派发回执恢复、隐藏侧栏、选中 pane 显示修复合为一个提交；不分拆提交，不推送。真实图片验收使用单张 PNG；多图片/损坏复用由自动化用例覆盖，未执行真实多图组合。整机网络断开与 Herdr server 重启未执行，真实断线测试限于本看板专用 SSH 客户端连接。
