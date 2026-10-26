# Herdr Kanban — AI 编程任务看板

[![CI](https://github.com/legendxcheng/herdr-kanban/actions/workflows/ci.yml/badge.svg)](https://github.com/legendxcheng/herdr-kanban/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Platform: Windows x64](https://img.shields.io/badge/Platform-Windows%20x64-0078D4.svg)](#开始使用)

**在一个 Windows 桌面看板里，通过 SSH 统一管理本机与远端电脑上的 AI 编程任务。**

[English](README.md) | 简体中文

Herdr Kanban 是面向 **Herdr 与 Codex CLI** 用户的开源桌面任务管理工具，支持跨项目、跨电脑工作。通过 **SSH** 接入远端 Windows 电脑，选择需要显示的 Workspace，就能在同一个看板里管理本机和远端任务。将提示词保存为任务卡片，拖入「开发中」后，Agent 会在项目所属电脑上启动；你可以在看板里操作终端，之后还能回到同一个 Codex 会话继续工作。项目使用 **Tauri 2、React、TypeScript、Rust 和 SQLite** 构建。

## 适合什么场景？

当多个 AI 编程任务同时推进时，你需要记住哪个终端对应哪个需求、最初发了什么提示词，以及应该恢复哪个会话。Herdr Kanban 将这些信息关联到任务卡片，并按 Herdr Workspace 组织项目。

它适合维护功能与缺陷待办、让远端开发电脑运行 Agent 并在本机跟进，以及跨仓库、跨电脑集中查看哪些并行任务正在等待你的反馈。

## 主要功能

| 功能 | 你可以做什么 |
| --- | --- |
| 三列任务看板 | 按「未开发 → 开发中 → 完成」推进任务，设置三档优先级，归档已完成任务。 |
| 本机 + SSH 多机管理 | 接入远端 Windows 电脑，识别各项目所属的执行电脑，让任务在对应电脑的环境中运行。 |
| 提示词编辑 | 编写任务说明，通过 `@` 引用项目所在电脑的文件、通过 `$` 调用技能，粘贴图片并查看缩略图；远端任务的图片会传输到执行电脑。 |
| 任务独立终端 | 从看板启动任务，自动分配 Herdr pane，并维护任务与 pane 的独占关联。 |
| 交互终端 | 选择开发中的卡片即可操作对应终端，需要更多空间时可最大化终端区域。 |
| Codex 会话恢复 | 保存任务对应的会话身份，之后恢复同一会话，不重复发送最初的提示词。 |
| Agent 状态 | 查看工作中、等待反馈或审批、空闲、本轮结束等状态；等待反馈的任务自动排到前面。 |
| Workspace 管理 | 选择侧栏显示哪些本机与远端 Herdr 工作区，添加已有或新建 Workspace，拖动调整项目顺序，查看当前 Git 分支。 |
| 汇总看板 | 在一个按优先级排序的看板中操作侧栏已显示 Workspace 的任务，支持编辑、打开终端和流转任务状态。 |
| 任务概况 | 按需查看各 Workspace 的任务数量，以及今日完成数量。 |
| 自动任务标题 | 先保存提示词，再后台生成简短标题；生成失败时保留摘要标题和任务。 |

Agent 本轮结束不会自动将任务标记为完成；你检查执行结果后，再决定是否完成任务。

## 使用流程

1. 在执行任务的电脑上准备好 Herdr 和 Codex CLI；使用远端电脑时，先完成下方的 SSH 接入。
2. 在 Herdr Kanban 中打开「添加 Workspace」，选择执行电脑，添加已有 Workspace，或为代码仓库新建一个。
3. 新建任务，填写提示词和优先级，按需加入文件引用、技能或图片。
4. 将卡片拖入「开发中」。首次启动会准备 Agent pane，并发送已保存的提示词。
5. 点击卡片，在对应终端中查看执行情况，回应 Agent 的问题或审批请求。
6. 检查结果后标记「完成」。应用保存会话并关闭该任务 pane；之后将卡片拖回「开发中」，即可恢复已保存的会话。

启动或发送结果不确定时，使用卡片上的恢复入口核实结果，再决定是否重试。

## 开始使用

当前桌面目标平台为 **Windows x64**。[GitHub Releases](https://github.com/legendxcheng/herdr-kanban/releases) 暂未提供预编译安装包，可以按下方步骤从源码运行。

### 环境准备

- **运行环境：** Windows、WebView2、[Herdr](https://github.com/herdrdev/herdr)，以及已完成所需账户配置、可正常工作的 [Codex CLI](https://github.com/openai/codex)。执行任务前先启动 Herdr。
- **源码开发：** Node.js **22.12+**、npm、Rust **MSVC** 工具链、Visual Studio C++ Build Tools 和 Windows SDK。
- 不同源码版本的终端运行依赖请以该版本的[打包说明](docs/kanban-packaging.md)为准。

Herdr 提供 Workspace 和 Agent 进程，Codex CLI 负责 AI 执行；安装看板不会自动安装或配置这些工具。

### 从源码运行

```powershell
git clone https://github.com/legendxcheng/herdr-kanban.git
cd herdr-kanban
npm ci
npm run tauri dev
```

`npm run dev` 只启动前端开发服务。任务存储、文件访问、Agent 执行和终端交互需要完整的 Tauri 桌面宿主。

### 构建 Windows 安装包

```powershell
.\build_kanban.bat -NoPause -NoOpen
```

也可以双击 `build_kanban.bat`。安装包和 SHA-256 校验文件输出至 `release/kanban/<版本号>/`；构建副本与缓存位于 `release/`。环境准备、更新方式和验收步骤见[打包说明](docs/kanban-packaging.md)。打包脚本不会自动发布 GitHub Release。

## 通过 SSH 远程开发

当前桌面端与远端执行均支持 **Windows x64**。远端电脑需要可访问的 SSH 服务、已运行会话的 Herdr，以及已完成账户与项目访问配置、可正常工作的 Codex CLI。本机需要 OpenSSH 客户端工具，包括 `ssh` 和 `scp`。

1. 先在本机终端使用配置好的 SSH 别名执行 `ssh <别名>`，确认能够登录目标电脑。
2. 打开「添加 Workspace → 添加远端电脑」。选择 Herdr 已保存的电脑并连接，或填写 SSH 别名与远端 Herdr 会话，打开 Windows 密码接入终端。
3. 需要授权时，在独立终端中输入密码。接入流程会配置专用 SSH 密钥；看板不收集或保存密码。
4. 等待连接检查完成。看板检查远端环境并部署辅助程序，通过后即可选择该电脑的 Workspace。
5. 添加已有 Workspace，或使用远端电脑上的目录新建 Workspace，之后按与本机相同的看板流程创建和执行任务。

文件、技能、Git 信息、Agent 进程和 Codex 会话均使用项目所属电脑的资源；任务记录保存在本机看板数据库中。停用连接会保留项目和任务历史，远端 Agent 继续运行。发送中断且结果不确定时，应先核实任务状态再重试；看板不会自动重发提示词。

已验证流程与剩余覆盖范围见 [SSH 验证记录](docs/verification/2026-10-25-ssh-multi-machine.md)。

## 本地数据

所有接入电脑的任务与应用记录统一使用本机的一份 SQLite 数据库，默认保存在 `%APPDATA%\herdr-kanban`，不会在多台电脑之间共享或同步数据库。远端项目文件和 Codex 会话仍保存在执行电脑上。可通过 `KANBAN_DATA_DIR` 指定独立的测试数据目录。个人数据库、依赖和构建产物不入库。

AI 执行仍使用你配置的 Codex CLI 及其服务连接；任务数据保存在本地，不代表模型推理可以离线运行。

## 开发与参与

```powershell
npm test
npm run build
cargo test --locked --manifest-path src-tauri/Cargo.toml
```

CI 在 Windows 上执行前端测试、生产构建及 Rust 常规测试。默认忽略的真实 Herdr 用例需要独立测试 Workspace；常规测试通过不等同于真实桌面或 Agent 兼容性验收通过。

欢迎通过 [GitHub Issues](https://github.com/legendxcheng/herdr-kanban/issues) 反馈缺陷或提出需求。报告问题时请附上 Windows、Herdr 和 Codex 版本、复现步骤、预期行为及错误信息；分享日志和截图前移除凭据与私人项目数据。

- [代码结构与调用链](kanban_code_structure.md)：架构、存储和运行时边界。
- [Agent 工作约定](AGENTS.md)与 [kanban-dev 技能](.agents/skills/kanban-dev/SKILL.md)：在本仓库调用 `$kanban-dev`，先研究方案，确认后实施并同步结构文档。
- [迁移与验证记录](docs/verification/2026-10-25-kanban-repository-migration.md)：独立仓库迁移范围及验证情况。

## 项目来源与许可证

本项目从 [ContextSwitcher](https://github.com/legendxcheng/ContextSwitcher) 的 Kanban 子项目提取，旧提交和 Issue 仍保留在原仓库。历史规格与验收文档中的旧路径、提交号和 Issue 代表当时环境，当前开发命令以本 README 为准。只迁入已跟踪的 Kanban 证据，历史 `.scratch` 临时附件未归档。

本项目使用 [MIT License](LICENSE)。
