# Session 资源跟踪与全局清理验证

规格：[GitHub #53](https://github.com/legendxcheng/herdr-kanban/issues/53)，实现任务 #54–#59，完整验收 #60。

## 实现与审查

实现提交：`fb2bfc7`、`8d965e1`、`f349225`、`3e281a8`。新增独立资源台账、删除后有界后台关闭、全局预览和确认、历史未知结果核实、精确进程退出证据、本机与 SSH 路由。磁盘对话历史不删除，InProgress 绑定与普通 shell 保留。

规范审查：未发现未解决的规范问题。规格审查提出的旧目标误解除新绑定、关闭期间新子进程证据丢失、替换目标被旧 unknown 永久占用、执行代次丢失以及旧进程借用后继证据，均已修复并复核。迟到核实错误不能覆盖已确认成功，也已补充回归。

## 自动验证

| 层次 | 结果与范围 |
| --- | --- |
| TypeScript / 前端生产构建 | PASS；保留已有大 chunk 提示 |
| 前端全量 | 47 文件、352 项 PASS；随后新增两项清理交互测试，清理专项 3/3 PASS |
| Rust 全量串行 | 当次 422 PASS、2 FAIL、44 ignored；其中旧目标迁移触发器兼容回归已修复并专项通过 |
| Rust 剩余全量失败 | 原有 `four_remote_clients_survive_idle_and_switching_until_machine_disable_or_shutdown` 在 `Instant::now() - 3600s` 发生时间下溢，单独复跑仍失败；该测试及相应行为未修改 |
| 最终清理工作流 | 19/19 PASS：真实 SQLite、并发、幂等、删除、历史 unknown、身份替换、旧进程证据隔离、重开数据库 |
| 关闭回执证据 | 1/1 PASS；执行电脑持久化合并的子进程证据，回执丢失仍可核实 |
| 生命周期 | 串行 103 PASS、2 ignored |
| 原单 Workspace 空闲清理 | 11 PASS、1 ignored；其原资格规则不变 |
| 最终桌面构建 | `cargo build --offline --locked --features tauri/custom-protocol` PASS |

完整 Rust 回归并非全绿；没有把跳过项或既有失败计为通过。最后补丁采用针对性回归，没有再次重复全量套件。

## 真实桌面与 20＋20 循环

仅使用隔离数据目录、专用 `kanban-cleanup-20261112` Herdr session 和专用 SSH profile。生产任务库、用户默认 session 未用于清理。工件保存在 `.scratch/session-lifecycle-implementation/live/`。

真实 Tauri 桌面确认入口与正式 IPC 贯穿工作流、SQLite、本机及远端 helper。每轮本机和 SSH 各创建一个真实 Codex，依次轮换无绑定、完成、删除、重置四种场景，共各 20 轮。归属卡片由隔离 SQL fixture 建立，后续操作使用正式工作流；这不代替每轮首条提示词启动链路验收。

- 40 个目标均退出；每轮独立检查目标 Pane 消失，以及本机 psutil / 远端 Get-Process 对已采样 PID 的存活情况，共 480 条进程观察均为空。
- 每台保留一个 InProgress Codex 和一个执行普通命令的 shell，每轮 Pane 身份集合保持不变。该检查证明目标保留，不等同于每轮手工输入交互验证。
- 预热后第 3–20 轮未决非保护资源为 0。此计数来自 `8d965e1` 构建；最终严格证据隔离补丁可能把无旧进程证据的历史记录保留为 absent/unknown，不能据旧计数宣称最终版本台账全部已释放。
- 关闭路径以 PID＋创建时间核实退出，执行电脑落盘关闭前新增子进程证据。独立 PID 抽查不替代正式身份核实。

| 第 3–20 轮本机私有内存 | 最小 | 最大 | 最后一轮 |
| --- | ---: | ---: | ---: |
| Kanban 主进程 | 15.27 MiB | 16.59 MiB | 16.59 MiB |
| 专用 Herdr server | 16.24 MiB | 17.50 MiB | 17.48 MiB |
| 查看客户端 | 91.04 MiB | 91.20 MiB | 91.14 MiB |

Kanban 主进程有约 1.1 MiB 增量，包含持久批次查询/缓存，不足以证明所有内存泄漏已消除。WebView 子进程未完整纳入此表；远端命令行过滤采样到的 Herdr 相关进程总量约 2.59–2.63 MiB，未证明包含完整宿主进程树，不能作为完整 SSH 宿主内存验收。目标进程退出的硬证据与宿主缓存趋势分开报告。

循环记录：`cycles.json`；截图：`protected-preview.png`、`closed-results.png`。循环版本为 `8d965e1`；最后证据隔离补丁另做最终构建桌面复测，不将旧构建循环冒充最终版本重复 20 轮。

## 最终构建桌面复测

`3e281a8` 构建重启隔离应用后，旧批次仍可读取；隐藏本机项目后，其 InProgress 仍受保护。本机和 SSH 分别新建未导入看板的第二个 Workspace，通过英文全局按钮确认关闭其中 Agent，最终 UI 显示 Released 2 / retained 2 / failed 0 / unverified 0。两个普通 shell 和两个保护目标保留；停用 SSH 连接后预览明确列出 machine.disabled。All tasks 页面可打开同一全局入口。截图已核对最终结果，见 `final-multiworkspace.png`、`all-tasks-preview.png`，结构化证据见 `final-result.json`。

最终台账保留 3 条无完整退出证据的早期历史资源，没有借后继进程退出证据把它们虚报为 released；这与本批两个目标成功释放并不冲突。

## 历史恢复证据与限制

独立 `kanban-cleanup-history-20261112` session 通过真实 `start_task` 派发测试提示词并收到 Codex 回复；首次完成保存 UUID、关闭原目标成功。再次启动的进程 argv 使用精确原 UUID，终端显示原提示词与回复；未重放提示词。

自动恢复核验返回 `flow.session_unverified`，因此这项完整用户流程未通过。人工绑定修复后派发计数仍为 1，且 InProgress 保护生效。原有人工绑定逻辑会清除当前任务的恢复指针；之后完成因缺少新 UUID 报 `flow.session_missing`，全局清理仍可关闭目标，旧 UUID/cwd 保留在资源台账及磁盘历史。没有为让验收通过而削弱现有恢复核验。证据：`history/result.json`、`history/english-after-resume.png`。

## 交付边界

实现已提交，完整验收 #60 和父规格 #53 保持开放：自动恢复核验、完整宿主进程树内存采样以及全部桌面交互矩阵仍有缺口。不会把自动测试、真实目标退出、对话内容可恢复和完整端到端验收合并为同一个 PASS。未制作安装包、未安装、未推送或发布。
