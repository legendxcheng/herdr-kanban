# 自动任务标题验收（2026-10-25）

规格：[GitHub #1](https://github.com/legendxcheng/herdr-kanban/issues/1)。基线：`4919ccc4301168f09fbb9da03581b003adf61505`。

## 实现范围

新建表单直接聚焦提示词，不再要求标题。文本、引用或已导入图片均可创建；任务、附件引用及生成意图在同一事务保存。摘要立即可用，独立 Codex CLI 后台命名；纯图片直接使用本地化名称。编辑仍可改名，旧请求不能覆盖用户改名、正文修改或删除/归档；旧编辑表单仅改优先级时保留后台最新名称。

标题模型固定 `gpt-6-luna`、`low`、Standard；不继承开发模型和加速档位。最多 2 个并发进程，进程时限 30 秒、排队时限 60 秒；输入最多 2,000 个 Unicode 字符，输出单行最多 40 字符。失败保留摘要，不重试或升级模型。关闭清理进程，重启结束未完成请求。

## 自动化检查

| 检查 | 结果 |
| --- | --- |
| `npm test` | 21 个文件，128 项通过 |
| `npx tsc --noEmit` | 通过 |
| `cargo test --manifest-path src-tauri/Cargo.toml --lib --offline --locked` | 132 项通过，9 项 opt-in live 默认忽略 |
| `cargo fmt --manifest-path src-tauri/Cargo.toml -- --check` | 通过 |
| `node node_modules/@tauri-apps/cli/tauri.js build --debug --no-bundle` | 生产前端及 Windows debug 桌面构建通过；保留已有大 chunk 提示 |

新增 Rust 测试通过真实 SQLite 和外部生成器替身验证原子保存、内容提取、图片归属、失败兜底、编辑竞态、删除/归档及重开恢复。进程边界测试验证 stdin 原样传递、退出码异常、输出上限、缺失程序、超时与取消。前端沿用 Tauri IPC 替身验证表单焦点、有效内容和旧编辑表单保护；这些结果不替代以下原生验收。

构建缓存因 E 盘空间不足迁至 `%TEMP%/kanban-issue1-target`，后续 Rust 命令设置 `CARGO_TARGET_DIR` 为该目录。日志位于本机 `.scratch/title-frontend-tests.log`、`.scratch/title-rust-tests.log`。

## 真实 Codex CLI

本机 CLI `0.164.0` 使用现有登录，单独运行 opt-in live 测试：

| 输入 | 结果 | 耗时 | 输入 / 输出 token |
| --- | --- | --- | --- |
| 中文 | 修复登录失败提示 | 9,950 ms | 4,996 / 19 |
| 英文 | Fix the login error message | 8,489 ms | 4,991 / 19（缓存输入 3,712） |
| 多行及引用显示名称 | 登录失败时显示错误并保留输入 | 8,384 ms | 5,010 / 23 |

另用本地请求捕获服务核对相同 CLI 参数：工具列表为空，未附带项目规则、技能正文或引用文件内容，Standard 请求不携带加速档位。模型使用量包含 CLI 环境上下文，不能将 2,000 字符业务输入上限当作总 token 上限。捕获产物在 `.scratch/title-request-capture.json`；未写入凭据或业务用户正文。

## 原生 Tauri / WebView2

独立构建进程、隔离 SQLite 和 WebView2 数据目录；可控 CLI 替身制造后台延迟，实际运行 Rust IPC 与 React UI。

- 无标题输入、打开聚焦正文、空内容禁止保存；创建约 78 ms 返回，卡片先显示摘要，随后自动显示生成名称。
- 验证生成期间的优先级编辑、手动改名、删除、项目切换，以及关闭重开后不重新生成。任务均保留正确数据。
- 另将标题生成器换成真实 CLI，通过完整桌面链路生成“为登录表单添加清晰错误提示”，数据库请求状态为 `succeeded`。
- 排队验收连续创建 20 个任务，以每次 8 秒的 CLI 替身运行约 64.4 秒：最高并发 2，16 个成功、4 个因排队超时保留摘要，20 个任务全部保留。

脚本及结果：`.scratch/verify-title-native.py`、`.scratch/verify-title-queue.py`、`.scratch/title-native/results.json`、`.scratch/title-native/queue-results.json`；原生截图 `generated.png`、`real-cli.png` 位于同一结果目录，已检查真实 CLI 截图。

## 真实 Herdr / 开发会话

使用独立 `kanban-acceptance-titles-*` Workspace、真实 Herdr 和实际开发 Codex；标题 CLI 用延迟替身，以确定启动发生在生成完成前。

- 标题请求仍为 `pending` 时 `start_task` 立即受理（本次计时四舍五入 0 ms），生命周期完成为 `succeeded`。
- 读取真实终端确认 Codex 实际回复唯一标记 `TITLE_START_23c536a6`，原生 UI 显示终端已连接。
- 完成并删除本次隔离项目，关闭测试 Workspace；验收前已有 Workspace ID 仍全部存在。未改开发模型。
- 最初验收脚本只读取终端最后 20 行，漏掉已出现的回复；改为 60 行后完整通过。这是验收读取范围问题，没有因此修改产品派发逻辑。

结果与截图：`.scratch/title-live/results.json`、`.scratch/title-live/started.png`；脚本 `.scratch/verify-title-live.py`。

## 验证边界

本次覆盖 Windows 本机真实 CLI、桌面和 Herdr；未运行其他操作系统、干净机器或安装升级验收。没有发布安装包或替换用户当前安装的应用。上述 `.scratch` 文件为本机验收产物，不作为仓库分发依赖。
