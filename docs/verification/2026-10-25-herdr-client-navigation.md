# Herdr 0.9.5 多客户端独立导航验收

## 原因及实现

本机客户端和 default server 均确认升级为 0.9.5。新版 UI 支持每个客户端独立选择 workspace/tab，但公开 CLI `agent focus` 仍经 `handle_api_request_with_shutdown_check` 调用 `focus_all_shell_clients_on_default_target`。仅升级或删除定位命令不能同时满足“选中正确任务”和“不切走另一个窗口”。

参照官方 v0.9.5 源码：

- [全局与客户端范围的请求处理](https://github.com/herdrdev/herdr/blob/v0.9.5/src/server/headless/client_views.rs)
- [稳定 endpoint generation 1 信封](https://github.com/herdrdev/herdr/blob/v0.9.5/src/protocol/wire.rs)
- [官方客户端自身的定位请求](https://github.com/herdrdev/herdr/blob/v0.9.5/src/client/endpoint/activation/protocol.rs)

现在官方 Herdr client 通过 Kanban 进程内的专用命名管道连接现有 server。适配器只加入一次连接范围的 `pane.focus`，核对 endpoint generation、方法能力、boot_id、请求 ID、成功回执和目标 pane 快照。官方渲染及输入协议帧原样传输，没有恢复 ANSI 终端桥接、自定义输入转换或自制右键菜单，也没有新增桥接可执行文件。管道只允许当前用户/SYSTEM，拒绝远程访问；取消、超时或连接失败时释放线程和句柄，不退回全局定位。

`HERDR_SESSION` 保持真实 session，`HERDR_CLIENT_SOCKET_PATH` 指向随机管道；不使用会覆盖管道设置的 `--session`。使用 Herdr 的偏好身份环境变量提供稳定的 Kanban 数据目录/session 键，避免每次临时管道都生成新的侧栏偏好；不启用远程连接、不修改全局配置文件。

## 自动检查

- Rust：138 passed，10 ignored；新增协议封装回归覆盖分片、大小限制、目标快照/回执的顺序、初始已选目标、失败回执和不兼容握手。
- 新增 opt-in `native_terminal::connection::tests::live_client_focus` 在隔离 0.9.5 会话与真实 WezTerm 上显式运行通过。
- 前端：136 passed；TypeScript/Vite 生产构建通过，保留既有 chunk 大小提示。
- `npm run tauri -- dev` 自动编译当前源码成功，真实桌面验收运行此 debug 可执行文件的隔离副本。另一次 custom-protocol 构建因用户正在运行目标路径的 kanban.exe 而无法覆盖文件，未关闭用户应用；不将该次构建记为通过。

## 原生双窗口验收

隔离数据目录 `.scratch/wezterm-native/`，会话 `kanban-wezterm-acceptance`，测试看板独立数据库/WebView。两个 Codex Agent 只进入空闲界面；未提交模型提示词。文字定位检查使用未提交的标记，完成后清除。

1. 独立 WezTerm → 官方 Herdr client 停在 `w2:p1`；Kanban 在 `w1:p1` 和 `w1:p2` 间连续切换。独立窗口画面仍有它自己的 shell 标记，独立窗口输入仍进入 `w2:p1`，Kanban 输入只进入选中的任务 pane。
2. 将测试 Agent B 移到 `w1:t3`，独立窗口停在 `w1:t2` 的 `w1:p3`；Kanban 在 `w1:t1` 与 `w1:t3` 间切换。独立窗口仍保持 `w1:t2`，两个任务输入目标正确。
3. 快速 A→B→A 选择：最终输入只进入 A；独立窗口保持原位置。旧专用 WezTerm 退出。
4. 在真实原生窗口向 pane 发送 Win32 右键事件，专用 WezTerm 终端缓冲区包含 Herdr 的 Rename pane / Split right / Close pane。验证不是 React 模拟菜单；桌面截图被其他前台窗口遮挡，不作为本次证据。
5. 关闭隔离看板后专用 WezTerm/client 退出，测试 Agent 的 terminal_id 保持不变。独立窗口仍是单独的客户端。

本机证据：`.scratch/wezterm-native/independent-result.json`（不同 workspace）、`independent-w1-p3-result.json`（同 workspace 不同 tab）、`native-client-result.json`、`native-client-menu.txt`；脚本 `check_independent.py`、`setup_independent.py`、`native_client_acceptance.py`。测试配置首次启动的欢迎页和设置页曾拦截输入，关闭后重新执行通过；不把首次失败记为通过。

## 边界

- 独立的是各客户端选择的 workspace/tab。同一 tab 的 pane 焦点、分屏布局、共享终端内容及尺寸仍遵循 Herdr 的规则。
- 用户仍可在原生 Herdr 中自行导航；卡片绑定不跟随手动导航改变。
- 本次使用原生事件注入和 WezTerm CLI，不宣称人工物理鼠标、IME、剪贴板或跨屏 DPI 全部验收。
- 没有升级/重启用户的 default server，没有安装 NSIS 包，没有 Git 提交或推送。
