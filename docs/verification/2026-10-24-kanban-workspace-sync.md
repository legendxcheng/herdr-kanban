# Herdr workspace 同步验收记录

日期：2026-10-24。规格：[GitHub #13](https://github.com/legendxcheng/ContextSwitcher/issues/13)。

## 自动化结果

- 前端完整 `npm test -- --run`：54 项通过。覆盖移除 ContextSwitcher 读取、新建目录与身份、未知结果核实不重发、明确失败复用操作 ID、已有关联、同步保持选择及离线保留任务。侧栏改为展示目录后，更新对应项目切换断言。
- `npm run build`：TypeScript 与生产构建通过；保留既有超过 500 kB 的 chunk 提示。
- Rust 完整 `cargo test --offline --locked --lib`：53 项通过，2 项原有 Agent live 用例忽略。随后新增目录适配契约测试单独通过；常规测试总计 54 项。新增 workspace live 测试默认忽略，已显式运行通过。
- 项目服务 15 项用例覆盖幂等导入、session 隔离、旧库迁移/歧义保留、目录回退、离线缓存、双向改名、本地删除排除、创建落库失败/响应丢失/并发/重启恢复、其他未决操作标记保护与显式认领保留卡片。均使用真实 SQLite。
- 目录适配契约验证 version 3、无效目录、缺失 identity_cwd、格式损坏、文件缺失和版本变化；明确不以 pane cwd 补齐默认目录。
- 既有任务工作流与生命周期回归通过，包括目录同步后已保存会话继续使用原目录、稳定身份发现/预占及未决操作恢复。
- `cargo fmt --check` 和独立目录中的 Tauri custom-protocol 桌面构建通过。

## 真实 Herdr 与桌面证据

使用 Herdr 0.8.0-preview 的 default session；只创建明确命名的验收 workspace，不向个人 pane 输入。隔离数据库、构建和 WebView2 数据均在 `.scratch/workspace-sync-acceptance` / `.scratch/workspace-sync-build`。

实际运行 `projects::tests::live_workspace_creation_directory_rename_and_local_removal --ignored`，显式提供 `HERDR_ENV=1` 和 `KANBAN_WORKSPACE_ACCEPTANCE_DIR`：

1. 通过正式 ProjectService 与真实 Herdr CLI 创建一个 `kanban-acceptance-workspace-<UUID>`，返回稳定 ID `wV`。
2. 创建后按已知 ID 改名读回，再从 Herdr 侧改名并同步回原 Project；均通过。
3. 有界等待会话持久化，读到 `identity_cwd` 与指定目录一致；重新关联目录来源为 `herdr`。
4. 本地移除后 Herdr workspace 仍存在，自动同步不重新导入；显式添加成功。
5. 对比创建前后真实 workspace 列表，焦点仍为 `w3`，测试 workspace 未获焦点。
6. 真实 Tauri WebView2 启动显示 14 个已有 workspace 及默认目录；测试 workspace 外部新增后，隔离桌面数据库自动出现对应 Project。重启该验收实例后查询，Project ID、workspace ID、名称、目录与目录来源完全保留。
7. 验收结束仅关闭上述确认为本次创建、名称匹配且只有一个 pane 的测试 workspace，停止隔离验收进程。未停止用户原 Kanban 或操作个人 pane。

本次 Windows Computer Use 工具可读取界面，但 click / Tab 未引起预期界面变化；因此新建及改名的真实结果来自后端服务和真实 CLI，不能写作“UI 新建点击验收通过”。

## 尚未实机覆盖

- 通过真实 Tauri 表单点击创建/改名，以及原生目录选择与文件引用根目录交互。
- 真正断开 Herdr 后重连的桌面交互；此行为目前由服务与组件故障测试覆盖。
- 其他 Herdr 版本、非默认 session 自定义持久化文件，以及 Herdr 自身重启后的 identity_cwd 行为。
- 本增量没有启动真实 Codex、重新运行两个 Agent live 测试，也没有安装替换用户正在运行的应用。

## 审查

按 code-review skill 分别执行 Standards 与 Spec 审查。已修复目录等价判断重复、旧未决任务迁移阻塞、已导入 workspace 显式认领、失败操作 ID 复用、重名标记手动消歧和其他操作标记保护。复审两轴均无剩余具体发现。
