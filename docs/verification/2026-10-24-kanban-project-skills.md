# 项目技能发现修复验收

日期：2026-10-24。

## 问题与修复

Prompt 编辑器的 `$` 搜索此前只扫描用户 `.claude`，IPC 没有项目目录参数，因此项目 `.agents/skills/kanban-dev/SKILL.md` 不会进入候选。

已在 `MentionPopover → api.searchSkills → commands::search_skills` 传递项目目录。搜索按项目 `.agents/skills`、用户 `.agents/skills`、用户 `.codex/skills`、用户 `.claude` 的优先级排序，再按名称及路径确定稳定顺序。同名或重复路径只保留优先项；筛选关键词后取 30 条。缺失目录和不可读文件跳过。旧 IPC 调用可省略 rootPath，继续搜索用户目录。

数据库与引用格式不变；保存名称及路径，派发继续保留 `$技能名`，不展开 SKILL.md 正文。

## 自动验证

- 修复前运行 `npm test -- src/skillDiscovery.test.ts`：项目目录传参断言失败，实际请求仅含 query。最终将该测试加强为 `skillDiscovery.test.tsx`，驱动真实 Tiptap、MentionPopover 和 API 边界，验证项目候选选择、序列化、原生技能发送及目录切换；只模拟 IPC 和 jsdom 缺失的布局。
- `kanban/` 下 `npm test`：84 项通过。
- `kanban/` 下 `npm run build`：类型检查与生产构建通过；保留既有超过 500 kB 的 chunk 提示。
- `kanban/src-tauri/` 下 `cargo test --offline --locked --lib`：78 项通过、5 项原有真实 Herdr 测试跳过。
- 新增 5 项 Rust 文件系统回归，覆盖项目技能、项目隔离、没有用户目录、用户各来源、旧插件目录、同名优先、重复根路径、稳定排序、超过 30 条后仍可搜索、缺失目录、非 UTF-8 文件和无名称元数据回退。

## 浏览器与真实目录

`.scratch/project-skills/build-probe.ps1` 从当前 commands.rs 提取原始技能发现函数，移除 Tauri 命令属性后编译为本地探针，不另写一套扫描逻辑。探针查询 `E:/ContextSwitcher` 的 `kanban-dev`，返回真实项目 SKILL.md 路径及描述。

`.scratch/project-skills/verify.cjs` 使用真实 Chromium、生产构建的 React/Tiptap，搜索请求桥接探针，其他 IPC 与任务持久化使用内存模拟。验证：

- 新建任务输入 `$kanban-dev`，显示唯一正确名称及完整描述。
- Enter 和鼠标均能插入技能引用，路径为 `E:/ContextSwitcher/.agents/skills/kanban-dev/SKILL.md`。
- 新建保存、编辑保存及重新打开保留技能引用。
- 切换至其他目录后不沿用该项目技能候选。
- 页面无未处理异常。

结果：`.scratch/project-skills/results.json`；截图：`.scratch/project-skills/candidate.png`。

复现命令：根目录运行 `.\.scratch\project-skills\build-probe.ps1`；在 kanban 目录运行 `node node_modules/vite/bin/vite.js preview --host 127.0.0.1 --port 1448 --strictPort`，再在根目录运行 `node .scratch/project-skills/verify.cjs`。

## 验收限制

未执行原生 Tauri/WebView2 IPC、真实 SQLite 编辑保存或 Herdr/Codex 实际技能加载验收。未重新打包、安装或替换正在运行的桌面程序；Rust 修改需要重新构建并启动桌面应用后生效。当前仍是单行 frontmatter 解析，未扩展祖先项目目录、自定义 CODEX_HOME 或插件目录协议。
