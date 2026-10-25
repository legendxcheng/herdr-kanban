# Herdr Kanban

基于 Tauri 2、React、TypeScript 和 Rust 的 Windows 桌面看板，直接同步 Herdr Workspace，为任务保存提示词、管理独立 Agent pane，并提供交互终端和 Codex 会话恢复。

## 开发

应用源码位于仓库根目录。准备 Node.js 22.12+、Rust MSVC 工具链、Visual Studio C++ Build Tools 与 Windows SDK；桌面需要 WebView2，实际任务派发还需要可用的 Herdr、Codex CLI 及账户配置。

```powershell
git clone https://github.com/legendxcheng/herdr-kanban.git
cd herdr-kanban
npm ci
npm run tauri dev
```

`npm run dev` 仅启动前端服务；完整功能依赖 Tauri 桌面宿主。

## 检查与打包

```powershell
npm test
npm run build
cargo test --locked --manifest-path src-tauri/Cargo.toml
.\build_kanban.bat -NoPause -NoOpen
```

也可双击 `build_kanban.bat`。安装包与校验文件位于 `release/kanban/<版本号>/`，构建副本和缓存位于 `release/`；详见[打包说明](docs/kanban-packaging.md)。CI 执行前端测试、生产构建及 Rust 常规测试；默认忽略的真实 Herdr 用例需独立测试 Workspace，不能把常规测试当作真实 Agent 验收。

## 项目导航

- [代码结构与调用链](kanban_code_structure.md)
- [Agent 工作约定](AGENTS.md)与 [kanban-dev 技能](.agents/skills/kanban-dev/SKILL.md)：在本仓库调用 `$kanban-dev`，先研究方案，确认后实施并同步结构文档。
- [GitHub Issues](https://github.com/legendxcheng/herdr-kanban/issues)：新增需求与缺陷。
- [迁移与验证记录](docs/verification/2026-10-25-kanban-repository-migration.md)

默认任务数据位于 `%APPDATA%\herdr-kanban`；`KANBAN_DATA_DIR` 可指定隔离验收目录。迁移仓库不改变应用安装身份、数据库格式或已有任务。个人数据库、依赖与构建产物不入库。

本项目从 ContextSwitcher 的 Kanban 子项目提取当前快照；旧提交和 Issue 仍留在[原仓库](https://github.com/legendxcheng/ContextSwitcher)。历史规格与验收文档中的旧路径、提交号和 Issue 代表当时环境，当前开发命令以本 README 为准。只迁入已跟踪的 Kanban 证据；历史 `.scratch` 临时附件未归档。

MIT License，见 [LICENSE](LICENSE)。
