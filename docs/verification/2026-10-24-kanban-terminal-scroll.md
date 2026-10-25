# Kanban 终端滚轮验收

日期：2026-10-24。规格：[GitHub #21](https://github.com/legendxcheng/ContextSwitcher/issues/21)、[本地规格](../superpowers/specs/2026-10-24-kanban-terminal-mouse.md)。

用户取消鼠标点击要求后，本次只交付滚轮接入与步长增大：常见一格滚轮滚动 6 行；不新增点击、拖动或悬停转发，不升级用户的默认 Herdr。

## 原因与实现

Herdr 的应用鼠标报告及 alternate-scroll 路径忽略一条 `terminal.scroll` 中的 `lines` 放大值。修复前的真实 Codex 验收，较大的滚轮输入只将最早可见编号从 95 移到 94。后端现在在一次任务身份核验后，写入有限批量的单行滚动命令，同时兼容宿主历史与应用内部滚动。

前端一格按 3 行/120 CSS px 换算为 6 行；保留小数累积、方向反转清理、单批 24 行及最多 16 批的队列限制。尺寸变化暂停输入，等待最终尺寸同步及完整帧渲染；切换/断开时清空待发送滚动。键盘使用原有输入链路。

## 真实桌面证据

环境：真实 Windows Tauri/WebView2，隔离数据库、独立 Herdr session `kanban-mouse-20261024`，Herdr 0.9.5 protocol 22，真实 Codex CLI 0.164.0 全屏 transcript。独立 CODEX_HOME 中使用编号输出夹具；不是模拟 Tauri IPC。测试窗口 1280×800、devicePixelRatio 1。

| 操作 | 最早可见编号：底部 → 上滚一格 → 下滚一格 | 结果 |
| --- | --- | --- |
| 默认字号 | 95 → 89 → 95 | 上滚 6 行，可返回 |
| Ctrl+= 放大字号 | 97 → 91 → 97 | 上滚 6 行，可返回 |
| Ctrl+- 缩小字号 | 95 → 89 → 95 | 上滚 6 行，可返回 |
| 实际拖动分隔线调整面板宽度 | 85 → 79 → 85 | 上滚 6 行，可返回 |
| 终止专用测试查看控制器后，点击重新连接 | 85 → 79 → 85 | 不关闭 Codex pane，恢复后滚轮正常 |

重复下滚返回底部；切项目再返回后，原任务输出保留，键盘输入 `WHEEL_KEYBOARD_OK` 可见并已清空、未提交给模型。断线测试只终止专用 session 的查看控制器，不终止 Agent 或共享 server。

同项目双任务补验：两张开发中卡片分别绑定 `w2:p1`、`w2:p2` 的真实 Codex。任务 A 连续上滚 10 次后立即点击任务 B，B 最早可见编号保持 85，等待 1.5 秒后仍为 85；在 B 上滚一格变为 79。点击回 A 恢复 A 的输出，不混入 B 的输出。验证使用实际卡片点击和滚轮，未模拟 IPC。

本机已安装 Herdr `0.8.0-preview.2026-08-04-d78e3d3b5126` 另开独立 session `kanban-wheel-legacy-20261024`，真实宿主历史偏移经六条单行事件从 0→6→0；首帧包含 width/height，兼容网格就绪判断。这项是实际 Herdr 协议/宿主回滚验证，未冒充旧版完整 Windows/Codex 组合验证。

本机证据（`.scratch` 不提交）：`terminal-mouse/native-scroll.cjs`、`scroll-results.json`、`scroll-final.png`、`native-recovery.cjs`、`recovery-results.json`、`native-switch.cjs`、`switch-results.json`、`legacy-scroll.py`、`legacy-results.json`。早期 `native-acceptance.cjs`/`native-results.json` 包含后来取消的点击试验，不代表最终点击支持。

## 自动化验证

- 前端全量：`npm test`，6 个文件、88 项通过。
- 前端类型与构建：`npm run build`（TypeScript + Vite）通过；保留既有大 chunk 警告。
- Rust：`cargo test --offline --locked`，80 项通过、6 项既有专用 live 测试 ignored；`cargo fmt -- --check` 通过。
- 最后补充纯横向触控板事件不清除垂直小数积累后，`npx vitest run src/TerminalPanel.test.tsx` 4 项及 `npx tsc --noEmit` 再次通过。
- 终端组件测试覆盖六行步长、小增量/反转、点击不转发、切任务丢弃排队动作、键盘回归、字符区边界和渲染尺寸就绪。xterm 渲染器及 Tauri IPC 使用替身，这些测试不替代上面的真实桌面结果。
- 提交隔离检查：工作区含其他未提交功能，因此只暂存本次增量，并将索引导出到独立目录再验证。独立交付树前端 71 项通过、类型检查及生产构建通过；Rust 64 项通过、5 项既有 live 测试 ignored。数量与共享工作区不同是因为未包含其他功能的测试，不是删除测试。

## Standards

固定基点 `ea25b15fd8346a7d2e098442c7b18ea4283bd7dc`，只审查本次暂存差异。0 项硬性规范违反，1 项低优先级维护建议：`terminal.rs` 的键盘/滚轮身份核验序列存在相同逻辑，可在后续维护时提取；当前逻辑一致，不构成功能错误。翻译键及结构文档符合现有规范。

## Spec

初次发现同项目真实任务切换验收缺口，补充上述两任务原生验收后，经审查复核已消除。六行步长、单行协议批量转发、有界队列、连接失效清理、网格校验和点击取消均符合规格，未发现范围扩张或可确认的运行逻辑错误。

合计：Standards 1 项低优先级维护建议、0 阻断；Spec 0 项剩余发现。

## 边界

真实系统 DPI 切换未执行：电脑控制工具拒绝访问 Windows Settings，未通过其他方式绕开。未单独做真实剪贴板回归或异常网格/修饰键的原生故障注入；切任务丢弃队列由组件及真实双任务测试覆盖，项目切换及重连由真实桌面覆盖。代码未注册点击或选择事件，保留现有选择/复制路径。

未替换用户正在运行的看板二进制或默认 Herdr 安装；专用本机测试运行的是新编译的隔离看板。源码交付与安装部署分开记录。
