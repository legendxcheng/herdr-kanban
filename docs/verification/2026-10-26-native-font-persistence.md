# 原生 Herdr 字号记忆验收（2026-10-26）

## 行为与范围

原生查看窗口每次打开都会新建 WezTerm 进程。原先快捷键仅修改当前窗口的缩放，关闭后重新加载用户默认字号；Kanban 没有读取或保存该变化。

现在正常加载原 WezTerm 配置，再用 CLI override 的 Lua 扩展接入字号动作。`native_terminal` 管理本机共用的 `native_font::Preferences`，在打开时安装每窗口扩展，在监视循环及关闭时读取该窗口的字号记录。存储为 Kanban 数据目录下的 `terminal/font-preference.json`，不修改用户 WezTerm 或 Herdr 配置。

- 默认 Ctrl 加减及显式配置的字号动作保持可用；用户非字号绑定仍优先。
- 字号按 1.1 倍增减，有效保存范围 6–72 pt；Ctrl 0 恢复用户默认并保存 null。
- task、Workspace、local/SSH 查看窗口共用本机偏好。
- `client-<uuid>/font-events.jsonl` 为追加日志，宿主只发布当前 connection 的变化。切换前排空已写完整记录，拒绝旧窗口的迟到写入。
- 读取残缺记录时等待下一次轮询；发布失败保留读取位置，正常情况下约 16ms 内发布。最后一个尚未发布的调整不承诺在进程强杀/掉电后保留。
- 字号变化只更新 WezTerm 的 font_size override，行列数随之协商；不重连 Herdr，不改变任务进程。

## 自动检查

- `cargo test --manifest-path src-tauri/Cargo.toml --lib`：187 passed，19 ignored。ignored 为原有专用环境用例，未算作本次通过。
- 最终 `cargo test --manifest-path src-tauri/Cargo.toml --lib native_terminal`：15 passed，3 ignored。包含本次 4 项存储回归及修改后的生产启动参数检查。
- 存储回归覆盖：切换发生在监视轮询前、旧窗口迟到写入、重置与新管理器、完整/残缺/无效记录、发布失败重试、Unicode 路径和 Lua 长字符串定界符。
- `npm run build`：类型检查及生产构建通过，仍有既有的大 chunk 提示。本次没有修改前端产品代码，未重新运行前端全量组件测试。
- 实际 WezTerm `show-keys --lua` 验证原配置加载与默认映射。另使用独立 `config_builder` 配置检查保存值 21.175、空值、null、越界值和损坏文件；用户默认 17.5、配色、配置路径、Ctrl+= 非字号自定义绑定、F8 字号绑定和 key_table 均符合预期。

配置兼容检查脚本和结果：`.scratch/font-persistence/config_contract.py`、`contracts/result.json`。没有写入用户配置。

## 真实桌面

使用隔离的 `KANBAN_DATA_DIR`、APPDATA、WebView2 数据目录及 `kanban-acceptance-font-size` Herdr session；两个专用 Workspace 各运行一个空闲 Codex，未发送模型提示词。测试窗口按专用进程 ID 和数据目录校验，不按可执行文件名选择。

环境：Windows Tauri/WebView2，Herdr 0.9.5，WezTerm 20240203-110809-5046fc22。通过真实前端点击切换及实际原生窗口键盘事件、WezTerm CLI 读取行列/文本和磁盘偏好进行核对；没有模拟 terminal IPC。

| 检查 | 实际结果 |
| --- | --- |
| Ctrl+= 放大 | 13 → 14.3 pt，终端单元格从 10×22 变为 11×24 px，偏好保存 |
| 跨任务和 Workspace | 第二个 Workspace 的新 WezTerm 恢复 14.3 pt 和 11×24 px |
| 继续放大 | 14.3 → 15.73 pt，单元格 12×26 px |
| 断线/重连 | 仅终止测试查看进程，再点击实际 Reconnect 按钮；新窗口恢复 15.73 pt |
| Ctrl+- 和小键盘加 | 保存值和原生行列随操作变化，之后切换仍恢复 |
| 快速切换与刷新 | 调整后立即切 Workspace、WebView reload，均保留最新字号 |
| 退出并重开应用 | 正常关闭测试 Tauri，使用同一独立数据目录重新启动；恢复 15.73 pt 和 12×26 px |
| Ctrl 0 | 恢复 13 pt、10×22 px，磁盘值为 null；再次重连仍正确 |
| 普通输入 | 通过真实 WezTerm/Herdr 输入未提交的测试文本，读取后清空；未按 Enter 提交 |
| 原生右键菜单 | 实际 Win32 右键事件打开含 Rename pane / Close pane 的 Herdr 菜单 |

单元格尺寸由 WezTerm 的 pixel_width/cols、pixel_height/rows 核算，避免窗口最大化或布局改变导致列数不同而误判字号。截图已查看，原生 Herdr 与 Kanban 布局正常。

证据：`.scratch/font-persistence/native_acceptance.py`、`native/result.json`、`native/font-persisted.png`。本机常规开发入口构建通过；后续相对数据目录绝对化及启动断言补充已通过最终 Rust 终端回归。

## 验证边界

- Ctrl+Shift+= 和小键盘减号的配置映射存在；本次 Win32 键盘注入未稳定触发，不计为真实快捷键通过。
- 未执行物理键盘/IME 全组合、跨屏 DPI、真实 SSH 任务和 NSIS 安装升级验收。SSH 使用同一字号模块和本地偏好，但这是代码调用链证据。
- 不接管用户自定义 Lua 回调内部自行调用的字号动作或命令面板里的原生字号动作；本次保存的是已接入的键绑定动作。
- 测试早期遇到 Windows 保留端口导致 CDP 不监听，最终改用 19686；早期按 exe 名选择开发实例的证据废弃，最终记录来自专用 PID。Herdr 首次使用引导曾拦截普通输入，最终隔离配置关闭 onboarding 后重测通过。
- 当前安装目录中的应用没有替换，本次没有打包或发布。
