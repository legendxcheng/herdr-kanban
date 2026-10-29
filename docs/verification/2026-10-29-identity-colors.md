# 计算机与 Workspace 身份颜色验证

日期：2026-10-29。

## 实现范围

- `src/identityColors.ts`：计算机和 Workspace 独立分配颜色，以 Machine ID / Project ID 保存本机 WebView 偏好；读取异常可恢复，写入失败时保留内存映射。
- `ProjectSidebar`：机器名称与 Workspace 名称分别着色；机器连接状态圆点保留绿／红／黄语义。
- `KanbanBoard → TaskCard`：来源从拼接字符串改为结构化身份，All Tasks 的普通来源、当前共享工作区进行中紧凑来源及展开详情使用同一颜色函数。
- `App.css` / `taskSummary.css`：状态色限定到圆点，来源使用单行 flex 并分别截断长名称。
- 本次未新增 IPC 或数据库迁移，未执行任务派发或修改远端配置。

## 自动化验证

- `npm test -- src/identityColors.test.ts src/board-tests/summary.test.tsx src/board-tests/navigation.test.tsx`：25 项通过。
- 颜色偏好测试覆盖：重新加载后复用、新身份不挤占旧颜色、命名空间隔离、超出调色板、无效存储和存储不可用。
- App 组件集成测试覆盖：跨机同名 Workspace 不同色、同源任务同色、侧栏与卡片一致、改名、隐藏恢复、顺序变化与离线刷新。
- `npm test`：最终共享工作区复核 39 个文件、265 项通过（包含并行加入的紧凑卡片测试）；首次全量为 38 个文件、263 项通过。
- `npm run build`：TypeScript 与 Vite 构建通过，保留原有超过 500 kB 的 bundle 提示。

## 浏览器验证

通过 Browser 工具在 Edge 加载 `.scratch/identity-colors/index.html`，直接挂载当前源码的 `ProjectSidebar` 与 `TaskCard`，使用隔离样例数据和空操作回调。未连接 Tauri IPC、真实数据库或 Herdr。

- 两台机器、六个 Workspace、三种任务状态：侧栏和卡片的计算样式颜色逐项一致，同机器同色、六个 Workspace 各有不同颜色。
- 将机器切为离线并同时改名：身份色不变，状态点从 `rgb(62,207,142)` 变成 `rgb(242,139,130)`。
- 隐藏再恢复、重新加载页面：原颜色全部一致。
- 窄视口检查（实际 CSS 视口宽 909px）：侧栏 180px，长机器名正确省略，普通来源行高约 18.59px，横向内容不溢出来源行。
- 侧栏各卡片高约 87.60px，改名和状态变化未改变高度。
- 浏览器预览证据：`.scratch/identity-colors/normal.png`、`narrow.png`、`results.json`；样例入口和脚本位于同目录。

## 验收边界

上述浏览器结果验证真实前端组件的样式及本机浏览器偏好；不能等同于安装版 Tauri/WebView2 或真实本机／SSH Herdr 验收。本次未打包安装，也未替换正在运行的应用。颜色保存于本机偏好，清除该存储会重新分配；色值不同不保证大量身份都能凭颜色单独辨识，名称仍保留。

共享工作区同时存在进行中卡片紧凑布局及其他未提交改动；本次保留这些改动，未提交或推送。
