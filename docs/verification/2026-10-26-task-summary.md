# 跨 Workspace 汇总看板验收

日期：2026-10-26。规格：[GitHub #22](https://github.com/legendxcheng/herdr-kanban/issues/22)；子任务：[汇总排序 #23](https://github.com/legendxcheng/herdr-kanban/issues/23)、[持续刷新 #24](https://github.com/legendxcheng/herdr-kanban/issues/24)、[任务跳转 #25](https://github.com/legendxcheng/herdr-kanban/issues/25)。后两项仅被 #23 阻塞，已建立 GitHub 原生子任务及阻塞关系。范围为侧栏已显示的本机及远端 Workspace，未归档任务按优先级汇总。用户已确认方案并授权实现；不改变单 Workspace 的等待反馈置顶规则。

## 已实现行为

- 固定「汇总看板」入口、三列与任务数量，卡片标明电脑和 Workspace。
- urgent → high → normal，同级 created_at 升序及 id 稳定排序；批量摘要查询仅访问本地数据库，并校验显示范围。
- 点击任务按最新状态进入所属 Workspace 的 Todo 面板、进行中终端选择或完成任务弹窗。缺失目标显示提示，不先选另一任务。
- 草稿离开保护、前台轮询与启动/标题事件、错误重试、慢查询合并、旧范围响应隔离、稳定空态。
- 中英文，900px 下三列均可见，键盘可操作入口与任务按钮。

## 自动化

- 最终 `npm test`：30 个文件、186 项通过，包含新增 13 项汇总回归。审查修复先在单用例复现，再通过相关 31 项测试，最后执行全量套件。
- `cargo test --locked --manifest-path src-tauri/Cargo.toml`：183 项通过、19 项既有真实环境用例 ignored，无失败；二进制与文档测试均为 0 项。新增两项任务摘要测试使用真实内存 SQLite，覆盖跨机器同名项目、同级时间/ID 排序、隐藏/missing/未选入/归档/删除、重新显示，以及 pending/unknown/failed/succeeded、最新操作和旧请求投影。
- `npm run tauri -- build --debug --no-bundle`：TypeScript、Vite 与 Windows debug 桌面构建通过。保留已有大 chunk 和 Rust dead_code 警告；未生成安装包。
- 组件回归使用模拟 IPC 验证三个状态的精确目标、后台刷新不误开首张任务、目标删除或状态变化、离线缓存、终端查看连接释放、无启动/派发、保存后导航、迟到响应与监听清理。该证据不等同于真实 Herdr。

日志位于 `.scratch/summary-frontend-tests.log`、`.scratch/summary-rust-tests.log`、`.scratch/summary-desktop-build.log`。

## 真实桌面与 SQLite

运行本次 debug 二进制的独立副本，使用独立 KANBAN_DATA_DIR、WebView2 用户数据目录及显式不可用的 Herdr 入口，不访问或修改用户的正式任务库。夹具包含本机、离线远端、隐藏项目、归档任务及跨电脑同名 Workspace。

通过真实 Windows Tauri/WebView2 点击和真实 Rust IPC / SQLite 验证：

1. 跨项目优先级排序和稳定 ID 次序，隐藏、归档任务排除；直接提交隐藏 Project ID 给摘要 IPC 也不返回该任务。
2. 点击远端缓存 Todo 定位原任务，编辑标题及优先级并保存，SQLite 读回一致；返回汇总后按新优先级重排。
3. 未保存草稿进入汇总时取消保留草稿、放弃后导航正常。
4. 完成任务打开对应编辑弹窗。
5. 使用正式 hide_project IPC 隐藏远端项目，重载后汇总排除远端任务。
6. 删除夹具任务后空列在定时刷新前后保持同一 DOM。
7. 1280×800 与 900×600 实际窗口截图、中英文；900px 三列不发生横向溢出。

脚本：`.scratch/summary-board/native_acceptance.py`；结果与截图：`.scratch/summary-board/native/result.json`、`summary-1280.png`、`summary-900.png`、`summary-empty-english.png`。

测试环境排障：初选 CDP 端口 9592 属于 Windows TCP 排除区间 9533–9632，导致窗口已启动但调试端口未监听。改用 19292 后测试完成；未修改应用权限或系统端口策略。

## 未覆盖与交付边界

- 首轮桌面使用离线夹具；后续真实 Herdr／SSH 补充见下节，不将两类证据混同。
- 未执行 release 打包、安装或替换用户运行中的应用，未推送 Git；本次用户调用 implement 已授权当前分支提交。
- 实施期间原有 Agent 启动修复由并行工作提交为 `38ce4df`，本次未改动该组文件。以上 Rust 总体结果包含当前工作区已有实现，不能将其归因于本次汇总功能。

## 真实 Herdr／SSH 补充验收

2026-10-26 在独立 `.scratch/summary-board/live/data` 数据库运行本次 debug 二进制。先验证 Tailscale IP 连通及普通 SSH 身份；仅使用已配置的 devpc 连接，不改 SSH、Herdr 或 Codex 全局配置。通过正式 IPC 新建专用 `kanban-acceptance-summary-4c99cd4-local`（w23）和远端对应 Workspace（w7），分别创建两项本机和一项远端任务，提示词仅要求回复不同固定标记，不读写项目文件。

通过真实 UI 从汇总依次选择 `local_b → remote_a → local_a → remote_a → local_b`：

- 原生 WezTerm/Herdr 窗口每次显示该任务独有标记，不出现其他两项任务标记；本机目标 w23:p3 / w23:p2，远端 w7:p2，机器显示正确。
- 同 Workspace 两任务及跨机器切换均保持准确目标。返回汇总释放本应用查看窗口，底层任务继续存活。
- 导航前后任务派发时间、生命周期操作 ID、目标绑定和进行中状态不变；跳转没有新建会话或重复派发。
- 五次验证后，三个任务均经正式 complete_task 流程完成，无关闭警告。

审查修复另在真实桌面精确恢复本机验收任务，验证从汇总打开 Todo 后经过后台刷新仍不打开其他终端；明确关闭编辑器后，恢复进行中任务并读到原生终端中的原会话标记，导航前后派发时间和启动操作不变。再次正常完成该测试任务并删除纯编辑夹具。结果见同目录 `close-result.json`，脚本 `.scratch/summary-board/live_close_acceptance.py`。

结束时再次核对专用 Workspace 的 ID、名称及仅剩一个空白 shell，关闭本次新建的 w23 和远端 w7；其他 Workspace 和正式数据库未修改。

证据：`.scratch/summary-board/live/fixture.json`、`result.json`；脚本 `.scratch/summary-board/live_acceptance.py`。这些是实际 Tauri、SSH、Herdr 和 Codex 任务证据，不是模拟 Agent。

## 双轴审查与修正

- Standards：发现 1 项 P2，汇总进入 Todo 后首次选择抑制没有在明确关闭面板时解除，导致已有进行中任务仍显示空终端。先在既定 App／IPC 边界新增测试复现缺失 terminal_open，再修复关闭回调恢复默认选择。新增回归同时断言首次跳转及刷新不误连、关闭后连接正确任务、不派发正文；相关 31 项回归及类型检查通过。复核确认 P2 已解决。
- Standards：另有 1 项非阻塞 P3 维护建议，摘要页面与计数 hook 的串行刷新结构相似。本次保留各自数据／错误策略，不扩展为共享查询机制重构。
- Spec：0 项缺失、越界或错误实现发现。审查基线固定为本功能开始前的 `38ce4df`，范围为完整已暂存实现及其后修复；两个轴分别由只读审查代理完成。

TDD 证据：`.scratch/summary-board/tdd-red.log`、`tdd-green.log`。
