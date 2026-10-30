# 自动标题跟随任务机器

日期：2026-10-30。

## 已实现

- `TaskTitles` 将保存的 Project ID 传入生成器，`RoutedTitles` 从数据库解析任务所属机器。模型/SSH 操作不持数据库锁；完成后复核机器身份与停用状态。
- 本机调用本机 Codex；SSH 经当前版本 helper 的 `task_title` 方法调用远端 Codex，使用 SSH 登录用户的凭据和额度，不复用任务 pane，不回退本机。
- 标题仍使用 `gpt-6-luna` / low / Standard，与开发任务模型独立。已有任务不批量重新命名。创建立即保存摘要，原编辑、删除、归档、退出恢复保护保持。
- Codex 入口支持独立 EXE、PATH、用户 npm 目录。npm 包装器通过 node.exe + 官方 codex.js 的结构化参数执行；未知包装器或错误显式入口失败，不拼接提示词到 shell。
- 两个 worker、60 秒排队、模型进程 30 秒保持；SSH 总预算 240 秒含首次部署，单次请求最多 55 秒。部署及 SSH 等待可随退出取消；远端已启动的模型由自身 30 秒上限清理，不保证 SSH 断线时立即终止模型。
- 请求表保存稳定错误码；任意诊断文本归一为 `title.generation_failed`，不保存提示词/凭据到错误列。无数据库迁移、前端 IPC 变化。

## 验证

- `cargo test --manifest-path src-tauri/Cargo.toml --lib -- --test-threads=4`：295 项通过，31 项环境依赖测试默认忽略。共享工作区包含另一项短多行提交修改，此结果是当前整体工作区的常规 Rust 检查，不代表只测试本次独立提交。
- 新增检查覆盖：本地/远端机器选择，远端失败零本机回退，后台任务创建到结果写回，远端失败保持摘要和具体错误码，停用/取消，数据库锁释放，npm 缺失 PATH 与显式错误入口，包装器参数隔离，远端输入限长，SSH 总时限/取消，部署元数据不读取 stdin且不执行数据中的 shell 表达式。
- 原有标题检查覆盖手动改名/改正文/优先级保存、删除和归档、无效输出、纯图片、队列与退出恢复；本轮完整 Rust 检查通过。
- `cargo test --manifest-path src-tauri/Cargo.toml --lib real_codex_generates_a_title_without_development_tools -- --ignored --nocapture`：真实本机 Codex 三组输入通过。中文、英文和混合引用名称分别约 10.8 / 9.3 / 9.0 秒，得到“修复登录失败提示缺失”、“Fix login error message”、“登录失败提示并保留输入”。
- `cargo build --manifest-path src-tauri/Cargo.toml --bin kanban` 通过，生成当前 debug 构建；不是 release 安装包。改动 Rust 文件的 rustfmt 检查及 `git diff --check` 通过。保留已有未使用成员警告。

## 真实 SSH 尝试与阻碍

目标为已保存的 `devpc`，隔离验收 fixture 在 `.scratch/title-ssh/machine.json`，未改用户实际任务数据库。

1. 只读 SSH 探测确认远端存在用户 npm Codex 和 `C:\Program Files\nodejs\node.exe`，没有独立 OpenAI 安装目录的 codex.exe。既有 helper 的 `hello` JSON 请求成功。
2. 新增 opt-in 测试 `live_ssh_title_uses_remote_codex_with_local_codex_unavailable`：刻意令本机 Codex 入口失效，再部署指定构建产物并请求远端生成，以证明远端链路不依赖本机 Codex。
3. 首次测试在 PowerShell 部署准备处 30 秒超时。最小对照中 Windows PowerShell 5 的 Console.In.ReadToEnd/ReadLine 均挂起，PowerShell 7 及原生 helper 输入正常。部署元数据已改为编码脚本携带，本机真实 PowerShell 回归通过，第二次尝试越过该阶段。
4. 第二次在 scp 上传阶段超时，累计约 187 秒；后续小文件 scp / legacy scp 探测也出现 `Connection timed out during banner exchange`。尚未到达远端模型调用，不能宣称真实远端标题生成通过。没有重发任何用户任务。

## 交付边界

未替换运行中的安装版、修改远端 Codex/PATH/登录状态或升级 Herdr。尚未完成新版本真实 SSH 标题闭环、断线后的远端模型清理观察、release GUI helper 和原生桌面 UI 验收。网络恢复后可通过上述 opt-in 测试继续；部署与运行结果需要分开记录。
