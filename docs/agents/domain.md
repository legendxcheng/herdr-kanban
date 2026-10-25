# Domain Docs

## Layout

采用 single-context 布局，覆盖整个 Herdr Kanban 仓库，包括 src/ 与 src-tauri/：

- 根目录 GLOSSARY.md：领域术语。
- docs/adr/：架构决策记录。

## Before exploring

探索代码前，读取已有的 GLOSSARY.md 和与当前工作相关的 ADR。

文件不存在时直接继续；无需提示缺失或预先创建空文件。
由 /domain-modeling 在实际澄清术语或确定决策时按需创建。

## Vocabulary

Issue 标题、方案、假设和测试名称中的领域概念，
使用 GLOSSARY.md 定义的术语。

遇到未收录的概念，先检查现有词汇是否已经覆盖；
确有缺口时，记录为 /domain-modeling 的待处理项。

## ADR conflicts

方案与已有 ADR 冲突时，明确指出对应 ADR、
冲突内容及建议重新讨论的理由。
