# 完成后恢复失败的人工绑定修复

日期：2026-11-12。

## 故障与复现

自动恢复在创建窗格后失败会留下 unknown 生命周期记录。人工绑定先调用 `ensure_idle`，因此在检查选中终端前报 `flow.verify_required`（英文为 `Resolve the pending launch or dispatch on this task first`）。原弹窗只检查未决派发，允许用户选终端并保存。

新增 Rust 回归经真实 Workflow/SQLite、Herdr 替身执行首次启动、完成、恢复会话不匹配、人工绑定，修改前在 repair 得到完全相同的错误。命令：`cargo test --manifest-path src-tauri/Cargo.toml --offline --locked --lib completed_task_can_bind_after_failed_resume_without_replaying -- --test-threads=1`。新增前端两项 pending/unknown 保存按钮回归修改前均失败。

本次未取得用户故障任务标识，不能断言用户原始恢复失败也是会话不匹配；已复现并修复恢复失败后的人工绑定阻塞。

## 实现

- 只有已停止的恢复且没有本次提示词写入时才允许人工接管；依据启动快照 `needs_prompt=0`，兼容无快照但保存过会话且已派发过的旧任务。真正 pending、未决派发和未知关闭保留原处理路径。
- 人工选择仍只要求目标有效、归属正确且未被占用，不依赖会话 ID 上报或历史会话一致。
- 修复与自动核验共用任务锁；外部查询返回后在事务中重查操作、绑定、机器、占用和执行代次。绑定、旧会话定位清除、旧失败记录保留、新人工成功记录、任务回到进行中及代次更新原子提交。
- 旧恢复错误和目标保留。新成功记录 source 为 `manual_binding`，保存前一操作 ID 与人工选择目标。迟到激活和历史错误回写不能覆盖新绑定。
- 增加派生字段 `Flow.can_repair_resume`，前端与后端共用判断。无数据库迁移或新增 IPC 命令。

## 自动化结果

- 生命周期：103 passed、2 ignored。包含本机／SSH 替身、同一／另一终端、会话未上报、旧数据兼容、真实后台 submit_start 入口、SQLite 关闭重开、写入失败回滚、目标占用、观察期间操作变化和迟到回写。
- 工作流：17 passed、7 ignored，涵盖占用、派发、归档及删除竞争。
- 全量前端：46 文件、347 passed，包含完成后 unknown 恢复的选择、保存、移列和打开终端，以及 pending/非可修复 unknown 禁止保存。
- `npm run build` 和 custom-protocol Windows debug 桌面构建通过。保留原有大 chunk 和未使用代码提示。完整 Rust suite 未重跑。
- `git -c core.whitespace=cr-at-eol diff --check` 通过；仓库已有 Rust CRLF 行尾按该规则检查。

## 隔离原生桌面与真实 SSH 验收

使用当前源码构建的 Tauri/WebView2 程序、独立 SQLite 和独立 WebView 数据目录。隔离任务预置为已完成、恢复 unknown、保留旧会话定位，连接实际 devpc 的 `wP:p2`。验收前实时 `agent get` 确认目标存在且 idle、未上报会话 ID。

1. 在实际界面选择任务，打开修复绑定，选择 `wP:p2` 并保存。
2. 弹窗关闭，任务变为 in_progress，完成时间清空、执行代次增加；原 unknown 操作变为 failed 且保留原错误，新成功记录保存人工选择证据。
3. 原生 Herdr 查看连接就绪，独立应用子进程中存在 WezTerm。
4. 刷新页面、关闭并重启独立应用后均重新打开绑定终端。
5. 旧自动恢复定位已清除，派发记录总数为零。

脚本和结果：`.scratch/resume-binding-desktop-20261112/accept.py`、`fixture.json`、`result.json`、`connected-webview.png`。数据库使用旧隔离验收库的副本，未写入用户实际任务数据库。按正式机器路由部署内容寻址 helper，机器配置只更新在隔离库中；未替换 Herdr server，未发送对话输入，未关闭用户 Agent。截图仅记录 WebView 状态，不作为原生终端内容的视觉证据。

桌面验收从预置 unknown 恢复记录开始；真实外部恢复失败的产生由 Workflow/Herdr 替身回归覆盖，不宣称现场重现了用户原始恢复失败原因。未生成新安装包、未安装或推送。
