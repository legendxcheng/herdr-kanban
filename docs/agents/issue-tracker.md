# Issue tracker: GitHub

本仓库的需求与规格存放于 legendxcheng/herdr-kanban 的 GitHub Issues。
使用 gh CLI 操作，在仓库目录运行；在其他目录运行时显式指定
--repo legendxcheng/herdr-kanban。

## Conventions

- 创建：gh issue create --title "..." --body-file <path>
- 读取：gh issue view <number> --comments；
  同时用 gh issue view <number> --json number,title,body,labels,comments
  获取正文、标签与评论。
- 列出：gh issue list --state open --json number,title,body,labels,comments
  按需要添加 --label 或调整 --state。
- 评论：gh issue comment <number> --body-file <path>
- 添加或移除标签：gh issue edit <number> --add-label "..." / --remove-label "..."
- 关闭：gh issue close <number> --comment "..."

多行正文先写入 UTF-8 临时文件，再用 --body-file 传入，保留真实换行。
标签映射见 docs/agents/triage-labels.md。

## Pull requests as a triage surface

**PRs as a request surface: no.**

## Skill instructions

- “publish to the issue tracker”：创建 GitHub issue。
- “fetch the relevant ticket”：读取对应 issue 的正文、标签与评论。

## Wayfinding operations

供 /wayfinder 使用：

- Map：一个带 wayfinder:map 标签的 issue，正文维护
  Notes / Decisions-so-far / Fog。
- Child ticket：优先建立 GitHub sub-issue 关系；
  不可用时，在 map 正文加入任务列表，并在子 issue 顶部写 Part of #<map>。
  使用 wayfinder:research、wayfinder:prototype、wayfinder:grilling
  或 wayfinder:task 标签。
- Blocking：优先使用 GitHub 原生 issue dependencies；
  不可用时，在子 issue 顶部写 Blocked by: #<n>, #<n>。
  所有阻塞项关闭后才可开始。
- Frontier：按 map 中的顺序，选择未关闭、无未完成阻塞项、
  且尚未分配负责人的第一个子 issue。
- Claim：首次写操作使用 gh issue edit <number> --add-assignee @me 认领。
- Resolve：评论记录结果，关闭子 issue，并将摘要和链接补充到
  map 的 Decisions-so-far。

历史规格与验收记录中的 ContextSwitcher Issue 链接指向迁移前的来源；保留原链接。新增需求与规格使用本仓库。
