# 本机 Herdr 自动启动验收

日期：2026-10-27。范围：Kanban 首次启动时检查本机 Herdr、按需启动并等待就绪；失败后的显式重试。本次不包含远端服务启动或任务重新派发。

## 实现

- `herdr_startup.rs` 提供应用级 `LocalHerdrStartup` 和 `ensure_local_herdr(retry)` IPC，与 Workflow 使用同一解析后的 Cli 入口/session。支持 PATH 和显式配置的 `.exe/.cmd/.bat` 入口。
- 读取 `status --json` 并核对 client/server session。只有明确 `running=false`、`status=not_running` 才运行 `--session <session> server`。权限拒绝、缺少程序、错误格式、不同会话及明确协议不兼容均不启动。
- 单次检查总计最多 20 秒，每条状态命令最多 2 秒；并发调用共享启动结果。启动进程不放入短命令的 kill-on-close Job；退出 Kanban 保留服务。诊断写入数据目录 `herdr-startup.log`。
- 超时后保留未退出的启动进程；包装器成功退出但后台服务尚未就绪时保留待交接状态，显式重试继续等待，不重复发起不确定的启动。
- 首次同步等待就绪；已有本机项目暂缓挂载任务终端，空态仍保留终端布局。失败保留本地缓存，普通焦点/定时同步不自动重新启动。显式刷新/重试重新检查，成功后继续同步。首次检查结束后普通刷新不卸载看板，保留草稿。
- 无数据库迁移；不修改当前任务、旧派发记录或用户 Herdr 配置。

## 自动化与构建

共享工作区同一时段存在 Workspace 创建记录移除、任务重置等其他修改。最终另从 `75fee484a96b4460c9183af8bad73cee10099433` 提取基线，仅叠加本次 13 个产品/测试文件的改动，生成 `.scratch/herdr-startup/source`。快照脚本和补丁分别为 `.scratch/herdr-startup/snapshot.py`、`scoped.patch`；它们未修改 Git 暂存区。

为避免与其他 Cargo 验证进程争用测试 EXE，快照仅调整验证用 lib/bin 名称，复用依赖缓存；正式仓库的 crate 名称不变。

| 检查 | 结果 |
| --- | --- |
| 隔离快照 `npm test -- --maxWorkers=4` | 34 文件、235 项全部通过 |
| 隔离快照 `npm run build` | TypeScript 与生产构建通过；保留既有大 chunk 提示 |
| 隔离快照 `cargo test --lib -q` | 231 通过、24 项环境依赖测试默认忽略 |
| 新增启动模块常规测试 | 8 项通过：已有服务、状态分类、缺少程序、访问拒绝/格式错误、并发启动、失败缓存/显式重试、慢启动超时续等、包装器交接超时不重复启动 |
| 新增前端启动用例 | 初次同步和任务终端等待就绪；失败保留卡片；焦点刷新不重复启动；点击重试继续同步 |
| 隔离真实 Herdr 测试，显式执行 ignored 用例 | 1 项通过，详见下节 |
| Windows debug 应用构建 | 通过；用于桌面冷启动检查 |
| 本次 Rust 文件 rustfmt、相关 diff 空白检查 | 通过；空白检查按 CRLF 使用 `core.whitespace=cr-at-eol` |

快照首次全量前端以默认并发运行出现两项时序失败（拖拽用例未及时收到终端状态、语言用例技能搜索提示与预期 alert 重叠）；限制为 4 个 worker 后完整通过，未为此修改产品代码。共享树早期 240 项前端和 239 项 Rust 常规通过，之后其他改动继续写入；后续共享树曾出现三项生命周期 `binding.occupied` 失败和测试 EXE 占用导致的 LNK1104，因此最终结果以上述隔离快照为准，不将整个共享树判为通过。

## 真实 Herdr

使用本机 Herdr 0.9.5，测试生成专用临时 APPDATA、session 和带空格路径的 `.cmd` 入口，未使用默认用户 session。正式启动管理器完成：

1. 确认服务未运行。
2. 经包装器启动服务，并等待真实在线状态。
3. 正常读取 `workspace list`。
4. 释放启动管理器后服务继续运行。
5. 新建启动管理器复用该服务，不创建第二次启动日志。

用例清理仅停止专用 session。最终隔离快照再次通过同一真实测试。调用方式：设置 `KANBAN_STARTUP_ACCEPTANCE_EXECUTABLE` 为实际 Herdr 可执行文件，执行 `cargo test --lib herdr_startup::tests::windows::live_server -- --ignored --nocapture`。

## Windows 桌面检查与边界

专用 debug Kanban 使用独立 SQLite、APPDATA 和 WebView2 profile。隔离窗口实际冷启动通过：Herdr 原本离线，启动 Kanban 后恰好一次 server 启动，原生 UIAutomation 读到正常的 `Sync workspaces` 按钮且无启动错误，真实 Workspace 查询成功；关闭 Kanban 后 Herdr 仍在线。证据：`.scratch/herdr-startup/desktop-703fdbf4f3084268b219d4a2e8c97e08/cold/native-ui.json`、`launches` 及复现脚本 `desktop.ps1`。

权限拒绝场景读到了界面中的具体错误，等待超过一次 15 秒轮询后仍未启动服务。重试按钮真实点击未完成验收：UIAutomation Invoke 返回 ElementNotEnabledException，后续原生鼠标尝试的 SetForegroundWindow 返回 false，不能证明点击送达测试窗口。因此重试行为目前由前端 IPC 模拟测试和真实 CLI 启动管理器测试覆盖，不将它们表述为桌面点击已通过。

默认受限运行环境未能正常创建可读的 WebView2 窗口；上述冷启动证据来自经自动审批的桌面验证进程。测试应用、专用服务和本次 Vite 进程已关闭。未替换用户运行中的安装实例，未生成新安装包，也未覆盖另一台电脑的安装升级。
