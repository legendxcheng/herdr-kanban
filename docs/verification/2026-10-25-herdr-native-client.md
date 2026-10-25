# WezTerm 直接运行 Herdr 客户端（2026-10-25）

> 本记录对应 0.8.0 预览版与最初的原生客户端接入。升级至 0.9.5 后的客户端范围定位及双窗口独立导航见[后续验收](2026-10-25-herdr-client-navigation.md)；下文全局 focus 与状态语义保留历史含义。

## 修正范围

用户要求的是独立 WezTerm 中的完整 Herdr 体验，尤其是 Herdr 原生右键菜单。此前使用了 `kanban-terminal-bridge.exe → terminal session control`，只能呈现 pane 输出，没有运行 Herdr UI；确认 WezTerm 进程存在不能证明该需求已完成。此前桥接方案及其验收属于历史记录，不再是当前实现。

现在启动链路为 `Kanban → wezterm-gui.exe → herdr.exe --session <name> client`。使用已安装 Herdr 的 connect-only 客户端入口；在不存在的隔离 session 中实际验证该命令报连接错误、不会启动服务器。因此查看窗口的 Windows Job 仅包含 WezTerm、控制台宿主及客户端，不包含 Herdr server 或 Agent。

不再转发 ANSI、键盘、鼠标、滚轮或行列尺寸，不实现或仿制 Herdr 菜单。删除 bridge crate、wire 协议、构建脚本和打包资源，移除专用 Lua。WezTerm 正常加载用户配置，仅通过命令行关闭装饰和 tab bar；Herdr 正常使用自身配置。旧专用字号文件不再使用，不修改或删除用户配置。

启动前校验任务绑定，再调用 `agent focus <pane>`。打开请求串行化并校验连接是否已过期，避免旧请求迟到后覆盖新选择。清除从宿主继承的 `HERDR_ENV`、pane/tab/workspace/socket 上下文及 `WEZTERM_*` 环境变量，避免嵌套检测或复用其他 WezTerm 实例。

窗口跟随沿用独立 WinEvent 监视、普通 Z 序和 Job 清理。无 SetParent、owner 或 TOPMOST。关闭、归档、删除任务、删除项目、修复绑定及完成任务的关闭路径均使用 NativeTerminalManager。旧 `terminal.rs` 只在 cfg(test) 下编译，保留历史协议回归，不进入产品运行链路。

## 实际验证

- 前端 136 项通过，TypeScript/Vite 生产构建通过；保留既有大 chunk 提示。
- Rust 135 项通过、9 项已有 live 测试 ignored。新增启动参数/继承环境回归，覆盖官方 connect-only 入口、无自定义输入配置；任务关闭取消迟到打开改用真实原生管理器入口测试。测试数量减少源于删除 bridge wire 的 3 项测试，并新增 1 项启动检查。
- Windows debug 桌面构建通过。`npm run tauri -- dev` 使用 Vite 提供的当前 TerminalPanel，无 xterm import，不依赖此前 bridge 构建步骤。
- 使用 `.scratch/wezterm-native/` 下独立 APPDATA、数据库和 WebView2，Herdr session 为 `kanban-wezterm-acceptance`；没有向模型提交提示词。
- 直接启动官方 Herdr 后，客户端日志显示 `encoding=SemanticFrame`；此前桥接是 `TerminalAnsi`。最终真实 Tauri 实例的 WezTerm 后代为 `OpenConsole.exe` 和 `herdr.exe ... client`，没有 bridge。
- 在真实 Tauri 终端最大化区域，通过 Win32 鼠标移动/右键 down/up 事件打开 Herdr 原生菜单。实际内容包括 Rename pane、Split right、Split down、Zoom、Close pane；读取该专用 WezTerm 的画面并查看截图确认，不以 React 模拟菜单或 CLI 文本输入作为最终右键验收。
- 关闭看板后该 WezTerm 退出，测试 Agent 的 terminal_id 保持不变。
- 同一项目在 A/B 两任务间切换：旧客户端退出，Herdr 的 focused agent 定位到选中 pane；发送未提交的 `NATIVE_CLIENT_ONLY_B` 仅在 B 中读到，A 中没有该文字，随后清空草稿。切回 A 后焦点正确。关闭看板后两个测试 Agent 均保留。

本机证据：`.scratch/wezterm-native/native-client-result.json`、`native-client-menu.txt`、`native-client-menu.png`、`native_client_acceptance.py`、`native-client-switch.json`、`native_client_switch.py`。测试原生右键采用 Win32 事件注入，不宣称人工物理鼠标已经验收。

## 行为边界

- 显示完整 Herdr UI，保留其侧栏、tabs、分屏和菜单。任务选择只负责打开时的定位；用户随后可在 Herdr 中自行导航，Kanban 卡片绑定不会自动改为当前 Herdr pane。
- 当前 Herdr 版本同一 session 共享导航状态，`agent focus` 及原生菜单操作可能影响其他已连接客户端。不会伪称为 pane 独占的输入代理。
- 状态“Herdr 已打开 / Herdr open”表示原生窗口已创建；后续连接、菜单和错误画面由 Herdr 呈现，不伪造协议首帧握手。
- 关闭查看窗口保留 Agent；用户主动在 Herdr 菜单中关闭 pane 是 Herdr 自己的真实操作，不属于单纯关闭查看窗口。
- 本次没有安装 NSIS 更新包。跨屏 DPI、虚拟桌面、物理输入法和剪贴板仍未完整验收；原生窗口层级仍保留此前跟随方案的限制。
