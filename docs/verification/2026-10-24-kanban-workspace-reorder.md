# Workspace 拖动与 Herdr 双向排序验收

日期：2026-10-24。用户已确认“Kanban 中拖动，Herdr 也跟随调整”的方案并授权实施。此增量扩展既有 #18 的单向导航同步；保留 Herdr 为顺序及在线身份的权威来源。

## 已实现行为

- 当前 Herdr session 的 ready 项目行可以上下拖动，显示插入线；松手按稳定身份移动，拖动不选择项目，同位置不发请求。未关联、其他 session、外来任务/文件拖放不发移动。
- 新增 `move_project(projectId,beforeProjectId)`，后端解析 session + Workspace ID 并检查在线源/锚点。只移动一个 Workspace，保留侧栏中被排除的其他 Workspace 及现有任务身份。
- 通过 Herdr 的 `workspace.move_block` 原子移动；本机 CLI 缺少包装，因此使用同一 CLI 的 `status server` 解析 endpoint，Windows 连接 named pipe。无 Herdr 会话文件写入、安装升级或服务重启。
- 使用实际响应更新原 `workspace_order` 缓存，无新 schema。超时、未知响应及落库失败不自动重发，通过重新读取实际顺序恢复。重排前后的查询代次阻止迟到刷新覆盖新顺序。
- Herdr 外部重排继续由焦点、手动及前台 15 秒同步更新。当前 Project ID 保持时看板及终端不重建。

## 自动化与构建

在共享工作区执行；数字包含其他已有/并行改动，不代表本增量单独新增的测试数量。

- `kanban/`: `npm test`，15 个文件、97 项通过。
- `kanban/`: `npm run build`，TypeScript 与 Vite 生产构建通过；仍有既有大 chunk 提示。
- `kanban/src-tauri/`: `cargo test --offline --locked --lib`，98 项通过、6 项既有独立环境 live 测试忽略。
- `cargo fmt --all --check` 通过；按仓库 CRLF 规则检查的 Git diff 空白检查通过。
- 独立 debug Tauri 可执行文件构建通过，采用唯一 `-o` 输出以保留用户运行中的程序。rustc 对多个输出类型及 `--out-dir` 有两条参数提示，未影响构建。

新增前端回归覆盖：上移/移到底部、当前项目和终端连接保持、外部重排、同位置和非法拖放、超时后实际读回不重发、慢刷新隔离、保存期间禁用拖动、拖后点击抑制。

新增服务回归使用真实 SQLite，覆盖：Herdr 和缓存一致、任务正文与 Project ID 保留、本地排除项不被批量覆盖、响应丢失后读回、session/目标变化拒绝、数据库事务失败后对账、数据库关闭重开及离线缓存。适配器回归覆盖实际 `workspace_list` 响应、异常/重复 ID，以及真实 Windows named pipe 的延迟与分片响应。

## 原生桌面与真实 Herdr

使用 `.scratch/workspace-reorder/kanban-reorder.exe`、独立 `KANBAN_DATA_DIR` 与 WebView2 profile；真实 Herdr 版本为 `0.8.0-preview.2026-08-04-d78e3d3b5126`，协议 19。脚本通过原生 WebView2 的 CDP 驱动真实页面鼠标拖放，正式 Tauri IPC、Rust named pipe、Herdr 与 SQLite 均未替换。

最终验收：

1. 创建带唯一名称的两个专用 Workspace A/B，使用 `--no-focus`；同步导入。
2. 在 A 的真实表单创建含提示词的 todo 任务。
3. 鼠标将 B 拖到 A 上方：Kanban、SQLite 缓存 IPC 与真实 Herdr 列表顺序一致，A 继续选中且任务仍在；成功后无错误 Toast。
4. 鼠标将 B 拖到 A 下方/末尾：两侧与缓存一致，成功后无错误 Toast。
5. 从 Herdr socket 直接将 B 移到 A 前，再触发焦点同步，Kanban 跟随。
6. 页面重载后顺序一致；重新进入 A，任务及提示词对应的数据仍保留。其他 Workspace 的相对顺序保持。
7. 无页面 JavaScript 错误。脚本核验 ID、唯一名称、单 tab/pane 后仅关闭本次创建的测试 Workspace。最终测试资源为 `w1N`、`w1P`，均已清理；其他资源未关闭。

证据位于 `.scratch/workspace-reorder/`：`verify-native.cjs`、`native-results.json`、`native-drag-up.png`、`native-reload.png` 及隔离数据库。

初轮原生检查观察到“外部已移动但界面报无效响应”。检查实际协议及管道后修正两点：RPC 返回 `workspace_list` 而非重排事件类型；Windows NOWAIT 管道的暂时无数据不能当 EOF。真实管道回归先失败再通过；最终原生验收加入“成功不得出现错误 Toast”断言并通过，截图已目视复核。

## 验证边界

- 原生覆盖 Windows WebView2 的鼠标拖放、真实 Herdr 接口双向变化及页面重载。反向变化由真实 socket 发起，未用鼠标操作 Herdr 自身窗口。
- 原生测试任务没有连接 Agent；活动终端在重排时不重新连接，由组件回归覆盖。
- 超时、离线、旧刷新、数据库重开和事务故障由服务/组件/管道测试覆盖；没有中断用户 Herdr 服务或重启它来验收持久化。
- 未执行其他平台、系统 DPI、release 安装包或安装替换验收。用户运行中的原应用未重启。
