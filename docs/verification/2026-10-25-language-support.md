# 中英双语验收（2026-10-25）

## 实现范围

- 应用语言仅提供简体中文 / English，侧栏底部切换，立即生效并保存本机偏好。
- 首次使用、无效偏好或存储读取失败时，中文系统语言（含地区变体）使用简体中文，其他系统语言使用英文；存储写入失败时本次会话仍可切换。
- 补齐侧栏、空态、看板、卡片、派发、确认与反馈文案；旧错误和 Toast 随当前语言显示，外部诊断保留原文。
- 编辑器 placeholder / 无障碍属性、图片控件和日期跟随语言。草稿、优先级、选择、撤销历史和终端连接不因切换重建。
- 项目名、任务名、提示词、图片/引用名称与终端输出不自动翻译。无数据库迁移。
- 原生目录选择 IPC 新增可选标题，旧调用兼容；系统按钮语言由操作系统决定。

## 自动化与构建

| 检查 | 结果 |
| --- | --- |
| `npm test` | 22 个文件，136 项通过 |
| `npm run build` | 类型检查与生产构建通过；保留既有大 chunk 提示 |
| `cargo test --manifest-path src-tauri/Cargo.toml --lib --offline --locked` | 132 项通过，9 项 opt-in live 忽略 |
| `cargo fmt --manifest-path src-tauri/Cargo.toml -- --check` | 通过 |
| `node node_modules/@tauri-apps/cli/tauri.js build --debug --no-bundle` | Windows debug 桌面构建通过 |

Rust 复用 `%TEMP%/kanban-issue1-target`。首次构建发现历史缓存的 Tauri permission 元数据指向已迁移的 E 盘路径，清理该缓存中的 tauri / shell / opener 三个包后重编译通过；未改产品配置绕过权限检查。

新增测试覆盖系统语言、无效偏好、重启初始化、存储异常、词库键/占位参数一致性、保留用户插值文本、双语切换入口、失败草稿和错误更新、撤销历史、选中任务/终端连接不变、终端继续输入、已有通知及统计日期更新。终端与 IPC 使用测试替身，不能据此宣称真实 Herdr 验收通过。

## 浏览器布局与交互

运行 `scripts/verify-language.cjs`，需本机 Playwright / Edge；`PLAYWRIGHT_MODULE` 可指定现有 Playwright 模块路径，`KANBAN_VERIFY_URL` 指向 Vite 服务。

本次使用 Edge、真实 React / Tiptap / xterm，模拟 Tauri IPC：

- `fr-FR` 系统语言首次显示英文；切换后页面重载恢复选择。
- 中文、英文分别检查 900×600、1280×800、1920×1080，语言选择可见、侧栏及页面无新增横向溢出。900 宽看板沿用原有列区横向滚动。
- 切换未增加终端 open / close 调用；终端输入仍能发往原连接。
- 创建失败的中英混合草稿、紧急优先级保留，错误文字即时更新。弹窗打开时通过语言 store 触发切换，原有模态焦点约束不变。
- 检查中英表单和英文统计弹窗截图；900 宽英文 Projects 与数量保持同一行。
- 无页面 JS 错误。

脚本结果和截图：`.scratch/language-verification/`。这些是浏览器证据，不替代以下原生宿主验收。

## 原生 Tauri / WebView2 与 SQLite

使用独立 `KANBAN_DATA_DIR=.scratch/language-native/data` 和独立 WebView2 用户目录。Herdr / Codex 可执行文件显式指向不存在的验收路径，因此没有连接真实 Agent、派发真实任务或使用个人任务数据库。

- 真实 IPC 的 Herdr 不可用错误随界面在中英间切换，底层操作系统诊断保留原文。
- 选择中文、正常关闭独立桌面进程、重开后恢复中文。
- 隔离数据库准备一个验收项目，使用英文界面新建中英混合正文、紧急优先级任务，经真实 Rust IPC / SQLite 保存；编辑读回一致。
- 编辑表单保持打开，以程序触发语言控件 change 验证中文表单仍保留正文和优先级；未绕过任何保存或业务校验。
- 真实统计 IPC 返回并以英文显示；项目名和任务内容保持原文。
- 再次正常关闭并启动桌面进程，恢复英文；重新编辑已保存任务，中英混合正文与紧急优先级一致。

截图和本地复核脚本：`.scratch/language-native/`。验收进程正常关闭；未安装或覆盖用户正在使用的应用。

## 未覆盖边界

本次没有真实 Herdr/Codex 会话下的终端切换验收、NSIS 安装/升级、系统目录选择器按钮语言验收。原生桌面保存偏好使用 WebView2 localStorage；如果本机禁止/清除该存储，语言选择只能保留到当前会话。
