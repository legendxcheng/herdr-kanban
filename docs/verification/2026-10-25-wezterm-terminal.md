# WezTerm 独立终端窗口验收（2026-10-25）

> 历史方案：本文的桥接实现已被[直接运行 Herdr 原生客户端](2026-10-25-herdr-native-client.md)替换。以下保留早期验证经过，不代表当前启动、输入与打包链路。

## 实现范围

移除 xterm/FitAddon，原内容区域由专用无边框 WezTerm 窗口显示。没有 SetParent、owner 关系或全局置顶；Rust 动态同步位置、尺寸和显隐，React 保留标题、准备/错误状态、重连及最大化按钮。每次连接启动独立 WezTerm/桥接进程，使用 PID、唯一窗口类、连接 ID 和随机 loopback 令牌隔离；关闭查看连接继续使用 Herdr terminal.release，保留底层 Agent。

新增控制台桥接程序随 Tauri resources 分发；目标电脑仍需自行安装 WezTerm。原有任务/绑定数据库结构保持不变。

## 自动化验证

- 前端：136 项测试通过，TypeScript 与生产构建通过；保留既有大 chunk 提示。覆盖原生窗口区域/弹窗同步、WebView occlusion 不隐藏终端、请求串行合并和旧连接迟到通知隔离；移除了失去实际含义的 xterm 输入模拟断言。
- Rust：137 项常规测试通过，9 项已有 live 用例未运行。覆盖旧 close/bounds 不影响新连接、非法区域、协议限长，以及 Winsock 非阻塞监听接入后延迟分片接收。
- 独立桥接 crate：5 项测试通过，覆盖跨块 Unicode、滚轮控制标记、bracketed paste、单独 Escape、有界 JSONL 和 socket 延迟分片。
- Windows debug 桌面程序构建通过；beforeDev/beforeBuild hook 能生成独立 bridge exe。CI 已在后端检查前加入桥接构建及其测试。
- Edge + 真实 React/Tiptap + 模拟终端 IPC：更新后的 `scripts/verify-language.cjs` 通过中英切换、900×600 / 1280×800 / 1920×1080 页面布局、连接保持及表单回归。该项不代表原生终端输入验证。

## 原生桌面和真实 Herdr/Codex

使用独立 `.scratch/wezterm-native/` 下的 APPDATA、Kanban SQLite 和 WebView2 用户目录，Herdr session 为 `kanban-wezterm-acceptance`。未使用个人任务数据库或个人工作 pane。环境为 Windows、Tauri/WebView2、WezTerm `20240203-110809-5046fc22`、Herdr `0.8.0-preview.2026-08-04-d78e3d3b5126`、Codex CLI `0.164.0`。创建两个测试 Codex pane，测试任务绑定在隔离数据库中准备，没有向模型提交开发任务。

实际检查：

1. 独立窗口无 owner、无 WS_CHILD、无 TOPMOST；窗口内显示真实 Codex 画面。
2. 在 1280×800、1000×700、1500×950 的外窗尺寸及不同屏幕位置，WezTerm 矩形与 WebView 内容区域换算结果逐项一致（允许 1px，本次记录均相同）。分隔线键盘调整、终端最大化及恢复也保持对齐。
3. 新建任务弹窗打开时隐藏 WezTerm，取消后恢复；最小化主窗口隐藏，恢复后重新对齐。
4. 经明确指定该测试 WezTerm 的私有 socket、禁用自动启动的 CLI 注入未提交 Unicode 文本，真实 Herdr agent read 能读到相同中文；随后清空草稿。该项验证 WezTerm→ConPTY→桥接→真实 Agent 的输入路径，不等同于物理键盘或 IME 候选验收。
5. 滚轮私有标记被桥接消费，转为 Herdr 滚动请求，未作为文字进入 Agent。未用长 transcript 验证实际一格六行的视觉结果。
6. 同项目从 A 切到 B 后旧 WezTerm PID 消失；B 的输入只能在 B pane 读到，A pane 没有该输入。切到另一空项目后查看窗口关闭。
7. 手动关闭测试 WezTerm 后 UI 显示断线，Agent 仍存在；重连启动新的查看进程并成功连接。
8. 正常关闭 Kanban 后查看窗口退出，两个测试 Codex pane 都仍可读取。另起隔离实例强制终止 Kanban，验证 Windows Job 清理 WezTerm 及桥接后代进程，同时两个 Agent 继续存在。最后清理独立测试 Herdr server。

另外，在独立真实 WezTerm 中直接执行已配置的字号动作，验证字体从 13pt 调整到 13.75pt 并写入专用目录偏好。物理 Ctrl 加减号的注入尝试没有形成有效验收证据，因此没有宣称原生快捷键已通过。

本地证据：`.scratch/wezterm-native/results.json`、`lifecycle-results.json`、`crash-result.json`、`native-two-tasks.png`、`validate.py`、`lifecycle_probe.py`、`font_probe.py`、`crash_probe.py`。这些脚本和截图属于本机隔离验收材料，不随安装包分发。

## 已修复的原生验证问题

Windows accept 得到的 socket 继承非阻塞 listener 状态，最初空闲读取返回 WSAEWOULDBLOCK，被误判为断线。现对已接入 socket 显式设置 blocking，再设置有界写入/握手超时；普通接收通过连接关闭解除阻塞。回归以真实 loopback 延迟分片覆盖，并重新通过真实 Herdr 连接及任务切换检查。

## 未覆盖和限制

- 未验证跨物理显示器 DPI、虚拟桌面、系统剪贴板复制/粘贴、IME 候选操作、物理字号快捷键和触控板精细滚动。独立跨进程窗口仍可能存在瞬时帧差，不能承诺任意负载下零延迟。
- 当前滚轮以 pane 原点坐标发送六条单行请求，不支持依鼠标位置选择应用内部嵌套滚动区。WezTerm 仍显示 Herdr 转发的画面，不能据此保证所有终端兼容性问题已消除。
- 暂时隐藏 WezTerm 时没有截图替身；切换任务会重建专用查看窗口。旧 xterm 的 localStorage 字号未迁移，新字号偏好位于应用数据目录 terminal/font-size.txt。
- 本次没有构建/安装 NSIS 更新包，没有替换用户已安装的应用；已验证 debug 桌面可执行文件与桥接资源。窗口集成为 Windows 专用实现。

## 用户反馈后的跟随修复（同日）

用户反馈窗口跟随不及时、频繁消失。旧版本的首次验收只检查移动完成后的矩形，未覆盖外部应用获得前台以及连续原生移动，不能代表跟随体验通过。

旧版本复现：在独立 Herdr/Codex 测试实例中，切到专门创建的外部测试窗口后，WezTerm 的 IsWindowVisible 变为 false；在后台将主窗口移动 (100,70) 像素，终端相对主窗口的偏差变为 (-100,-70)。对应源码在 foreground 不属于主窗口/终端时先隐藏并 return，跳过位置同步；前台时又每 16ms 重复请求置顶和带尺寸的定位。

修复：

- 显隐与位置同步分开，取消 foreground 和 WebView document.visibilityState 的隐藏条件；后台终端保留，并跟随主窗口在普通 Z 序中的位置。激活终端时主窗口排到终端后方，不改变焦点。无 owner、无父子关系、无 TOPMOST。
- 独立监视线程通过 SetWinEventHook 接收主进程位置、全局前台事件，用自身消息循环立即处理最新状态，并保留 16ms 兜底；不依赖主线程/WebView 在系统移动循环中发送通知。按实际差异提交操作，纯移动不发送尺寸修改，稳定层级不重复置顶。
- 仅在主窗口最小化/隐藏/DWM cloaked、React 弹窗或无有效区域时隐藏。窗口控制遵循 [SetWindowPos](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setwindowpos) 和 [SetWinEventHook](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setwineventhook) 的线程、消息循环及 Z 序约定；失焦时前台句柄也可能暂时为 NULL，不能据此判定主窗口不可见。

新版本原生验收：

- 12 轮“外部测试窗口 → 终端 → 主窗口”激活切换，检查终端持续可见、二者层级相邻、外部窗口遮挡二者、不抢焦点、不变成 TOPMOST。
- 外部窗口保持前台时连续移动主窗口三次，每次终端相对区域偏差为 0px。
- 通过 SC_MOVE 和原生键盘消息进入 Windows 真正的移动循环（GetGUIThreadInfo 的 GUI_INMOVESIZE 为真），连续采样 60 次，间隔 25ms；终端无隐藏，采样时相对偏差均为 0px。该项不是物理鼠标注入验收，不推断所有帧都零延迟。
- 当前系统“拖动时显示窗口内容”关闭，初次鼠标注入只显示移动框，不能算作连续内容跟随证据；有效原生循环验收临时开启此选项，在 finally 中恢复，并核对原设置恢复成功。产品不会更改此系统选项。
- 重新通过 1280×800、1000×700、1500×950 定位、分隔线、终端最大化/恢复、弹窗和最小化恢复，以及真实 Codex 未提交 Unicode 输入与滚轮标记消费。修复版 debug exe 已重新构建。

本次证据：`.scratch/wezterm-native/follow-before.json`、`follow-after.json`（仅采用后台/焦点对照，不采用其中未产生位移的鼠标采样）、`follow-regression.json`、`follow_regression.py`、重新生成的 `results.json`。Rust 137 项和前端 136 项通过，桥接本次未改动。
