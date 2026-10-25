# Kanban 独立仓库迁移

日期：2026-10-25。

来源：`legendxcheng/ContextSwitcher` 提交 `af7d31e218fbea2b52a0ca9547bdb683fcf94534`。目标：`legendxcheng/herdr-kanban`，接续远端已有 MIT LICENSE 提交；使用当前源码快照，未导入原仓库历史或迁移旧 Issue。

应用从 `kanban/` 平铺到仓库根目录。包含前端、Rust 后端、测试、锁文件、图标、Windows 打包入口、Kanban 规格与已跟踪验收资料、项目技能和 Agent 规则。Python 主应用及其 CI、专属 ADR、依赖、个人数据和构建缓存不属于迁移范围。原仓库保持不变。

`scripts/build-kanban.ps1` 从根目录镜像应用，额外排除 `release` 与 `.scratch`，避免递归复制。浏览器验收脚本输出改为根目录 `.scratch`。结构文档与技能链接适配根目录，新 Issue 路由指向新仓库；历史链接保留原来源。LICENSE 同时保留原代码 2025 年和目标仓库 2026 年版权声明。

运行时代码、依赖锁文件、Tauri 安装身份与默认 `%APPDATA%\herdr-kanban` 数据位置均保持不变。

## 本次验证

- 迁移清单：201 个已跟踪文件全部存在；138 个运行时代码及资源文件逐字节一致，package.json、package-lock.json、Cargo.toml、Cargo.lock 均保留原内容。
- `npm ci --no-audit --no-fund`：成功。
- `npm test`：20 个文件、126 项测试通过。构建副本生成后再次运行，数量与结果一致；Vite 配置限定 `src/**/*.test.{ts,tsx}`，避免根目录测试扫描 release 副本。
- `npm run build`：类型检查与生产构建通过；保留原有大 chunk 提示。
- `cargo test --offline --locked --manifest-path src-tauri/Cargo.toml`：122 项通过，8 项真实 Herdr 测试默认忽略，无失败。
- 技能 `quick_validate.py`：通过；91 处本地 Markdown 链接全部可解析。
- PowerShell 脚本解析通过；实际构建副本中不存在嵌套 release、.git 或 .scratch，生成产物与依赖均被 Git 忽略。
- `build_kanban.bat -NoPause -NoOpen`：从独立根目录完整生成 Windows x64 Release 与 NSIS 安装包，退出码 0；构建耗时约 5 分钟。
- 安装包：`release/kanban/0.1.0/Herdr-Kanban-0.1.0-windows-x64-setup.exe`；重新读取文件验证 SHA-256 为 `bda7a52a8f43042105f1a81da5b093b36386991ca6ce961288968b44b7d1570e`，与生成的 SHA256SUMS.txt 一致。安装包留在本机，不提交二进制到 Git。
- 提交空白检查按 Windows CRLF 运行，保留原 `DispatchDialog.tsx` 文件末尾空行以保证运行时代码逐字节一致；除此之外无空白问题。

本次未启动或安装应用，未执行真实桌面交互、Herdr/Codex 任务生命周期或安装升级验收；这些历史证据与本次源码迁移检查分别保留。
