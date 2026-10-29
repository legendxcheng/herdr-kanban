# Codex CLI Full Access 验证

日期：2026-10-29。用户明确要求修改运行参数、Git 提交并打包。

## 实现

- 任务新建与精确 UUID 恢复共用 `herdr_codex::prepare_codex_session`，将 `--approve-for-me` 替换为 `--dangerously-bypass-approvals-and-sandbox`。保留 `--no-daemon`、任务目录、模型和思考强度参数；本机与 Windows SSH helper 使用同一实现。
- Full Access 表示 `danger-full-access` 沙箱模式和 `never` 审批策略。参数已通过本机 Codex CLI 0.165.0 帮助及[官方参数文档](https://learn.chatgpt.com/docs/developer-commands?surface=cli)核对。
- 新恢复进程的启动证据要求唯一目标会话写锁，不再等待自动审批辅助会话的第二条锁。后续必须保持完整进程身份、精确命令和原锁集合一致。
- 旧 `--no-daemon` 与 `--approve-for-me` 恢复命令、已有单锁和双锁绑定继续兼容。未知参数、混合权限参数、其他程序和会话切换仍拒绝补充确认；不重发原提示词。
- 已经运行且绑定有效的会话沿用原权限，新参数在下一次启动或恢复时生效。标题生成用的只读短进程保持原独立配置；没有修改全局 Codex 配置、数据库结构或 IPC。

## 验证

- `cargo check --offline --locked --lib` 通过。
- 本次四个 Rust 文件的 `rustfmt --edition 2024 --check` 通过。
- 本机 CLI 接受 `codex --no-daemon --dangerously-bypass-approvals-and-sandbox resume --help` 参数组合。
- `cargo test --offline --locked --lib -- --test-threads=2`：276 项通过、0 失败、28 项环境依赖测试跳过。覆盖 CLI 参数传递、Full Access/历史恢复命令、单锁与历史双锁证据、会话变化拒绝等；计数包含共享工作区原有功能。
- 首次测试编译与首次打包均遇到系统可提交内存不足；用户授权关闭异常占用约 68 GB 的 Vite 验图进程后，常规测试恢复通过。该进程来自本项目上午 InProgress 两行卡片验图任务，11:13:19 启动，地址为 `http://127.0.0.1:7949/.scratch/compact-cards/index.html`。构建参数和产品配置未为此降级。失败及成功日志位于 `.scratch/full-access/` 与 `release/kanban/logs/`。
- 既有脚本完成 TypeScript、Vite、Windows x64 Rust Release 和 NSIS 打包；保留现有 chunk 大小与 dead code 提示。安装包为 `release/kanban/0.1.0/Herdr-Kanban-0.1.0-windows-x64-setup.exe`，3,862,072 字节。交付文件、构建目录文件与 `SHA256SUMS.txt` 核对一致：`b084607a079e390e7f4989056622109a7c3d58570933268f8601d1762819aa32`。打包快照中的相关实现与本次代码哈希一致。

## 边界

本轮未进行真实桌面、SSH 或安装升级验收。默认 Herdr 服务要求管理员客户端，当前受限终端的只读连接被拒绝；未修改或重启该服务，也未操作已有业务会话。测试中的 CLI 进程替身不等同于真实 Herdr/Codex 权限菜单或会话恢复验收。

打包沿用 `scripts/build-kanban.ps1 -NoPause -NoOpen`，按当前完整工作区生成隔离源码快照；工作区已有其他未提交变更，因此安装包与本次范围提交并非同一源码集合。本次提交只包含 Full Access 相关源码、测试和结构/验证文档。
