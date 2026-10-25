# Skill 候选布局与滚动验收

日期：2026-10-24。规格：[GitHub #14](https://github.com/legendxcheng/ContextSwitcher/issues/14)。

## 修复与范围

`MentionPopover` 的候选项改为固定符号列与独立文本列，名称上置、描述下置。保留完整文字，列表有界滚动。方向键只滚动候选列表，超高候选从标题开始显示；悬停不触发滚动。搜索、引用存储、IPC 与数据库未改变。

## 真实浏览器证据

使用真实 React/Tiptap 与 Chromium，模拟 Tauri IPC。现有 TaskFormDialog 是统一验收入口，覆盖新建及编辑，不向真实 Herdr 发送任务。

- 修复前相同英文描述测得名称区域宽度 0px，名称可读宽度断言失败。
- 修复后名称区域宽度 307px，描述在名称下方；900×600、1280×800、1920×1080、390×844 的弹层均处于窗口内且无横向溢出。390px 是浏览器压力检查，桌面应用最小窗口仍为 900×600。
- 长名称、中文描述、空描述、超长描述通过；完整描述末尾能通过列表滚动实际进入可见区域。
- 上下键循环及超高条目的标题可见，表单 scrollTop 不变；手动滚动和鼠标悬停不被自动滚动打断。
- 审查补充复现：鼠标静止在首项时按 ArrowUp，旧鼠标进入事件将最后一项 `skill-7` 覆盖为 `skill-5`，回归断言失败。改为仅实际鼠标移动更新高亮后，循环选择与 Enter 插入 `$skill-7` 通过。
- 使用真实鼠标移动到另一可见候选，验证高亮确实变化且 scrollTop 不变；不再以手动派发原生 mouseenter 事件代替 React 的悬停交互。
- Enter/Tab、Escape 先关候选、鼠标选择、长文件路径引用通过。新建后编辑并保存的 Skill mention 保留正确 kind/label/ref。
- 浏览器无 pageerror。

可复用脚本：[verify-mention-popover.cjs](../../scripts/verify-mention-popover.cjs)。使用可解析的 `playwright` Node 包与已安装的 Chromium；本机已有，未改变项目依赖。先在 `kanban/` 启动本地 Vite，再从仓库根目录运行：

```powershell
# 终端一，工作目录 kanban/
npm run dev -- --host 127.0.0.1 --port 1441 --strictPort

# 终端二，工作目录仓库根目录
node kanban/scripts/verify-mention-popover.cjs
```

可用 `KANBAN_VERIFY_URL` 指定其他本地开发端口。`--baseline` 只检查首项布局并输出 before.png，旧代码下应失败。完整脚本输出截图和 results.json 至 `.scratch/skill-popover/`。首次失败证据为 `before.png`，通过后的截图为 `after-900.png`、`after-1280.png`、`after-1920.png`、`after-390.png`。

## 项目检查与限制

- `npm test`：4 个文件，54 项测试通过。
- `npm run build`：TypeScript 与生产构建通过，保留既有大 chunk 提示。
- 本次验证不包含原生 Tauri/WebView2、真实 Skill 文件发现、Herdr 派发或重新打包安装。不能将模拟 IPC 的保存检查作为真实 SQLite 验收。
- 本次提交范围为候选组件、样式、浏览器验收脚本及相关文档；保留共享工作区其他改动，未重启用户应用。

## 代码审查

- Standards：初审未发现产品代码规范违规；指出悬停测试未真实触发 React 事件，以及文档提交状态需要同步。已改为真实指针移动断言，并同步文档。
- Spec：初审发现静止鼠标覆盖键盘选择的问题（P2）。回归先复现失败，再修改鼠标高亮触发方式，浏览器检查通过。
- 最终复审：Standards 与 Spec 均无剩余发现；规格审查独立 Chromium 探针连续 3 次验证静止鼠标下循环选择并插入最后一项成功。补修后 54 项前端测试与生产构建再次通过。
