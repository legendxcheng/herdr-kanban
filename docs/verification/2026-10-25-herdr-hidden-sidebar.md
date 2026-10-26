# Herdr 默认隐藏侧栏验收（2026-10-25）

## 问题与实现

用户报告打开的 Herdr 侧栏偶尔出现，拖动 Kanban/Herdr 分隔线也会出现。Herdr 0.9.5 在终端宽度不超过 64 列时采用窄屏布局，临时不画侧栏；恢复宽屏后仍按原来的 sidebar_collapsed 状态绘制。原启动路径没有明确设置收起状态，且已有偏好优先于启动配置。

`native_client_profile.rs` 为每次查看连接生成配置快照，设置 `sidebar_start_collapsed=true`、`sidebar_collapsed_mode="hidden"`；复制该 Kanban 数据目录/session 已有的其他 chrome 偏好，但将 `sidebar_collapsed` 设为 true。每个窗口拥有独立状态目录，旧客户端不能改写新窗口的启动状态。原 Herdr 配置及偏好不被修改，普通 Herdr 窗口仍使用原路径。窗口结束时释放进程后清理临时配置/状态。

保留主题、原生输入及主动展开侧栏的快捷键；在当前窗口主动展开仍有效，下次打开/重连恢复默认隐藏。配置是启动时快照；当前查看窗口内的配置/chrome 修改不写回用户全局文件。没有变更分隔线拖动代码、发送切换快捷键、重连 resize 或数据库迁移。

## 自动检查

- `cargo test --offline --locked --lib`：共享工作区 151 passed、0 failed、12 ignored。包含其他并行改动，不将总数全部归为本次新增测试。
- 新增覆盖隐藏配置覆盖、主题保留、相对提示音路径、非法配置、已有展开偏好、其他偏好保留、原文件不变、旧窗口状态隔离及独立目录清理；启动参数测试验证配置/状态路径仅进入专用子进程。
- `cargo build --offline --locked` 通过。Windows debug 桌面副本使用当前前端开发服务器；没有构建或安装 NSIS 更新包。
- 对本次 Rust 文件执行 edition 2024 格式检查；Cargo.toml 保留仓库原 CRLF 格式。

## 真实桌面复现与修复验证

使用独立数据目录 `.scratch/sidebar-20261025/`、SQLite、WebView2 端口 9558 和 Herdr session `kanban-acceptance-sidebar-20261025`。使用真实 Tauri/WebView2、WezTerm、Herdr 0.9.5 和两个空闲 Codex Agent；没有向模型提交任务提示词。拖动由 Playwright 驱动真实 WebView 分隔线，原生窗口内容通过该 WezTerm 实例 CLI 读取，不使用模拟 IPC。

修复前：

```text
python .scratch/sidebar-20261025/sidebar_regression.py before
drag-0: width_px=291, sidebar_hidden=True
drag-1: width_px=921, sidebar_hidden=False
AssertionError: Sidebar appeared during drag-1
```

修复后同一脚本 `after` 通过：

- 在约 291/921 像素之间反复拖动 6 次，侧栏始终隐藏，过程中 WezTerm PID 不变。
- 最大化至约 1572 像素再恢复，侧栏保持隐藏。
- 通过 Herdr 原生 `prefix+b` 主动展开成功；结束隔离的查看进程后点击真实“Reconnect”，新窗口默认隐藏。
- `sidebar_switch.py switch` 在 A/B 两任务之间切换 4 次，每次宽约 921 像素，侧栏隐藏；旧 profile 目录清理，只剩当前窗口的一份。

本机证据：`sidebar-before-result.json`、`sidebar-after-result.json`、`sidebar-switch-result.json`、对应每一步的终端文本以及两个 Python 脚本，均在 `.scratch/sidebar-20261025/`。最初用 WM_CLOSE 触发了 WezTerm 自身关闭确认，未产生断连，不能算重连验收；最终测试改为结束该隔离查看进程，再走真实重连按钮通过。

## 边界

这是程序驱动的真实 Windows 桌面验证，不等同于人工物理鼠标、跨屏 DPI 或安装包升级验收。用户已有应用/配置未替换。配置/偏好未改动由自动测试与独立路径保证，本次未重做其他正常 Herdr 窗口的完整导航矩阵。并行工作中的 pane zoom 与派发确认变更不属于本次侧栏修复。
