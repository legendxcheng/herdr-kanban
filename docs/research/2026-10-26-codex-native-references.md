# Codex CLI 原生文件与技能引用研究

日期：2026-10-26。状态：研究阶段未修改产品代码；后续用户确认后，已实现普通文件的原生文本格式。技能精确路径与真实 TUI 验收仍保留下述边界。

## 版本与证据边界

- 本机 `codex --version`：`codex-cli 0.164.0`；入口位于 `C:/Users/Administrator.DESKTOP-09JQGNK/AppData/Local/Programs/OpenAI/Codex/bin/codex.exe`。
- 官方源码使用 [`rust-v0.164.0`](https://github.com/openai/codex/tree/rust-v0.164.0)，固定 commit 为 `6aaac694803004a63c6f393d153c96d22521a6b9`。`fe3b29455488c2e189e43ad5526a107900c5e1b7` 是 annotated tag 对象，不是代码 commit。
- 下文“源码证据”来自该版本实现和仓库内原生测试的断言；没有在本机编译运行上游测试。
- 本机运行了 `debug prompt-input`，检查输出的模型可见输入；该命令不是完整 TUI 提交流程，也不是实际模型请求。
- 尝试隔离 TUI 验证时，Folder access / Trust repo 确认被自动审批拒绝，理由是该操作会持久修改目录信任；已按 Escape 退出，没有绕过审批。因此本轮未完成真实 TUI 候选选择后的端到端提交验收，也没有操作用户现有 Herdr pane。

## 普通文件：选中后是路径文字

原生 `@` 用于打开和筛选文件候选。选中普通文件后，CLI 将整个 `@查询词` 替换成选中的路径，`@` 不保留；不会添加 `[文件: ...]`、标题或代码块，也不会在这个步骤读取文件正文。

源码链路：

1. `file_search_popup::selected_match()` 返回候选的 `FileMatch.path`，不读取正文。[file_search_popup.rs:99–103](https://github.com/openai/codex/blob/6aaac694803004a63c6f393d153c96d22521a6b9/codex-rs/tui/src/bottom_pane/file_search_popup.rs#L99-L103)
2. Tab / Enter 接受候选后，将该路径传入 `insert_selected_file_path()`；旧文件候选与新统一候选均走此入口。[chat_composer.rs:2173–2197](https://github.com/openai/codex/blob/6aaac694803004a63c6f393d153c96d22521a6b9/codex-rs/tui/src/bottom_pane/chat_composer.rs#L2173-L2197)、[2382–2395](https://github.com/openai/codex/blob/6aaac694803004a63c6f393d153c96d22521a6b9/codex-rs/tui/src/bottom_pane/chat_composer.rs#L2382-L2395)
3. 普通文件进入 `insert_selected_path()`，替换当前 token，移动光标并处理分隔空白。路径含空白且不含双引号时，外加一对双引号；已有双引号时直接保留路径文字，不追加一套转义规则。[chat_composer.rs:2779–2798](https://github.com/openai/codex/blob/6aaac694803004a63c6f393d153c96d22521a6b9/codex-rs/tui/src/bottom_pane/chat_composer.rs#L2779-L2798)
4. 原生测试断言：选中 `src/main.rs` 后编辑器保留路径及尾部空格；提交时输出路径文字，且文件没有新增 `TextElement`。这是源码内测试断言，不是本轮执行测试的结果。[chat_composer.rs:10332–10364](https://github.com/openai/codex/blob/6aaac694803004a63c6f393d153c96d22521a6b9/codex-rs/tui/src/bottom_pane/chat_composer.rs#L10332-L10364)
5. TUI 将编辑后的完整文字放入 `UserInput::Text`；普通文件没有独立 `UserInput::File` 类型。[input_submission.rs:275–279](https://github.com/openai/codex/blob/6aaac694803004a63c6f393d153c96d22521a6b9/codex-rs/tui/src/chatwidget/input_submission.rs#L275-L279)、[user_input.rs:15–60](https://github.com/openai/codex/blob/6aaac694803004a63c6f393d153c96d22521a6b9/codex-rs/protocol/src/user_input.rs#L15-L60)

示意：用户输入“检查 ”并从候选选中 README 后，提交文本应接近：

```text
检查 README.md
```

子目录与空格路径：

```text
检查 src/App.tsx
检查 "docs/说明 文件.md"
```

这里“Workspace 相对路径”仍沿用本次 Kanban 已确认的产品约定。原生插入函数使用的是候选提供的路径，不应据此声称 CLI 在所有多根目录场景都会将路径重新转为当前 Workspace 相对路径。

仅手工输入 `@README.md` 而未选择候选，不等同于候选选择。`@` 替换发生在 TUI 接受候选的阶段，不是所有 Prompt 文本进入模型前都有一次通用 `@文件` 展开。

## Skill：名称输入与精确选择有所不同

Kanban 不应把技能正文手工拼进用户 Prompt。但“Kanban 只发送 `$技能名`”不意味着模型最终只看到这几个字符。CLI 的正常技能调用链会识别技能，并由 CLI 自己加载技能内容。

TUI 提交的两层数据：

- 普通用户文字保留在 `UserInput::Text` 中。
- 从技能候选选中时，CLI 保存 mention 的精确路径 binding，并额外提交 `UserInput::Skill { name, path }`。仅粘贴或手工输入 `$name`，CLI 也会尝试从当前已发现技能中匹配名称，再产生技能项。[input_submission.rs:275–339](https://github.com/openai/codex/blob/6aaac694803004a63c6f393d153c96d22521a6b9/codex-rs/tui/src/chatwidget/input_submission.rs#L275-L339)

精确选择与纯文本名称不完全等价。TUI 的查找逻辑优先匹配带路径的引用；无路径时，遍历当前技能列表，同名只保留第一个匹配。Kanban 目前保存了 `ref`，发送却只保留 `label`，因此无法保证“Kanban 选中的同名技能”一定等于“Codex 当前目录中按名称找到的技能”。[skills.rs:205–256](https://github.com/openai/codex/blob/6aaac694803004a63c6f393d153c96d22521a6b9/codex-rs/tui/src/chatwidget/skills.rs#L205-L256)

正常 turn 会收集显式技能，调用 `load_skill_prompts()`，读取选中技能并生成上下文片段；这些片段是 user-role 的 `<skill>` 内容，包含名称、路径和技能正文。该加载由 Codex 执行，和 Kanban 自己在用户 Prompt 中展开文件是不同的职责。[turn.rs:1147–1189](https://github.com/openai/codex/blob/6aaac694803004a63c6f393d153c96d22521a6b9/codex-rs/core/src/session/turn.rs#L1147-L1189)、[host_prompt.rs:60–108](https://github.com/openai/codex/blob/6aaac694803004a63c6f393d153c96d22521a6b9/codex-rs/ext/skills/src/host_prompt.rs#L60-L108)、[fragments.rs:87–128](https://github.com/openai/codex/blob/6aaac694803004a63c6f393d153c96d22521a6b9/codex-rs/ext/skills/src/fragments.rs#L87-L128)

源码支持带路径的技能文本引用，例如 `[$name](路径/SKILL.md)`，并优先按路径匹配；这可以作为未来保留 Kanban 精确选择的候选方案。但是否采用它，需要在目标 CLI 的技能发现目录、远端路径、同名技能与真实提交路径上验收，不能仅凭语法支持就认定完全等价。[mentions.rs:77–110](https://github.com/openai/codex/blob/6aaac694803004a63c6f393d153c96d22521a6b9/codex-rs/skills/src/mentions.rs#L77-L110)、[selection.rs:21–79](https://github.com/openai/codex/blob/6aaac694803004a63c6f393d153c96d22521a6b9/codex-rs/ext/skills/src/selection.rs#L21-L79)

## 本机 debug 验证与限制

在隔离 `CODEX_HOME`、不使用真实认证的环境，运行 0.164.0 的 `debug prompt-input`。provider 指向不可用的本地回环地址，用于避免真实模型请求。研究过程没有读取或输出用户真实任务正文。

| 输入 | 检查结果 | 本地证据 |
| --- | --- | --- |
| `REFERENCE_PROBE inspect README.md` | 模型可见输入保留该文本；未包含测试文件正文 sentinel | `.scratch/codex-native-reference/plain-prompt.json` |
| `REFERENCE_PROBE inspect @README.md` | 模型可见输入保留 `@README.md`；未包含测试文件正文 sentinel | `.scratch/codex-native-reference/at-prompt.json` |
| 含双引号的空白文件路径 | 模型可见输入保留路径及双引号；未包含测试文件正文 sentinel | `.scratch/codex-native-reference/quoted-prompt.json` |

三次调试命令均退出 0，未启动模型轮次。`.scratch/codex-native-reference/summary.json` 汇总实际 `model_visible_user_text`，三个案例的 `file_body_inlined` 均为 `false`。

这证明普通文本进入该调试构造器时不会自动展开文件，也不会自动去除 `@`；“选择候选后去掉 @”的证据来自上述 TUI 源码。

`debug prompt-input` 只将命令行文字组装为 `UserInput::Text`，随后使用独立请求构造流程。源码明确该路径不进入 `run_turn`，直接记录输入并构造 Prompt；因此不能用调试输出缺少技能正文来否定真实 turn 的技能注入行为。[cli/main.rs:2135–2145](https://github.com/openai/codex/blob/6aaac694803004a63c6f393d153c96d22521a6b9/codex-rs/cli/src/main.rs#L2135-L2145)、[prompt_debug.rs:84–119](https://github.com/openai/codex/blob/6aaac694803004a63c6f393d153c96d22521a6b9/codex-rs/core/src/prompt_debug.rs#L84-L119)

`.scratch` 文件是本机调试证据，不承诺纳入版本库。上述源码链接使用固定 commit，可独立复核。

## 对 Kanban 的建议及后续实施状态

1. 已实现：普通文件不读取正文，去掉 `[文件: ...]` 包装；在原引用位置输出 Workspace 相对路径。
2. 已实现：对含 Unicode 空白的路径沿用 CLI 的双引号规则，保持普通文字、引用顺序及自然分隔，不为引用插入额外换行。
3. 已实现：编辑器继续显示 `@文件` chip；发送文本、派发预览与后台启动使用同一输出约定，前后端共用 19 个路径/引号规则案例。
4. `$Skill` 继续交给 CLI 加载，不在 Kanban 注入正文。精确技能路径是否通过带路径文本引用保留，作为单独确认的增量，不把名称调用误称为已完整还原候选选择。
5. 验收覆盖根目录/子目录/空格/中文文件路径、相邻引用、文件与技能混排、本机与远端、无文件读取 IPC，以及手动派发/自动启动的一致性。真实 TUI 选择与最终请求对照仍待允许的隔离环境补充。

图片需单独处理。该版本 CLI 选中可解析图片时会转为图片附件；不能把“普通文件是路径文字”推广成所有图片也只发路径。本轮不建议改变 Kanban 既有图片准备、持久化和远端上传链路。[chat_composer.rs:2497–2517](https://github.com/openai/codex/blob/6aaac694803004a63c6f393d153c96d22521a6b9/codex-rs/tui/src/bottom_pane/chat_composer.rs#L2497-L2517)

后续实现验证：相关前端 40 项及 Rust 213 项测试通过，20 项 Rust 环境依赖测试跳过；限定本次模块及其依赖的 TypeScript 检查、Vite 构建通过。共享工作区全量前端当次 215 通过、1 项汇总交互失败；常规构建受另一项正在进行的汇总看板测试改动阻断（未使用导入及 onSelect 接口不一致），没有修改这些无关测试来获取通过。尚未重新打包安装或完成真实 TUI/Herdr 派发验收。

额外实测：直接调用本次实际 `src/dispatchUtils.ts` 的 `resolveMentions`，生成根目录、子目录、中文空格路径三个 Prompt，再交给本机 Codex CLI 0.164.0 的 `debug prompt-input`；三项模型可见用户文字与 Kanban 生成文本逐字一致，且没有文件包装或正文 sentinel。证据：`.scratch/codex-native-reference/kanban-native-output-validation.json`。该验证不经过模型轮次，也不替代真实 TUI/Herdr 验收。

提交前隔离验证（2026-10-26）：提交前另将仅含本次 14 个文件/改动块的暂存树导出为隔离副本，完整前端 215 项、Rust 201 项测试通过（19 项环境依赖测试跳过），`npm run build` 通过；这些结果验证本提交不依赖共享工作区其他未提交改动。 前端使用 `--configLoader runner` 避免共享依赖目录的临时配置写入冲突。隔离副本位于 `.scratch/native-refs-commit/`；只用于验证，不包含用户数据。
