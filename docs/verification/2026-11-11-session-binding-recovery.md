# 任务会话识别与绑定修复验证

日期：2026-11-11。

## 问题与实现

截图显示 `flow.session_mismatch`。只读检查确认远端 forge-oasis 的 `wP:pS` 仍运行会话 `01a1bf85-d8b0-7a30-a8b7-02385e6c6bd6`。截图所属 Kanban 数据库未取得，不能断言原任务保存了哪个会话，也不能把本次代码验证称作该卡片已恢复。

代码中可复现的问题：查看把未取得会话 ID 与取得不同 ID 合并为同一错误；修复绑定原先未比较保存的会话与候选终端的会话，可能先保存成功，再被查看核验拒绝。

- 新增 `flow.session_unverified`，缺少会话识别或精确 resume 写锁证据不足时保留原绑定。前端沿用既有有界重连，只查询原目标。
- 已识别到不同会话继续报 `flow.session_mismatch`，提示选择原对话所在终端，不自动重试。
- 修复绑定先检查当前目标及保存会话；在数据库事务中复核本地快照，拒绝迟到结果。相同进程沿用原启动证据，替换进程不继承旧证据。
- 不修改历史会话 ID、不重发提示词、不启动或关闭 Agent。没有历史会话的旧绑定沿用显式修复能力。

## 自动化验证

修改前，`cargo test --manifest-path src-tauri/Cargo.toml --offline --locked --lib binding_recovery -- --nocapture` 的三项新增测试全部失败，分别暴露错误分类、错误会话仍保存成功、缺少核验后快照复核。前端新增两项错误分类/重连用例也失败。

最终验证：

- `npm test`：45 个文件、335 项通过。
- `npm run build`：TypeScript 与 Vite 生产构建通过，保留原有大 chunk 提示。
- `cargo test --manifest-path src-tauri/Cargo.toml --offline --locked --lib lifecycle::tests -- --test-threads=1`：99 项通过、2 项环境依赖用例跳过。包含本机/远端替身的识别延迟、错误会话拒绝、同会话替换终端、核验期间本地会话/绑定/代次变化、远端进程替换及无重复派发。
- 全量 Rust 串行首轮：397 通过、2 失败、44 跳过。其中旧模型配置测试的夹具允许绑定无会话外部 Agent，已调整为先确认拒绝，再提供原会话，仍验证外部进程不能冒充已应用模型配置；上述生命周期全量复跑已通过。另一项未修改的 `title_deepseek::tests::invalid_completions_and_multiline_titles_never_replace_summary` 在全量运行中返回 `title.network`，单独复跑通过。最终未再次运行整套 Rust，不声明全量一次通过。
- `git -c core.whitespace=cr-at-eol diff --check`：通过。

## 真实 SSH 只读验证

`lifecycle::tests::binding_recovery::live_readonly_binding_recovery` 使用只读连接读取本机机器配置，任务、保存会话及绑定均构造于独立内存 SQLite。外部调用仅为 Herdr 列表、身份与会话查询。

当前本机数据库记录的 helper `3f291c701616b8be73eccd0819e8be24` 不支持 `view_identity`，首轮返回 `machine.unsupported_method`。远端已存在 helper `e815ce39bfd78eb09d9cd14716584d8a`，与本机现有 debug 程序 SHA-256 前 32 位一致；在测试专用机器配置中选择它后通过。未修改个人数据库、上传 helper 或替换运行程序。生产应用已有 `ensure_current` 升级 helper 流程，本次不改该流程。

可复跑命令（仅用于上述已核对目标）：

```powershell
$env:KANBAN_READONLY_DB = "$env:APPDATA/herdr-kanban/kanban.db"
$env:KANBAN_READONLY_MACHINE = '60b54c3b-655b-4ece-9d57-1bd4d3fa1dec'
$env:KANBAN_READONLY_WORKSPACE = 'wP'
$env:KANBAN_READONLY_PANE = 'wP:pS'
$env:KANBAN_READONLY_HELPER_NAME = 'e815ce39bfd78eb09d9cd14716584d8a'
cargo test --manifest-path src-tauri/Cargo.toml --offline --locked --lib live_readonly_binding_recovery -- --ignored --nocapture
```

结果：错误会话拒绝且不落库；精确原会话的修复、`view_task` 与 `validate_view` 通过；内存数据库派发记录为零。此验证覆盖当前修改的本机服务代码与远端既有 helper 的正常会话上报链，不代表新 helper 的 Windows 写锁分支已经真实验证。

## 交付边界

未进行原生 WebView2 点击重连、更新后桌面重启、Windows 写锁异常或截图所在安装实例的验收。未生成安装包或执行安装。个人任务绑定和活动对话未修改。
