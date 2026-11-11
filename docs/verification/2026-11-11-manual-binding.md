# 人工绑定不依赖会话 ID

日期：2026-11-11。替代上一轮把人工绑定纳入自动会话恢复核验的行为。

## 真实问题

用户选择远端 forge-oasis 的 `wP:p2`，对话标题为“编写枪械战斗与养成数值模型GDD”。真实 `agent get` 返回 Codex idle，但没有 `agent_session`；`agent read --source visible` 能读到对话，前台进程为 npm Codex 普通启动，没有 `resume <UUID>` 参数。对话可用，但旧实现要求取得会话 ID 并匹配任务的旧 ID，因此人工修复被拒绝。

## 最终行为

- 人工保存选择时确认终端/进程存在及占用关系，不调用会话识别，不以历史会话否定用户选择。
- 普通有效绑定的打开和持续查看只检查终端/进程与绑定状态，不查询会话。没有 ID、会话不同均可连接；进程被替换则重新选择。
- 绑定替换与清除旧 `task_sessions` 自动恢复定位记录同事务提交，不删除实际 Codex 对话或派发历史。完成任务时从当前终端记录其实际会话，以便以后恢复；缺少恢复信息时不会误用旧 ID。
- 未决派发只读查看、自动恢复、完成前收集会话等流程仍独立处理；人工绑定不重发正文、不启动或关闭 Agent。
- 更新中英文弹窗文案，提示选择已有对话所在终端；不再要求新准备 Agent 或等待会话上报。

## 自动化验证

修复前，新增 `manual_binding` 三项测试均失败：绑定和查看调用了不应依赖的会话查询；人工选择另一段对话被旧会话匹配拒绝。

修复后：

- `cargo test --manifest-path src-tauri/Cargo.toml --offline --locked --lib lifecycle::tests -- --test-threads=1`：100 通过、2 跳过。覆盖本机/SSH 替身、无 ID/不同 ID、实际 SQLite 关闭重开、页面查看与持续授权、所选进程变化、保存期间本地状态变化、完成后记录新会话并精确恢复且不重发。
- `workflow::tests`：17 通过、7 跳过，保留跨任务占用、派发恢复及归档/删除竞争验证。
- `npm test`：45 个文件、336 项通过。新增人工选择→保存→弹窗关闭→终端打开用例，会话 ID 为 null，不调用启动或派发。
- `npm run build` 及 `cargo build --manifest-path src-tauri/Cargo.toml --offline --locked --bin kanban --features tauri/custom-protocol` 通过；保留既有大 chunk 与未使用代码提示。
- 本轮未重跑完整 Rust 套件；不将以上定向结果表述为完整 Rust 全绿。

## 真实远端与桌面验收

第一层：`live_readonly_binding_recovery` 明确选择 `wP:p2`，设置 `KANBAN_READONLY_EXPECT_MISSING_SESSION=1`，先确认当前会话识别返回 None。隔离内存数据库故意保存不一致的旧 UUID，人工绑定、`view_task`、`validate_view` 均成功，旧恢复定位清除，派发记录为零。

第二层：使用本次编译的独立 Tauri/WebView2 程序、隔离 SQLite 及 WebView 数据目录，真实点击“修复绑定”→选择 `wP:p2`→“保存绑定”：

1. 弹窗关闭，收到原生终端就绪结果，界面显示“Herdr 已打开”，检测到该测试应用下的 WezTerm 原生查看进程。
2. 刷新 WebView 页面，原绑定重新打开。
3. 关闭并重启隔离 Kanban，原绑定再次打开。
4. 核对 SQLite：目标仍是 `wP:p2`，旧自动恢复定位为空，派发记录为零。
5. 验收后再次查询真实 Agent，仍是原 terminal/进程、idle、相同状态序号，仍未上报会话 ID。

脚本和证据：`.scratch/manual-binding-desktop-20261111/accept.py`、`result.json`、`connected-webview.png`、`app.log`。WebView 截图只记录页面反馈，不作为独立原生窗口内容的视觉证据。

桌面测试按正式路由生成并使用内容寻址 helper，上传至远端独立文件，机器配置仅写入隔离 SQLite。没有替换 Herdr server，没有向对话发送按键或提示词，没有修改用户任务数据库；验收退出测试应用，不关闭用户 Agent。

## 交付范围

本轮修改代码与文档，未提交 Git、未生成新安装包或安装。上轮安装包不包含本次人工绑定修正。
