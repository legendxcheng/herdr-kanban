# Workspace OpenTerminal 验证记录

日期：2026-10-29。规格：[GitHub #43](https://github.com/legendxcheng/herdr-kanban/issues/43)。

## 实现范围

- 单 Workspace 的 NewTask 旁提供打开终端，支持空 Workspace、普通 shell 与任务终端切换、Todo 草稿保护、最大化与重连。
- schema 18 保存独立普通终端及创建意图。按执行电脑/session/Workspace 复用，启动目录来自 Project；复用不输入命令、不切回目录。
- 普通 shell 使用专用 `kanban-terminal-<操作 UUID>` Tab；UUID 用于丢失创建回执后的唯一核实。任务派发与空闲清理不能认领该终端。
- 本机和 Windows SSH 复用现有 CLI/helper/官方客户端；确认身份后显示，关闭查看不会关闭 shell。未知创建不自动重放。

## 自动验证

- `npm test -- --maxWorkers=4`：最终 40 文件、275 项通过，其中本功能组件回归 7 项。
- `npm run build`：类型检查和生产构建通过；保留既有大于 500 kB 的 chunk 提示。
- `cargo test --offline --locked --manifest-path src-tauri/Cargo.toml --lib -- --test-threads=4`：最终 276 通过、28 ignored。
- `cargo build --offline --locked --manifest-path src-tauri/Cargo.toml --features tauri/custom-protocol`：桌面构建通过，保留既有 dead_code 警告。
- 本次修改的 Rust 模块使用 `rustfmt --check --config skip_children=true` 检查通过；`git -c core.whitespace=cr-at-eol diff --check` 通过。仓库全量 `cargo fmt --check` 仍报告其他已有改动的格式差异（包括任务重置测试、项目测试模块顺序等），未为了本功能重排无关改动。
- 新增服务测试使用真实 SQLite 与 Herdr 替身，覆盖准确 cwd、并发只创建一次、离线不创建、明确关闭后重建、未知创建不重放、丢失回执重启后恢复、目标移动拒绝、机器停用/归属变更拒绝，以及专用 shell 占用保护。
- 前端模拟 IPC 覆盖空 Workspace、按钮重复点击、任务切换、后台刷新、远端 Project ID 路由与错误重连、迟到完成隔离、All Tasks 隐藏按钮、草稿保存/取消及最大化连接不变。原生管理器回归覆盖任务关闭与 Workspace 查看连接隔离。

## 真实本机桌面验收

使用独立编译的 Tauri/WebView2 应用、独立 SQLite 数据目录、真实 Herdr 0.9.5 与 WezTerm；没有替换用户安装版，没有向用户工作 pane 输入内容。

- 创建两个专用空 Workspace，目录均含空格；从真实 OpenTerminal 按钮进入，检查 pane 初始 cwd 精确匹配、没有 Agent 注册。
- 重复点击保持同一 pane/terminal。
- 两个 shell 均通过原生 WezTerm 输入运行 `pause-test.bat`，输出标记和 cwd 后停在 `pause`；切换 Workspace 返回仍保持暂停，未自动按键或出现结束标记。
- 最大化/恢复保留原生查看进程；手动回车完成批处理后分别 `cd subfolder`。
- 正常关闭隔离应用后重新启动，两个原 terminal ID 保持；原生输入、输出与 Herdr `process-info` 共同确认手动切换的目录保留。
- 关闭测试专用 shell 后再次点按钮生成不同 terminal ID；仅关闭本应用 WezTerm 查看进程后点击 Reconnect，保持 shell 身份。
- SQLite `dispatch_operations` 和 `task_bindings` 均为 0，没有启动或派发 Codex。
- 验收结束关闭两个测试 Workspace 和隔离应用，保留测试文件和结果记录。

证据：`.scratch/workspace-terminal/acceptance-local/result.json`、`fixture.json`、`native-final.png`；脚本为 `.scratch/workspace-terminal/accept.py --local` 及 `resume-local.py --local`。第一轮脚本通过了 shell/批处理/切换，尝试通过 WebView IPC 关闭窗口时被应用既有权限拒绝；改用 Windows 正常关闭窗口消息后完成重启及重连验收，未修改产品权限。

## 远端与未覆盖项

- 对现有 devpc（100.69.126.63）尝试真实接入时，SSH 端口 22 超时；尚未执行该电脑上的终端创建、批处理及断线重连验收。用户表示稍后上线；不能以本机实测或模拟路由宣称真实远端通过。
- 当前远端参数路由、helper CLI 协议复用已实现；待连通后使用专用远端 Workspace 补验。
- 尚未覆盖 Herdr 服务重启/恢复后 ID 变化、跨进程外部操作与本应用最后一次身份核验之间的竞态、多实例同时创建，以及安装包升级。
- 未知创建暂未查到目标时保持待核实；不会自动重发，也没有新增手动清除未知记录入口。
