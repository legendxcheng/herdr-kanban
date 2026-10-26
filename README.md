# Herdr Kanban — AI Coding Agent Task Board

[![CI](https://github.com/legendxcheng/herdr-kanban/actions/workflows/ci.yml/badge.svg)](https://github.com/legendxcheng/herdr-kanban/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Platform: Windows x64](https://img.shields.io/badge/Platform-Windows%20x64-0078D4.svg)](#getting-started)

**Manage AI coding tasks, prompts, terminals, and Codex sessions from a Windows desktop Kanban board.**

English | [简体中文](README.zh-CN.md)

Herdr Kanban is an open-source task manager for developers who use **Herdr and Codex CLI** across multiple projects. Turn a prompt into a task card, move it to **In progress** to start an agent, follow its terminal, and resume the same Codex session when you return to the task. Built with **Tauri 2, React, TypeScript, Rust, and SQLite**.

面向 Herdr 与 Codex CLI 的 Windows 桌面 AI 编程任务看板：集中管理提示词、Agent 终端和可恢复的 Codex 会话。[阅读中文介绍 →](README.zh-CN.md)

## Why Herdr Kanban?

When several coding agents are working at once, it is easy to lose track of which terminal belongs to which task, what prompt started it, and which session to reopen. Herdr Kanban keeps those connections on a task card, organized by Herdr Workspace.

Use it to manage a backlog of bug fixes and features, work across repositories, or keep track of concurrent coding tasks that need your feedback.

## Features

| Capability | What you can do |
| --- | --- |
| Task board | Organize work into To do, In progress, and Done, with three priority levels and completed-task archiving. |
| Prompt editor | Write task instructions, reference project files with `@`, invoke skills with `$`, and paste images with thumbnail previews. |
| Task-specific agent panes | Start a task from the board and let Kanban allocate its Herdr pane. Each active task keeps its own binding. |
| Interactive terminal | Select a running task to interact with its terminal; maximize the terminal when you need more space. |
| Codex session recovery | Save the task's session identity and resume it later without resending the original prompt. |
| Agent status | See working, waiting for feedback or approval, idle, and turn-ended states. Tasks waiting for feedback rise to the top. |
| Workspace management | Sync Herdr Workspaces, create or associate a Workspace, reorder projects, and see the current Git branch. |
| Task overview | Open an overview of task counts across Workspaces, including tasks completed today. |
| Automatic titles | Save a prompt immediately and generate a short task title in the background, with a fallback if generation fails. |

An agent finishing a turn does **not** automatically mark the task Done. You review the result and decide when to complete the task.

## A typical workflow

1. Open Herdr and select or create the Workspace for your repository.
2. Open Herdr Kanban and select the synced Workspace.
3. Create a task with a prompt and priority. Add file references, skills, or images as needed.
4. Move the card to **In progress**. Kanban prepares an agent pane and sends the saved prompt on first start.
5. Select the card to use its terminal and respond when the agent needs input.
6. Mark the task **Done** after reviewing the result. Kanban saves the session and closes the task pane; moving the task back to **In progress** resumes the saved session.

If a start or dispatch result is uncertain, use the task's recovery controls to verify what happened before retrying.

## Getting started

The current desktop target is **Windows x64**. Prebuilt installers are not currently published in [GitHub Releases](https://github.com/legendxcheng/herdr-kanban/releases); use the source setup below.

### Prerequisites

- **Runtime:** Windows with WebView2, [Herdr](https://github.com/herdrdev/herdr), and a working [Codex CLI](https://github.com/openai/codex) installation with its required account configuration. Start Herdr before using task execution.
- **Source development:** Node.js **22.12+**, npm, the Rust **MSVC** toolchain, Visual Studio C++ Build Tools, and a Windows SDK.
- Check the [packaging guide](docs/kanban-packaging.md) for runtime requirements for the revision you are building.

Herdr Kanban relies on Herdr for Workspaces and agent processes, and on Codex CLI for AI execution. Installing the board does not install or configure those tools.

### Run from source

```powershell
git clone https://github.com/legendxcheng/herdr-kanban.git
cd herdr-kanban
npm ci
npm run tauri dev
```

`npm run dev` starts only the frontend development server. Use the Tauri desktop app for task storage, file access, agent execution, and terminal interaction.

### Build a Windows installer

```powershell
.\build_kanban.bat -NoPause -NoOpen
```

You can also double-click `build_kanban.bat`. Installers and SHA-256 checksums are written to `release/kanban/<version>/`; build copies and caches stay under `release/`. See the [packaging and upgrade guide](docs/kanban-packaging.md) for prerequisites and validation. Packaging does not publish a GitHub Release.

## Local data

Tasks and application records are stored in SQLite under `%APPDATA%\herdr-kanban` by default. Set `KANBAN_DATA_DIR` to use a separate data directory for testing. Personal databases, dependencies, and build artifacts are excluded from this repository.

AI execution still uses your configured Codex CLI and its service connection. Local task storage does not make model inference offline.

## Development and contributions

```powershell
npm test
npm run build
cargo test --locked --manifest-path src-tauri/Cargo.toml
```

CI runs frontend tests, a production frontend build, and regular Rust tests on Windows. Ignored live Herdr tests require an isolated test Workspace; ordinary test results do not establish real desktop or agent compatibility.

Report bugs and suggest improvements through [GitHub Issues](https://github.com/legendxcheng/herdr-kanban/issues). For a bug report, include your Windows, Herdr, and Codex versions, steps to reproduce, expected behavior, and the error message. Remove credentials and private project data from logs or screenshots.

- [Code structure and execution flows](kanban_code_structure.md) — architecture, storage, and runtime boundaries.
- [Agent contribution conventions](AGENTS.md) and [kanban-dev skill](.agents/skills/kanban-dev/SKILL.md) — repository workflow; research a change before implementation.
- [Migration and verification record](docs/verification/2026-10-25-kanban-repository-migration.md) — extraction into this repository and validation scope.

## Project history and license

Herdr Kanban was extracted from the Kanban subproject in [ContextSwitcher](https://github.com/legendxcheng/ContextSwitcher). Earlier commits and Issues remain there. Historical paths, commit IDs, and Issue links in verification documents describe their original environment; use this README for current development commands. Historical temporary `.scratch` attachments were not archived.

Released under the [MIT License](LICENSE).
