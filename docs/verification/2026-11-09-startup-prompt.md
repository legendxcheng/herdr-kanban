# 首条提示词随启动输入验证（#48）

日期：2026-11-08 至 2026-11-09。规格：[GitHub #48](https://github.com/legendxcheng/herdr-kanban/issues/48)。实现基点 `2cd246a`，保留此前工作区未提交改动；基线快照 `.scratch/startup-prompt-implementation/baseline/`。

## 结果

| 层次 | 结果与证据 |
| --- | --- |
| Workflow + SQLite | 生命周期 86 passed / 1 ignored；含启动意图先落盘、状态波动、回执先于响应、迟到回执、真实文件重开、事务失败、取消／删除／重置／机器停用、身份替换、旧 not_sent 兼容及明确未启动后的重试 |
| 原生 argv / 启动器 | capture.exe 实际接收中文、多行、空行、tab、单双引号、反斜杠、美元符号、前导连字符、末尾技能标记和长正文；模型、思考强度、目录保持。子进程首个效果前验证身份文件存在；身份落盘失败时挂起子进程未执行 |
| 图片 | 准备后、创建 pane 时删除缓存，零次启动 Codex；本机／SSH 真实矩阵包含图片原图路径及 view_image 指令 |
| 前端 | 44 files / 327 tests passed；新“确认送达”阶段、刷新无续发、旧入口兼容。TypeScript 与生产前端构建通过 |
| 本机真实矩阵 | 10/10 自动送达、唯一精确回执；`.scratch/verify-startup-input-local-final2/results.json` |
| SSH devpc 真实矩阵 | 10/10 自动送达、唯一精确回执；`.scratch/verify-startup-input-ssh-final2/results.json` |
| 原生 Tauri / WebView2 | 本机 10/10、SSH 10/10，经真实 IPC 创建／启动；刷新及应用重开 sent 计数不变。`.scratch/verify-startup-input-native-{local,ssh}-1/` |
| 可见界面／终端重连 | 两端 All Tasks 显示 10 个进行中任务；选择原任务显示“Herdr 已打开”，刷新后恢复查看，派发计数不变。对应 `visible-success.png`、`visible-reconnect.png`、`visible.json` |
| 丢响应／重开故障演练 | 两端各 1/1。实际启动后在适配器边界丢弃响应、暂时拒绝观察，SQLite 关闭重开后用真实 CLI 核实原进程及精确回执；每端仅一次 launch。各矩阵目录 `lost-response.json` |
| 全量 Rust | 首轮默认并发 366 passed / 7 failed / 41 ignored；限制 4 线程后 371 passed / 2 failed / 41 ignored。剩余失败均为本轮未修改的标题 HTTP 测试，见下方说明 |
| 构建／审查 | Rust debug 应用构建通过；Spec 与 Standards 两路审查提出的恢复身份落盘、明确失败恢复、图片重验问题已修复并复核 |

正常矩阵每端包括短中文、多行转义、前导 `--`、约 12 KB、27,691 bytes 正文、末尾技能引用、文件／技能引用、图片及混合引用。每端日志恰有 10 个 `startup_request/attempt`，没有正文粘贴或 Enter 提交阶段；每次独立操作的原生 claim 防止重复启动，成功由会话唯一精确用户消息判定。Codex 0.167.0、Herdr 0.9.5；本机 standalone、远端 npm 安装均经过真实链路。

## 保留的失败与修正

1. `verify-startup-prompt-probe`：Herdr agent start 拒绝多行参数，错误为 `invalid_agent_argument`。随后原 pane 内短启动器直接调用原生 Codex，13 KB 多行实际回复；未采用模型读文件替代正文。
2. `verify-first-prompt-acceptance-startup-local-1`：新 pane 创建回执尚无 shell PID，过早严格比较导致 `binding.identity_changed`。修正为只读取得本次新 pane 的 shell 身份后固定，未绕过真实身份检查。
3. `verify-first-prompt-acceptance-startup-local-2`：修正后首轮本机 10/10。
4. `verify-first-prompt-acceptance-startup-ssh-1`：远端 npm 的原生路径为 `vendor/.../bin/codex.exe`，原解析缺失；保留失败并补齐安装布局识别。明确的启动准备失败立即返回，不再等待送达预算。
5. `verify-startup-input-{local,ssh}-final1`：以 GUI subsystem 测试发现 PowerShell `&` 不等待 GUI 应用，父控制台提前退出。改为 `Start-Process -NoNewWindow -Wait`，并以 `output` 接收 pane run 的非 JSON 成功输出。
6. `verify-startup-input-{local,ssh}-final2`：采用修正后的完整矩阵从第一项重跑，各 10/10。验证用 debug 二进制的 PE subsystem 改为 Windows GUI，以覆盖发布版控制台行为；这不是正式 release 打包／安装验收。

所有失败目录及后续重跑记录保留，未将失败项从正常矩阵中挑除。原生 UI 初次截图拍在异步同步完成之前，不能证明可见卡片；后续 `native_view.py` 等待真实项目、卡片及终端状态后补充截图与无重发断言。

## 回归限制

全量 Rust 首轮与全量前端／真实桌面并行时，4 项后台测试触发 3 秒夹具截止时间，另 3 项标题 HTTP 测试返回 `title.network`。限制 4 线程后所有后台测试通过，标题测试仍有 2 项失败：`local_and_offline_ssh_tasks_use_same_http_contract_and_commit_titles`（Disconnected）及 `http_failures_keep_summary_and_only_transient_errors_schedule_retry`（预期 HTTP 错误，实际 title.network）。标题子集单线程也出现不同样本的网络失败；`title_deepseek.rs`、`title_deepseek_tests.rs` 与本轮基线逐字节一致。本次没有修改标题实现或放宽其断言，不能宣称全量 Rust 全绿。完整日志在 `.scratch/startup-prompt-implementation/`。

Windows 原生命令行总长度仍受约 32K UTF-16 单元限制，包含路径、设置及转义开销；旧 pane send-text 同受该系统边界。超限明确失败，不截断、不退回输入框提交。本次正常样本全部成功不表示断网、崩溃等情况下数学上的 100% 即时送达。

故障演练是在真实启动后于适配器边界主动丢弃返回，并非拔网线或终止 Herdr server。应用重开、查看恢复、单次启动与精确回执已经实测；物理断网、服务重启另有边界，不据此宣称全部覆盖。launcher 在创建挂起进程后被强制终止可能留下尚未执行的挂起进程，保持 unknown，不能自动重放。

未打包、安装或替换正在运行的业务应用。运行验收使用当前共享工作区快照，包含既有未提交改动；本次提交仅收录 #48 增量。专用 Workspace 清理单独记录，不操作现场业务卡片。

## 独立提交与清理

以 HEAD `2cd246a` 加本次 27 个文件增量构造隔离源码副本，生命周期 86 passed / 1 ignored、原生 argv／挂起执行测试通过，排除既有 dirty 改动带来的编译／核心工作流依赖。运行二进制 SHA-256 和最终源码摘要见 `.scratch/startup-prompt-implementation/manifest.json`。

已逐个核对当前 Workspace ID 与本次生成标签后关闭本机 `w2F/w2G/w2H/w2J/w2K/w2M`、远端 `wS/wT/wV/wW`，随后确认全部从列表消失；记录为 `cleanup-results.json`。仅清理本次专用测试 Workspace，数据库、失败日志、回执、截图与命令脚本继续保留。
