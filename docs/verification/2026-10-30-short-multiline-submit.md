# Windows Codex 短多行提交与本机回执接线

日期：2026-10-30。

## 现场与复现

任务“排查越打越卡及性能优化”已被 Codex 执行，但 Kanban 保留 `dispatch.submit_verify_required`。只读核对：派发原文 165 字符、3 个 LF；会话 `01a18aae-c9f8-7b31-993c-d878981437a2` 在 02:41:16.774 UTC 收到 162 字符，正好丢失全部 LF。未修改这张任务卡、绑定或进程，未重发业务正文。

真实回归使用 Herdr 0.9.5、Codex 0.166.0、独立 Workspace `kanban-acceptance-submit-20261030`、内存 SQLite，并走 `Workflow::start_task`。提示词只要求固定回复，不执行业务任务。每次发送使用新的 UUID 和新 pane。

1. 旧长文本回归通过；增加路径前缀后的长文本也通过。
2. 新短文本回归 `live_path_first_multiline_start_has_exact_receipt_and_one_submission` 在原实现失败：`w29:p4`，会话 `01a18abd-9914-7710-a6c4-5bf996fd8c5d`，3 个换行均消失；虽然启动返回成功且 Codex 回复了固定标记，完整正文回执仍不匹配。说明只检查回复或状态会漏掉正文损坏。
3. 对照试验将 LF 编为 CRLF 后仍失败：`w29:p5`，会话 `01a18abf-b33f-7cb2-98a3-81a0c96eb520`，3 个换行变成 6 个。
4. 每个换行编为单个 CR 后通过：`w29:p6`，会话 `01a18ac1-7266-7830-8f5d-6e223748f047`，03:01:39.607 UTC 收到的正文逐字一致，测试耗时 10.96 秒。
5. 长文本、中文、空行、Tab 和末尾 `$kanban-dev` 回归通过：`w29:p7`，会话 `01a18ac2-b14a-7a81-8de6-c95c796bd732`，03:03:03.562 UTC 的正文逐字一致，耗时 11.85 秒。
6. 接通本机回执后的最终短文本复测通过：`w29:p8`，会话 `01a18ac5-a53f-7070-ac75-4e9804247f0c`，03:06:17.352 UTC 的正文逐字一致，耗时 18.73 秒。

这些实测证明当前 Windows 输入路径需要单 CR 编码，未把 Codex 内部的具体长度阈值或解析实现视作已确认事实。

## 修正

- `herdr_codex_submit.rs` 保留 bracketed paste，仅在传输副本中把每个 LF 换行编成单 CR；已有 CRLF 先合并，避免重复 CR。原始正文与严格回执比较保持不变。看板编辑器产生的 LF 正文经真实回归验证；没有新增对任意原始 CR/CRLF 文本的宽松回执匹配。
- `herdr.rs` 的本机 `send_checking_receipt` 不再丢弃 `received` 回调。原先本机传入恒为 false 的占位回调，10 秒状态窗口和 30 秒补查窗口内都读不到回执，只能等 Workflow 最后补查；现在真实回执可提前结束等待，并由 Workflow 再核验及事务提交。
- 本次没有改变状态序号确认分支、未知结果保护、任务有效性检查或身份校验。正文与 Enter 各发送一次，不增加重试，不自动处理历史故障。

## 回归

```text
cargo test --manifest-path src-tauri/Cargo.toml --offline --locked --lib short_multiline_paste_survives_windows_console_line_feed_handling -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --offline --locked --lib local_submission_checks_receipt_before_stale_status_expires_without_replaying -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --offline --locked --lib -- --test-threads=4
```

- 短文本单元回归先红后绿，重放上述真实 LF/CRLF/CR 的输入结果。
- 本机回执回归通过真实 `Cli` 和固定 idle 序号的 `.cmd` 替身执行。修复前回调计数为 0，回归失败；修复后回调一次，并断言输入严格为 `text/right/esc/enter` 四次。该测试不依赖模型或 40 秒墙钟超时。
- 完整 Rust 库测试：287 passed、0 failed、30 ignored，34.46 秒。两个真实多行测试已另行显式执行，不把其他 ignored 用例算作通过。保留既有 terminal dead-code 警告。
- 实际启动回归同时断言 SQLite 为 `in_progress / valid / sent / succeeded`，派发记录恰好一条，精确正文在同一会话及时间范围内唯一。

## 边界

本次未执行 SSH、原生 WebView2 拖放或安装包验收；没有改动前端，未重跑前端套件。工作区在测试期间出现其他任务的标题生成相关改动，均未纳入本次修复。未提交 Git、未打包安装、未替换运行中的 Kanban；部署修复版本后，新派发才使用此逻辑。
