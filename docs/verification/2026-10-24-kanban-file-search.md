# Kanban @ 文件搜索验收记录

日期：2026-10-24。范围：任务新建/编辑表单中的文件候选发现、排序、选择与引用保存。

## 实现

- `commands::search_files` 将工作交给 `spawn_blocking → file_search::search`，IPC 参数及 FileEntry 的 `rel_path`/`abs_path` 保持兼容，不新增数据库迁移。
- 使用 [ignore WalkBuilder](https://docs.rs/ignore/0.4.33/ignore/struct.WalkBuilder.html) 读取 `.gitignore`，支持嵌套/否定规则及无 Git 仓库的项目。默认不隐藏所有点目录，`.agents`/`.github`/`.codex` 仍可引用；忽略目录和固有噪声目录在下降前剪枝。
- 大小写不敏感，按文件名/完整相对路径精确匹配、文件名前缀、文件名包含、路径包含、文件名模糊、路径模糊排序；相同模糊等级优先字符间隔较少，再优先较浅/较短路径，以路径顺序稳定决胜。检查全部合格候选，内存中仅保留最优 50 条。
- 支持相对路径、`./`、Windows 反斜杠及 Unicode 查询。输入已有目录前缀时只搜索该子树，允许显式访问 `.scratch/probe/` 等忽略目录；该子树内的 `.gitignore` 和下级噪声目录仍有效。不通过查询访问项目外目录，也不跟随目录符号链接。
- 文件查询 120ms 防抖；变化后清空旧候选，未发请求被取消，迟到响应丢弃。技能查询保持即时，已发出的 Rust 扫描不取消，未新增缓存。

## 复现及自动回归

修复前执行 `cargo test --lib file_search_tests -- --nocapture`：8 项全部失败，其中“80 个 `.scratch` 副本 + 80 个 release 副本 + 1 个正式文件”的查询返回前 50 个 `.scratch` 副本，正式文件没有进入结果。修复后同场景只返回正式文件。

Rust 回归还覆盖嵌套忽略规则、否定规则、没有 `.git`、81 个可见候选先排序再取 50、稳定顺序、文件名与路径相关性、浅/短路径、相对路径、反斜杠、模糊查询、显式忽略子树、点目录及无效项目目录。前端回归覆盖快速输入合并、关闭时取消定时请求、清除旧候选、迟到结果、切项目隔离、保留后端顺序、Enter 选择以及反斜杠/中文触发。

最终执行：

| 命令 | 结果 |
| --- | --- |
| `cd kanban; npm test` | 14 个文件、93 项通过，含本次 5 项文件候选回归 |
| `cd kanban; npm run build` | TypeScript 与生产构建通过；保留既有 500kB chunk 提示 |
| `cd kanban/src-tauri; cargo test --offline --locked --lib` | 88 项通过，6 项既有 Herdr live 测试按原条件忽略；含本次 8 项文件搜索回归 |

## 真实 Windows 桌面

为避免覆盖正在运行的 `target/debug/kanban.exe`，在 `.scratch/file-search/` 建立独立验收入口，依赖当前 Kanban crate 并调用真实 `kanban_lib::run()`。该入口使用 Windows Common Controls v6 manifest，独立 devUrl 1456 加载生产前端；WebView2 CDP 使用 9496，SQLite 和 WebView2 用户数据均独立。通过不存在的 `KANBAN_HERDR_EXECUTABLE` 将 Herdr 保持离线，未调用个人 workspace/pane。

本机重跑入口：

```powershell
$env:TAURI_CONFIG = '{"build":{"devUrl":"http://127.0.0.1:1456"}}'
cargo build --offline --locked --manifest-path .scratch/file-search/Cargo.toml --target-dir kanban/src-tauri/target
python .scratch/file-search/verify_native.py
```

脚本使用真实 Tauri `search_files` IPC 对测试文件和仓库执行查询，并通过真实 React/Tiptap 表单执行以下流程：

1. 临时副本被过滤，81 个可见候选中的最优项仍位于 50 条结果首位；重复查询顺序相同。
2. 相对路径、反斜杠、模糊文件名及显式 `.scratch/probe/chk` 查询正确；`.agents` 文件可见，`../` 无结果。
3. 新建表单输入 `@MentionPopover`，候选只含正式文件；Enter 选择，创建任务后重新编辑，引用路径不变。
4. 编辑时输入 `@.scratch/probe/chk`，鼠标选择临时文件，保存并刷新；重新编辑及直接读取 SQLite 均确认保存的是正确 file mention。
5. 切换到真实 `E:/ContextSwitcher` 项目，查询 `MentionPopover`：`kanban/src/MentionPopover.tsx` 排第一，第二项为模糊匹配的 `kanban/scripts/verify-mention-popover.cjs`，没有 `.scratch`/release 结果。含 IPC 的单次查询观测 21.57ms，不作为性能保证。

结果：[原生验收 JSON](kanban-file-search-20261024/native-results.json)、[真实仓库候选截图](kanban-file-search-20261024/repository-candidate.png)、[刷新后编辑引用截图](kanban-file-search-20261024/saved-reference.png)。没有页面脚本错误；退出时关闭本次创建的应用及 preview 进程，临时文件夹已清理。

未执行安装器打包/安装、Herdr/Codex 派发或索引压力测试；本次文件搜索验收没有替换或重启用户应用。仍使用旧二进制的实例需要重新构建并重启 Kanban 才会加载新后端。
