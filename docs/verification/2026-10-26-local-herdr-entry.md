# 本地 Herdr 自定义入口修复（2026-10-26）

## 问题与实现

目标电脑通过 `D:\codex\bin\herdr.cmd` 使用 Herdr 0.9.5，普通及 `--session default` 状态查询均报告 `D:\codex\herdr\herdr.sock`；Kanban 却报告 `%APPDATA%\herdr\herdr.sock` 无服务。源码原先优先默认安装目录，并在找不到时直接以裸 `herdr` 启动，未按 Windows PATH/PATHEXT 发现批处理入口。现已优先保留 PATH 中的入口，显式覆盖仍具有最高优先级，默认安装目录仅作回退。

相关修正：Workspace 目录原先直接读取 Kanban 环境下的 `%APPDATA%/herdr/session.json`。现通过同一 CLI 查询 session 目录，并核对唯一会话名和实际 socket；不从 socket 父目录猜测数据目录。显式 `KANBAN_HERDR_SESSION_FILE` 保留。查询失败维持已有降级行为，不借用其他会话目录。

补测发现 Windows 批处理不接受含 CR/LF 的参数，原请求会失败为 `herdr.unavailable`。对于此类 API 调用，先通过入口的 `status --json` 获取并验证 native exe 与在线 socket，再绕过批处理的参数限制，将原文传给该 exe，并显式固定服务 socket。探测失败不会发送原操作，标记为确定未发送。常规单行调用和终端入口保持不变。

未新增远端配置要求，不更改默认 session，不自动启动或重启用户的 Herdr。

## 回归验证

- 先运行 `cargo test --offline --locked path_launcher_wins_over_installed_copy -- --nocapture`：旧优先级选中安装目录中的另一份 Herdr，测试失败；修复后通过。使用实际 Windows `.cmd` 子进程返回自定义 socket 与 Workspace。
- 补充 `workspace_directory_comes_from_the_selected_launcher_session`：仅修入口时项目目录为 `None`，测试失败；完成目录发现后通过。fixture 中 socket 目录与会话数据目录刻意分开。
- 最终 `cargo test --manifest-path src-tauri/Cargo.toml --offline --locked --lib -q`：173 项通过，18 项需显式环境的用例跳过。其中本次新增的 portable Herdr 用例另行显式执行并通过；其他 17 项未运行。
- 本次修改的 Rust 文件通过 rustfmt 检查，`git diff --check` 通过。保留既有未使用字段/方法警告。
- 覆盖显式路径、裸命令覆盖、PATH 目录顺序、PATHEXT 顺序、`.cmd/.bat/.exe`、含空格目录、缺少可执行文件回退，以及错误/重复会话和 socket 不匹配时拒绝借用元数据。
- 实际 WezTerm 探针执行含空格路径的 `herdr.cmd client`，收到 `client` 参数与指定私有客户端 endpoint；关闭探针进程。此项验证启动入口传递，不等于完整 Tauri/Herdr 终端交互验收。

## 实机与安装包

通过 `.scratch/herdr-entry/verify-live.ps1` 启动隔离的真实 Herdr 0.9.5 服务，wrapper 设置独立 XDG_CONFIG_HOME/XDG_STATE_HOME 并置于 PATH 首项，不设置 `KANBAN_HERDR_EXECUTABLE`。创建专用 Workspace 后显式执行 `live_portable_entry_keeps_the_real_server_and_project_directory`：入口、实际 API socket、客户端 endpoint、从自定义会话目录读回项目目录全部通过。另通过真实 `workspace create` 验证多行参数以及 `& %PATH% ! ^` 和双引号按字面值到达同一服务；修复前多行测试因 `herdr.unavailable` 失败，修复后通过。每次结束只停止隔离服务，保留测试文件证据。

WezTerm 探针已验证 `.cmd` 启动和私有客户端地址传递；未执行本次完整 Tauri 界面点击、真实 Codex 提示词派发或目标电脑安装验收。目标用户电脑需安装新包复测；本机验证不能替代该电脑的实际安装与同步验收。

最终通过 `powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File scripts/build-kanban.ps1 -NoPause -NoOpen` 完成前端类型检查/生产构建、Windows x64 Release 和 NSIS 安装包。保留既有前端大 chunk 提示及一个未使用函数警告。打包快照中的全部五个修改 Rust 源文件与工作区逐一哈希一致。

产物：`release/kanban/0.1.0/Herdr-Kanban-0.1.0-windows-x64-setup.exe`。SHA-256 与输出校验文件复核一致：`ac509cbaea4580255a004b40bb649f987da2abbb1bf61f8846ea3ed4bf13e747`。沿用原版本及安装身份，未在当前电脑执行覆盖安装，未替换运行中的应用。
