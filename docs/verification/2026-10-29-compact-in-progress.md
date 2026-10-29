# InProgress 两行卡片验证

日期：2026-10-29。

## 实现范围

- `TaskCard.tsx`：仅进行中列默认两行；标题单行省略、提示词图标入口、完成短文案；来源在第二行显示，模型及异常操作按需展开。
- `App.css`：78 CSS px 折叠高度，局部容器查询适配窄卡片。宽度不足时来源从信息按钮查看，操作保持单行。
- `Icon.tsx` 与中英文文案：文档、信息图标及短操作／状态名称。
- 共享工作区既有重置测试增加先展开“需核实”的操作步骤（该未提交功能不纳入此次提交）；新增两项组件测试验证未决状态下提示词可用、完成禁用、详情展开／恢复不选卡及模型详情收起。
- 无后端、IPC、数据迁移变化。保留工作区其他功能改动；提交快照排除身份颜色与退回 Todo 功能，仅纳入提示词入口必需的只读表单及看板连接。

## 已验证

1. `npm run build` 通过（TypeScript 与 Vite）；仍有已有的大于 500 kB 包体提示。
2. 相关 8 个测试文件共 80 项通过：compact_cards、reset、deletion、selection、dispatch、summary、summary_interactions、model_settings。采用 `node node_modules/vitest/vitest.mjs run <上述文件> --maxWorkers=2`。
3. 独立浏览器组件页面使用实际 TaskCard、App.css 与 taskSummary.css，没有调用后端。最终在中英文、158／164／180／220／280／350 CSS px 卡宽下，常态、模型及来源、准备中、待核实、失败共 60 种组合的折叠高度均约 78px，无卡片横向溢出及优先级裁切。此前 164–350px 的 50 种组合另检查了按钮没有越界。
4. 视觉检查发现最窄英文优先级被裁切，已通过局部缩小横向占位修正；来源区域允许收缩，优先级保留完整宽度。最终测量存于 `.scratch/compact-cards/measurements.json`。
5. 浏览器键盘 Enter 展开／收起模型详情及待核实详情通过；核实、查看提示词回调保持独立，没有触发选卡。原始组件预览及截图保存在 `.scratch/compact-cards/`。
6. 对本次产品文件执行兼容 CRLF 的 `git diff --check` 通过。

## 验证限制

- 较广的 176 项看板测试一次出现 worker 异常退出；限制为 2 个 worker 后为 175 通过、1 失败。失败为 language.test.tsx 技能搜索替身未处理 `search_skills`，额外出现第二个 alert，未修改该测试或计为通过。
- 浏览器使用本地组件 fixture，按钮回调为测试回调，不等同于真实 Tauri/WebView2、Herdr 或 SQLite 验收。
- 未重新打包、安装或替换运行中的应用；未验证原生桌面鼠标拖拽及不同系统 DPI。

## 提交快照验证（2026-10-29）

为避免纳入其他进行中的功能，本次提交以 `fa70557` 为基础生成隔离快照，排除身份颜色、退回 Todo 和 SSH 接入改动；仅包含两行卡片以及提示词按钮必需的只读表单与看板连接。共享工作区中的其他实现和重置测试保持原样。

- 隔离快照 `npm run build` 通过。
- 全量前端检查为 246 通过、2 失败；两项分别为 navigation / workspace-branch 的离线刷新测试仍查找旧“同步工作空间”按钮。将快照产品文件恢复为 `fa70557` 后，原样复现相同两项失败（10 通过、2 失败），确认为基线问题。
- 随后补充 pending / unknown / failed 三种状态的真实看板只读提示词测试：不能编辑、没有保存按钮、不查询模型目录、不更新或派发任务。最终 7 个相关测试文件共 73 项通过（compact_cards、deletion、selection、dispatch、summary、summary_interactions、model_settings）。
- 隔离环境通过 `--configLoader=runner --maxWorkers=2` 运行 Vitest，避免共享依赖目录的临时配置写入权限错误。
- 前述 80 项测试与浏览器截图来自共享工作区；此处单独记录实际提交快照的验证范围，不将未提交的重置或身份颜色功能作为提交内容。
