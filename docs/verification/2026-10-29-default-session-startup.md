# 默认会话启动校验修复

日期：2026-10-29。范围：本机默认 Herdr 会话的启动状态校验与既有重试提示恢复。

## 原因与复现

截图显示 `Sync failed; local projects and tasks are retained. Herdr returned invalid data`。本机 Herdr 0.9.5 实际返回 `running: true`、`compatible: true`、`endpoint_compatible: true`，但 client/server 的 `session` 均为显式 null。`session list --json` 确认相同 socket 属于 `default`；Workspace 列表正常返回。

原 `status_running` 将 null 与字符串 `default` 直接比较，导致 `herdr.invalid_response`。启动失败被前后端缓存，普通焦点/定时刷新不重新检查启动；显式重试仍撞到同一判断。本地缓存及其他任务/终端链路可用，但本机 Workspace 同步被阻断。

修复前新增回归并运行 `cargo test --offline --locked --lib herdr_startup::tests::default_session -- --nocapture`：3 项中 2 项失败、1 项通过；默认在线状态实际为 `Err("herdr.invalid_response")`，预期 `Ok(true)`。

## 实现

- `herdr_startup.rs` 的状态解析分别读取 client/server session，目标为 `default` 时接受显式 null 或精确字符串 `default`。
- 使用 `get` 区分缺失字段与显式 null；非默认会话仍要求精确名称。拒绝缺失、错误类型、空名称、其他会话及非默认目标下的 null。
- 未改变运行状态、协议兼容、仅明确离线才启动、并发合并、失败缓存及显式重试规则。
- 前端产品逻辑无需调整；已有成功同步会清除错误。重试回归增加 `herdr.invalid_response` 场景。

## 验证

| 检查 | 结果 |
| --- | --- |
| `cargo test --offline --locked --lib herdr_startup::tests -- --nocapture` | 11 通过、2 项环境依赖用例默认忽略 |
| `npm test -- src/board-tests/herdr-startup.test.tsx` | 3 通过：等待启动、超时恢复、无效返回恢复；后两项验证保留卡片、不自动重启及点击重试后清除提示 |
| `npm run build` | TypeScript 与 Vite 生产构建通过，保留现有 chunk 大小提示 |
| 显式执行 `herdr_startup::tests::windows::live_default_session_status_is_accepted` | 1 通过：真实默认 Herdr 经正式 `LocalHerdrStartup::probe` 和 CLI 命令执行器探测成功，Workspace 列表读取成功 |

真实默认会话用例通过 `KANBAN_STARTUP_ACCEPTANCE_EXECUTABLE` 指定 `Get-Command herdr` 解析的本机 0.9.5 入口，在允许访问当前管理员 Herdr 的环境运行。该用例只执行状态探测和列表查询，不调用 `ensure`、不启动/停止服务，也不发送任务。

本次未重跑独立命名会话的真实冷启动用例；默认会话在线/离线表示、身份拒绝及协议拒绝由解析回归覆盖，启动/超时/重试/不重复启动由既有模块测试覆盖。前端测试使用模拟 IPC，不等于安装版桌面点击验收。

工作区原有其他未提交修改保留。本次未生成安装包、未替换运行中的安装版、未执行安装后桌面验收，未将全仓库测试标为通过。

## 提交隔离验证

提交前发现本机启动模块及接线尚未进入 HEAD，因此将该必要依赖与本次默认会话修复一并纳入；按文件/改动块排除任务重置、SSH 接入和并行界面改动。由暂存区导出独立快照后，`npm run build`、前端启动/重试 3 项、Rust 启动模块 11 项再次通过（2 项真实环境用例默认忽略）。此次检查验证提交不依赖其余未提交源码；此前真实默认会话只读验证结果不变。
