# SSH Codex 程序发现修复（2026-11-09）

## 问题与实现

另一台电脑通过 SSH 连接本机后，新任务启动报 `startup.native_codex_missing`。用户在实际 SSH 会话中补充用户/系统保存的 PATH 后，可以发现 `codex.cmd`，调用相邻的 `resolve-codex.ps1` 得到当前原生程序，并运行 `--version` 成功。

- `title_executable` 按显式 `KANBAN_CODEX_EXECUTABLE`、当前进程 PATH、用户及系统保存的 PATH、既有兼容安装位置依次发现入口。只有当前 PATH 无入口时才读取保存的 PATH，不修改用户或系统环境。
- Windows 保存的 PATH 通过独立、隐藏、非交互 PowerShell 查询，展开环境变量和目录引号；缺少 APPDATA/LOCALAPPDATA 时仅在该查询子进程中补充系统已知用户目录。输出使用 UTF-8 JSON。
- standalone 入口调用相邻的 `resolve-codex.ps1`，校验返回值为存在的绝对 `.exe` 路径；每次解析，不缓存版本、不枚举或拼接 release 目录。任务启动和共用的标题命令构造都支持该入口。
- npm 原生二进制布局继续兼容。显式配置或已选启动器解析失败，不偷偷改用另一个安装版本。解析脚本只接收自身路径，任务正文仍经原生 argv 传递。
- 只读发现子进程沿用有界 I/O 和进程树清理，每次最多 10 秒；未改变任务派发、恢复或数据库协议。

## 验证

- `title_executable::tests`：3 项通过，覆盖显式配置、当前 PATH、保存 PATH、旧兼容位置的优先级，以及 npm 命令构造。
- `codex_launcher::tests`：3 项常规测试通过，覆盖同一启动器切换版本、中文/空格/引号路径、无效解析结果、6 种 npm 布局、原生长多行 argv 和启动记录先于进程执行。
- 显式运行 `installed_codex_is_discovered_without_process_path_or_appdata`：通过。子进程清空 PATH 并移除 APPDATA/LOCALAPPDATA/显式覆盖，实际发现本机 Codex 并运行 `--version`，返回 `codex-cli 0.167.0`。测试只读取安装，不调用模型。
- `lifecycle::`：87 项通过，1 项环境依赖测试跳过。
- `title_process::tests`：2 项通过。
- `cargo build --manifest-path src-tauri/Cargo.toml --bin kanban`：通过。
- 同一精简环境、同一 `stage_startup_prompt` helper 请求对比：此前 release 返回 `startup.native_codex_missing`；修复后的 debug helper 返回 `startup.invalid_operation`。请求故意使用非法 operation，程序发现成功后即停止，不写启动请求、不创建任务或启动 Codex 会话。

## 验收边界

用户提供的真实 SSH 输出证明该账户的启动器发现链可用；修复后的自动化验证覆盖实际 Rust 代码、本机安装和 helper 协议。未在另一台电脑安装本次修复并重新执行完整 GUI 任务送达验收，不能将程序发现通过等同于任务已送达。

## 打包交付（09:59）

- `build_kanban.bat -NoPause -NoOpen` 成功，前端生产构建、Windows Release 和 NSIS 均通过。源码快照包含工作区已有其他改动；247 个源码及构建文件逐一 SHA-256 相等。
- 正式 GUI Release 的 `--remote-helper` 在上述精简环境与只读诊断请求下返回 `startup.invalid_operation`，再次确认程序发现通过，不创建任务或派发输入。
- 安装包：`release/kanban/0.1.0/Herdr-Kanban-0.1.0-windows-x64-setup.exe`，5,042,263 字节。
- SHA-256：`bb1c38fe0b3ec2ad87d60eb9694394a564334b9cf6bfbc11605b4dbc4cd9be59`。交付文件、原始 NSIS 产物和 `SHA256SUMS.txt` 一致。
- 构建日志：`release/kanban/logs/build-20261109-095337-180004.log`；来源/产物核验记录：同目录 `source-20261109-095337.json`、`artifact-20261109-095337.json`。
- 未执行安装或替换当前运行实例。
