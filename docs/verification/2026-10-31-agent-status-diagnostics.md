# 状态查询错误详情与日志

日期：2026-10-31。用户确认补充状态查询失败原因、可复制诊断和可打开的日志目录。当前仍未取得截图所在电脑的原始失败原因；本增量提供捕获证据的能力，不宣称已修复该现场故障。

## 行为与入口

- 卡片状态悬停显示失败阶段和错误码；展开卡片“任务详情”可见原因、整 Workspace／单任务范围、时间和本次查询总耗时。复制诊断附带 Project、machine、session、workspace、pane、任务 ID；整批失败额外附带用户选中的任务 ID，避免误以为只有该任务失败。
- 错误详情中的“打开日志目录”由后端打开实际数据库所在目录，支持 `KANBAN_DATA_DIR`，不接收前端传入的任意路径。默认是在运行看板的电脑上的 `%APPDATA%\herdr-kanban`。
- `agent-status.jsonl` 记录 `failed`／`recovered`。按数据文件、Project、pane 区分，同阶段同错误每 60 秒最多一条，错误变化立即记录；正常轮询不写成功噪声。复用现有 2 MiB 有界写入器，超过阈值在下一条写入时清空旧记录；日志为尽力写入，写失败不阻断查询。内存去重最多保存 4096 个身份。
- 仅记录稳定错误码、阶段、标识、时间和总耗时，不落盘 CLI stderr、提示词、环境变量或认证数据。不能识别的错误归为 `status.external_error`，不展示原始外部文本。
- 阶段区分项目归属、helper 准备、连接配置核验、连接 helper、远端状态请求、Workspace 列表、Agent 列表、单 Agent 复查、进程信息和进程身份核验。单任务失败不污染同批正常任务；恢复清除前端旧错误。
- `herdr_agent_snapshot` 错误返回结构化诊断，成功快照追加 `diagnostics`；旧字符串错误和无诊断字段快照在前端保留兼容。新增 `open_agent_status_logs` IPC。远端 helper 的内部错误字符串携带阶段，入口沿用内容哈希保证同版本 helper。
- 不修改数据库 schema、任务状态、绑定、状态查询 1 秒子命令时限或重试次数，不发送终端输入、不重启 Herdr。状态诊断入口以外的路由调用保留原错误语义。

## 验证

- `npx vitest run src/board-tests/agent_status.test.tsx`：初次 23 项通过，新增整批结构化异常、复制、打开日志 IPC、单任务隔离与恢复清除。随后追加慢快照回归：超过 15 秒标为过期时仍保留单任务错误详情；最终全量覆盖此文件 24 项。
- `npx tsc --noEmit`、最终完整前端 42 文件 314 项测试、`npm run build` 通过；保留现有 bundle 大小提示。
- Rust 日志定向 3 项通过：阶段与安全错误码、按 pane 限频和恢复、真实临时日志文件大小限制与失败／恢复内容。
- 后端服务回归覆盖结构化整批错误、单 pane 阶段与任务归属、清除错误、保留绑定；慢进程 CLI 替身验证 `process_info / herdr.timeout` 从实际调用边界传出。
- 首次默认并发完整 Rust 测试为 323 通过、1 失败、34 跳过。失败位于未修改的 `title_deepseek::tests::http_failures_keep_summary_and_only_transient_errors_schedule_retry`，收到 `title.network` 而非预期 `title.authentication`；该用例单独复测通过。未把第一次运行算作全绿，也未认定已查明该波动根因。
- 完整串行复核 `cargo test --offline --locked --lib --quiet -- --test-threads=1`：324 通过、0 失败、34 项环境测试跳过，143.91 秒。按 CRLF 校验的 `git diff --check` 通过。

## 验收边界

- 组件测试使用模拟 IPC；后端使用模拟 Herdr、CLI 替身与真实临时日志文件。
- 尚未在另一台电脑连接 xc 的真实 SSH 环境捕获本次原始失败，未验收安装版 WebView2 的系统剪贴板及资源管理器打开效果。
- 已按用户后续授权完成 Windows x64 打包，未安装或替换正在运行的应用。用户需运行含本增量的新版本才会产生状态诊断日志；旧版本过去丢弃的错误不能追溯恢复。
- 工作区已有短多行派发改动保持原样；完整测试属于共享工作区结果。

## Windows 安装包交付

- 执行 `powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File scripts/build-kanban.ps1 -NoPause -NoOpen`，独立源码快照构建成功；22 个已修改／新增文件与构建快照逐个哈希一致，包含已有短多行派发改动。
- 安装包：`release/kanban/0.1.0/Herdr-Kanban-0.1.0-windows-x64-setup.exe`。
- SHA-256：`9b59f2b1c00baf669d6c0b5cad072eac38041748c5c9989e8ee22765a0b34f66`；交付包、NSIS 原始产物及 `SHA256SUMS.txt` 一致。
- 构建日志：`release/kanban/logs/build-20261031-172117-16016.log`；同交付目录的 `BUILD-INFO.json` 已更新本次来源文件哈希、完成时间及产物校验。
- 打包通过不等于已安装或完成另一台电脑到 xc 的真实 SSH 故障验收。
