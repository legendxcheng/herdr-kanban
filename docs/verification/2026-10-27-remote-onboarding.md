# 远端电脑接入流程验证

日期：2026-10-27。

## 行为变化

- 主入口为“使用现有 SSH 配置连接”，传递 `configureKey: false`；后端缺省也为 false。接入脚本通过非交互 SSH 检查后才调用原 Herdr 注册流程，不生成新密钥、不写用户 SSH 配置或远端公钥。
- “配置 SSH 密钥”是独立显式操作，沿用专用公钥授权及既有 `user@host` 配置修复。失败不会自动从复用现有配置切换到密码/公钥写入。
- 保留 Herdr 已保存电脑的选择、连接检查，以及接入操作 ID、原生进程退出核验、取消、目标/session 精确匹配和辅助程序身份/哈希校验。
- `preflight` 保留 Herdr Workspace 查询，移除直接 `codex --version`；电脑接入不再因辅助进程环境下找不到 Codex 而被禁用。模型查询、任务启动和恢复继续使用各自原有链路，接入成功不表示已验证这些功能。
- 不改 Codex 入口、安装目录、PATH、SSH 服务或运行中的 Herdr/任务；未添加版本固定或自定义 Codex 启动器。

## 回归证据

修改前先执行新增回归：

- 现有 SSH 复用场景在旧脚本触发 `Unexpected key generation in enrollment regression`，4 类场景失败。
- Rust 的隔离子进程令 Herdr 返回可用 Workspace，同时令 Codex 调用失败；旧 `preflight` 返回 `machine.codex_unavailable`，回归失败。
- 新增界面行为测试在旧页面缺少对应入口时失败。

修改后：

- `powershell -NoProfile -ExecutionPolicy Bypass -File scripts/test-enroll-windows-machine.ps1`：13 类场景通过。包含已有自定义密钥、缺省复用、认证失败不注册、Herdr 注册失败、原 9 类专用密钥/迁移场景。SSH 配置选择使用真实 Windows OpenSSH `ssh -G`；网络、公钥写入和 Herdr 注册为替身，临时目录与真实用户配置隔离。
- `npx vitest run src/machines.test.tsx src/workspaceSelection.test.tsx src/i18n.test.ts`：18 项通过，覆盖两种接入意图、成功后精确连接、失败不自动授权、取消不连接，以及双语词条一致性。
- `cargo test --lib remote_helper::tests`：3 项通过，包括 Codex 不可用时允许 Herdr 接入、Herdr 失败仍拒绝接入、身份/session 保护。
- `cargo test --lib machine_enrollment::tests`：2 项通过，包括授权操作结果观测与上述 13 类脚本回归。
- `npm run build`、`cargo build --bin kanban` 通过。保留已有大包体积和 Rust 未使用代码警告。

## 真实 Herdr / 辅助程序验证

执行新构建的 `src-tauri/target/debug/kanban.exe --remote-helper`，通过现有主机身份向本机真实 Herdr 0.9.5/default session 发起 `preflight`。仅对子进程将 PATH 限定为 Windows System32，并显式指定实际 Herdr 可执行文件：

- 同环境 `where.exe codex` 退出码为 1，确认 Codex 不可发现。
- 辅助程序成功返回 Windows/x86_64、用户、主机及 default session，`error: null`，结果不含 Codex 版本。
- 仅查询现有 Workspace，没有启动/恢复任务、发送提示词、关闭进程或修改实际用户 PATH。

此验证是真实 Herdr 和真实辅助程序在本机协议入口的验证，不是真实 SSH 网络接入，也不是桌面点击验收。

## Windows 安装包

2026-10-27 执行 `scripts/build-kanban.ps1 -NoPause -NoOpen` 成功，包含前端生产构建、Rust Release 和 NSIS 打包。远端接入相关的 7 个源码/词条文件与隔离构建快照 SHA-256 一致。

- 产物：`release/kanban/0.1.0/Herdr-Kanban-0.1.0-windows-x64-setup.exe`，3,845,031 字节。
- SHA-256：`f2b2bd391953f3a1ba89382216124f3a2f45866227747395beb7b387fff0d3e6`，已重新计算并与 `SHA256SUMS.txt` 核对一致。
- 构建日志：`release/kanban/logs/build-20261027-143634-76920.log`。
- 未执行安装包，也未替换运行中的应用。

## 尚未覆盖

未在另一台电脑上通过新安装版完成端到端接入，未重新执行真实密码授权、模型查询、Codex 任务启动或恢复。已打包，未安装或替换正在运行的 Kanban；当前安装版仍需后续更新才会使用此流程。共享工作区已有其他未提交功能，本次测试/构建使用当前工作区，未将它们纳入本次开发范围。
