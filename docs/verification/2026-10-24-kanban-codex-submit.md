# 拖入任务后 Codex 首次回车未提交

日期：2026-10-24。用户在当前会话中确认方案并授权开发；本次未创建或发布 GitHub Issue。范围为 Windows Codex 提示词提交，不实现复现提示词中提及的批量归档/删除需求。

## 复现与判断依据

用户提供的输入是：`右侧的【已完成】区域 需要补上一键归档或一键删除  $kanban-dev `。后端 `resolve_prompt` 会去掉末尾空格。

环境：Windows、Herdr `0.8.0-preview.2026-08-04-d78e3d3b5126` / protocol 19、Codex CLI 0.164.0。测试使用独立工作区、测试目录与同名技能夹具；夹具只回复固定标记且不调用工具，不向用户既有任务发送测试输入。

- 原真实 Workflow → Herdr → Codex 路径返回 `Ok(DispatchRecord)`，等待 12 秒后仍显示完整输入框；目标仍为 idle，状态序号未变。补按一次 Enter 后进入 working，并实际回复固定标记。
- 已安装版本的 `agent send-keys right enter` 会将两个键合并为一批字节写入，成功响应不表示 Codex 已提交输入。
- 对已启动稳定的同一测试窗口，原按键方式仍可复现。拆开并等待 300ms 有成功样本，重复后仍失败；等待 1000ms 也出现未提交。增加 Escape 关闭技能候选、或仅在对照输入末尾保留空格时可以提交，说明单纯延长等待不够。
- 最终保留原文，使用独立 Right、Escape、Enter，并核验新的状态序号。没有通过额外追加空格或重复 Enter 修改输入语义。

## 实现

- `herdr_codex_submit.rs` 提供正文、粘贴边界、候选关闭、单次提交及核验；`herdr_codex.rs` 保留准备/恢复/会话职责并调用此模块。
- 发送前要求同一身份、idle/done、interactive_ready、非 launch_pending。正文后每次写入前继续检查任务存活与目标身份。Right 后等待 300ms，Escape 后等待 100ms，再发送一次 Enter。
- 以 Enter 前的 `state_change_seq` 为基线，约每 200ms 查询同一目标；核验窗口最多 10 秒，单次 CLI 请求另有原有超时。仅序号增加且处于 working/blocked/idle/done 时确认受理，不等待业务结果。
- 真实长文本回归发现：5 秒核验窗口可能在提示词实际提交后先返回待核实。扩大至 10 秒后，包含超过 11K 字符输入的完整矩阵通过。超过边界仍保守保留 unknown，不重发。
- 提交未确认使用 `dispatch.submit_verify_required` 翻译键，经现有工作流保存 unknown 和独占证据；派发弹窗仍识别其待核实性质。另一个 pane 的合法任务不受影响。
- `/quit` 不以新的任务轮次作为回执，由原准备流程继续等待原 Codex 退出至 shell。没有新 IPC、数据库迁移或用户全局配置修改。

## 验证结果

以下测试计数包含共享工作区其他已存在的功能改动，不能全部归于本修复。

| 层级 | 验证与结果 |
| --- | --- |
| Rust 自动化 | `cargo test --offline --locked --lib`：112 通过、8 项 opt-in live 默认忽略。新增提交测试覆盖独立按键与等待、陈旧/unknown 状态、很快完成、正文后各阶段身份/请求失败、忙碌目标拒绝和 `/quit`；工作流另覆盖未确认时不重发且其他 pane 可继续派发 |
| 前端 | `npm test`：110 项通过；中英文新错误键有效 |
| 构建及格式 | `npm run build` 与独立目标目录的 Rust 桌面构建通过；保留既有大 chunk 提示。涉及 Rust 文件的 rustfmt 检查通过；差异检查按仓库已有 CRLF 约定使用 `core.whitespace=cr-at-eol` |
| 真实提交矩阵 | 显式运行 `live_short_chinese_skill_prompt_is_submitted`：通过，53.74 秒。原提示词连续 3 次、普通中文、多行技能、超过 11K 字符长文本，共 6 次；每次等待真实对话新增且仅新增一条固定回复，未使用 CLI/IPC 替身 |
| 真实退出 | 显式运行 `live_codex_quit_preserves_the_test_shell`：通过，2.22 秒。统一发送路径的 `/quit` 退出测试 Codex，原 pane/terminal 仍存在 |
| 原生桌面拖放 | 独立 Tauri/WebView2、SQLite、前端服务和用户数据目录；真实创建文本提示词并拖入开发中，再创建含实际技能 mention 节点的提示词并拖入。两任务独占不同 pane、启动 succeeded、最终 snapshot 与用户原文一致；真实对话各只有 1 个轮次、1 个回复、0 个工具调用，无手动补 Enter |

本机测试脚本、原始失败/通过日志、对照结果、原生结果与截图位于 `.scratch/kanban-submit/`。原生检查使用真实宿主、数据库、Herdr 与 Codex，不属于浏览器模拟 IPC 验收。

## 边界

- 本次未替换或重启用户原来运行的 Kanban，也未重新打包/安装 release。
- 固定回复技能夹具验证输入和技能引用的提交边界，不验证真实 kanban-dev 的归档/删除方案输出。
- 状态序号核验证明目标发生新的生命周期变化；不等于任务开发完成或用户验收通过。不分析 Agent 回复中的自然语言来推断业务完成。
- 未执行其他 Agent、非 Windows、Herdr 服务重启或网络断开验收；提交后的超时/身份变化等故障路径由单元及工作流测试覆盖。
- 验收结束后清理本次创建的测试实例和两个测试工作区；用户既有窗口、工作区和任务未发送、关闭或重放。
