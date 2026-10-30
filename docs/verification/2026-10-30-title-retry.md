# 自动任务标题重试验证

日期：2026-10-30。

## 实现

- schema 19 为 `task_title_requests` 增加 `attempts`、`next_attempt_ms` 和到期索引。临时超时、通用 CLI 失败、I/O、无效输出以及退出取消，按 10 秒、30 秒、120 秒退避，含首次共最多 4 次。CLI 通用失败尚不能区分登录失败与网络故障，因此也按这个有限预算处理。
- 机器停用、身份不匹配、Codex 入口不可用、输出超限或出现开发工具调用等直接保留摘要并停止自动重试；不切换模型或执行电脑。
- 取消原排队 60 秒后直接失败的规则。两个 worker 按到期时间和创建顺序取任务；退避中的请求不占 worker。保留单次本机/远端模型 30 秒、远端部署及请求预算。
- 正常关闭取消在途工作，按已有尝试次数保存退避；重新打开保留 pending/retry_wait 计划，异常遗留 running 转入退避或次数耗尽失败。重启不会重置次数。
- 首次升级不自动补跑旧失败或中断记录，旧 pending/running 转 interrupted。用户通过“重试标题”重新读取当前正文，产生新请求 UUID 和版本；既有手动修改无法从旧数据完整推断，因此不批量覆盖。
- 新 IPC `retry_task_title(id, requestId)` 与 `Task.title_generation` 投影。三列卡片均显示明确的“重试标题”按钮或等待状态，按钮悬停显示失败原因，等待状态悬停显示下一次尝试和预计时间。重复点击合并，不启动开发任务或切换终端。
- 结果写回核验任务、请求 UUID、编辑版本、尝试次数和 running 状态。名称/正文修改及归档使等待、运行、失败和中断请求失效；删除级联撤销。仅修改优先级仍保留后台生成标题。

## 自动化结果

- `cargo test --manifest-path src-tauri/Cargo.toml --lib -- --test-threads=4`：311 项通过、33 项环境依赖测试默认忽略。共享工作区包含其他进行中的提交观察修复，此结果验证当时整体工作区，不代表独立提交或真实桌面验收。
- 最后一次队列到期时间调整后，`cargo test --manifest-path src-tauri/Cargo.toml --lib title -- --test-threads=4`：28 项通过、3 项 live 忽略。覆盖生产 worker 失败后自动再取任务、退避时其他任务继续、预算耗尽、旧库幂等迁移、磁盘数据库重开、旧请求/旧尝试结果拒绝、手动重试合并、改名/正文/归档/删除撤销，以及本地/远端路由。
- `npm test`：40 个文件、283 项通过。最终将重试图标改为文字按钮后，标题与紧凑卡片 12 项再次通过；此前标题/紧凑卡片/语言合计 18 项通过。覆盖三列入口、双击去重、等待次数、中英切换、成功事件刷新及保留未保存编辑草稿。这些是模拟 Tauri IPC 的组件检查。
- `npm run build` 通过，保留现有 bundle 大小提示。`cargo build --manifest-path src-tauri/Cargo.toml --bin kanban` 通过，生成 debug 构建；保留已有未使用成员警告。
- 改动 Rust 文件的 rustfmt 检查通过；`git -c core.whitespace=cr-at-eol diff --check` 通过，按仓库现有 Windows 行尾检查。
- 旧路由测试使用一次 `try_lock` 判断模型调用不持数据库锁，会与第二个 worker 的短事务偶发竞争。本次改为最多 1 秒内确认锁可用，仍能识别模型调用自身持锁；首轮失败后全量及标题定向检查通过。

## 真实本机 Codex

执行：`cargo test --manifest-path src-tauri/Cargo.toml --lib live_codex_title_succeeds_after_automatic_retry -- --ignored --nocapture`。

隔离内存 SQLite 和正式 `TaskTitles` worker，首次生成器返回可控 `title.timeout`，之后按生产 10 秒退避，第二次使用真实本机 Codex。没有调整到期时间或调用手动重试。约 19.92 秒完成，状态 succeeded、attempts=2，得到“登录失败提示原因并保留账号”。首次超时为故障注入，第二次模型调用和标题写回是真实执行；不修改用户任务数据库。

## 真实 SSH 尝试与边界

使用既有 `.scratch/title-ssh/machine.json` 的验收目标。只读 SSH 连通性探测成功。随后执行 `live_ssh_title_uses_remote_codex_with_local_codex_unavailable`，指定本次 debug helper，并令本机 Codex 入口不可用，以检查远端链路不回退本机。

该测试约 185.09 秒后在 helper 安装调用返回 `machine.timeout`，尚未进入远端标题模型调用。真实 SSH 标题闭环未通过，不能将路由模拟测试当作远端验收。没有重发任何用户开发任务或修改本机任务数据库；此次尝试可能在远端 helpers 目录留下部署上传临时文件。

未替换运行中的安装版，未生成 release/NSIS 安装包，未进行真实原生桌面按钮操作或人工视觉验收。已存在的失败任务需在运行新版本后点击“重试标题”。
