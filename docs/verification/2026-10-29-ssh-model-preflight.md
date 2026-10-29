# SSH 任务模型目录预检修正

日期：2026-10-29。

## 行为与范围

用户报告另一台电脑通过 SSH 向本机派发任务：SSH 环境无法识别 Codex，但 Herdr 可以启动 Codex。用户确认 SSH 启动和恢复应跳过目录预检，直接通过 Herdr 应用所选模型与思考强度。

- `lifecycle::run_start` 在创建 pane 前保留配置格式检查；仅本地显式配置执行模型目录查询与能力校验。远端新建和精确恢复均直接使用冻结配置，不调用 `codex debug models`。
- 模型与思考强度继续作为独立 CLI 参数传递；Herdr 实际启动失败不降级，未知结果沿用待核实语义。恢复仍保持原会话且不重复发送提示词。
- 表单保留目录查询、缓存、刷新及已有目录中的配置校验。SSH 目录查询失败时保留已有选择，中英提示明确说明可继续启动；缓存提示不再声称 SSH 启动前会重新查询目录。
- 无数据库迁移或 IPC 变更。未修改 Codex 安装、PATH、SSH 配置或正在运行的任务。

## 验证

新增的三个 Rust 回归在修改产品逻辑前均失败：SSH 启动被 `model.catalog_unavailable` 阻断，无法进入预期的成功或真实启动失败路径；非法参数仍触发目录调用。修改后通过。

- `cargo test --lib lifecycle::tests -- --test-threads=4`：54 项通过，1 项依赖真实环境的回执测试忽略。覆盖远端后台启动与精确恢复、模型/强度冻结、目录零调用、单次派发、实际启动失败保持 unknown、非法参数在创建 pane 前拒绝，以及原有本地目录校验与恢复流程。
- `cargo test --lib selected_model_and_effort_reach_the_real_cli_adapter_as_arguments -- --test-threads=1`：1 项通过。真实 CLI 适配器配合可控子进程验证模型和强度 argv；不是实际模型运行验收。
- `npx vitest run src/board-tests/model_settings.test.tsx src/i18n.test.ts`：11 项通过。包括 SSH 无缓存／有缓存但查询失败时，真实表单保留配置并提交保存；IPC 使用模拟实现。
- `npm run build`：TypeScript 和 Vite 生产构建通过，保留已有大 chunk 提示。Rust 测试保留历史终端模块未使用成员警告。

## 交付边界

以上验证使用当前共享工作区，包含既有未提交改动；只说明本次相关验证结果，不代表全仓库测试或安装版验收通过。

本次未从用户的实际发起方电脑执行真实 SSH 新建／恢复验收，未生成安装包或替换运行中的应用。需要更新发起方电脑的 Kanban 才能使用此修正；目标机无需为本改动调整 PATH。真实跨机验收仍应确认所选模型/思考强度生效，恢复保留会话且业务提示词只派发一次。
