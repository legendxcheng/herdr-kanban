# Workspace 导航同步验收

日期：2026-10-24。规格：[Issue #18](https://github.com/legendxcheng/ContextSwitcher/issues/18)，[本地规格](../superpowers/specs/2026-10-24-kanban-workspace-order-visibility.md)。

## 实现与自动化证据

- 数据库迁移 8 保存 workspace_order；完整项目记录与正常导航分离，缓存和在线快照共同隐藏已确认 missing 的关联项目。
- 同步事务按实际在线集合更新位置与存在状态，失败回滚。未决创建不使在线项目被误标关闭；未关联项目仍保留。
- 焦点和定时刷新不重复展示旧缓存。启动及显式刷新/CRUD 可以加载持久化导航，保留离线本地删除的界面反馈。
- 修复前运行 `cargo test --offline --locked --lib projects::tests::navigation -- --nocapture`：两个针对性测试失败，分别观察到 `[a,b,c]` 与期望 `[b,c,a]` 不符，以及关闭后的 Project 仍在返回列表。
- 修复后 `cargo test --offline --locked --lib`：68 项通过、5 项已有 live 测试忽略。新增测试覆盖重排/插入与身份稳定、关闭与同身份恢复、同名新身份隔离、提示词和会话保留、旧 schema 迁移、真实 SQLite 关闭重开、离线与成功空集合、未关联数据、事务回滚及未决创建。
- `npm test -- --run`：73 项通过。App 测试覆盖重排保持当前终端、关闭非当前项不抢选、关闭当前项释放连接并回退、全部关闭空态、慢请求/错误保留当前导航、显式本地刷新及合并请求丢弃旧快照。
- `npm run build`：TypeScript 和 Vite 生产构建通过，仍有既有大 chunk 提示。
- `cargo fmt --all --check`：通过。
- 使用 `cargo rustc --offline --locked --bin kanban --features tauri/custom-protocol -- -o E:/ContextSwitcher/.scratch/workspace-navigation/kanban-navigation.exe` 生成独立 debug 桌面程序；编译成功。输出名覆盖产生两条 rustc 参数提示，不影响产物。

## 原生桌面与真实 Herdr

使用独立 SQLite、WebView2 数据目录及新编译程序；没有更换用户默认数据库或重启用户原应用。脚本及截图位于 `.scratch/workspace-navigation/`。

1. 真实 WebView2 页面通过正式 Tauri IPC 同步并显示当时 17 个在线 workspace。逐项比较侧栏、缓存 IPC 和 Herdr CLI 列表，顺序一致，game-h5 为第 2 项。
2. 通过 Herdr CLI 创建两个带本次验收唯一名称的 workspace，使用 `--no-focus`。自动同步导入后，侧栏和缓存均与实际列表一致。
3. 在测试 workspace A 的真实任务表单创建任务及提示词，不启动 Agent 或派发任务。
4. 关闭测试 B 后焦点刷新：B 消失，A 继续选中，任务卡片保持。
5. 关闭测试 A 后焦点刷新：A 消失，选择回退到第一个可见项目，旧任务不再展示。
6. 只读查询隔离 SQLite：A 的 Project 为 missing，原任务及提示词完整保存。
7. 结束隔离实例后，在限制 PATH 和 LOCALAPPDATA、使 Herdr 可执行程序不可用的隔离进程环境下重启：显示同步失败，缓存仍为原有 17 项且顺序不变，已关闭测试 workspace 不重新出现。
8. 恢复正常环境再启动，按最新 Herdr 列表逐项核对通过，同步错误消失，已关闭测试 workspace 仍隐藏。期间其他并行验收新增/清理了 workspace，在线集合变为 15 项；首次以旧的固定 17 项断言失败，改为读取当时真实列表后通过，没有把外部集合变化当成产品失败或改写个人资源。

本次创建并关闭的 workspace 为 w16、w17；关闭前核对实际 ID、唯一名称、单 tab/单 pane。没有关闭其他验收任务或用户资源。页面无 JavaScript 错误。

证据：`verify_native.py`、`native-results.json`、`before-close.png`、`after-close.png`、`offline-restart.png` 和独立数据库。截图确认真实桌面显示；行为断言使用真实 WebView2 DOM 和 Tauri IPC。

## 验证限制

- 真实 Herdr 未执行手工重排；CLI 无 workspace reorder 命令。已比对真实顺序，重排/插入由服务替身回归覆盖，没有改写 Herdr 会话文件。
- 真正关闭全部用户 workspace 未执行；空列表行为在服务和 App 组件测试中覆盖。
- 本次测试任务未连接真实 Agent；选中项目消失时的终端释放由 App 组件测试验证，没有重复执行已有真实终端生命周期验收。
- 离线桌面使用 CLI 不可用故障，未停止真实 Herdr 服务或模拟网络断连。
- 同 ID 再出现、事务故障及未决操作保留由真实 SQLite 服务测试覆盖；未在真实 Herdr 中人为制造这些故障。
- 未执行 release 安装包构建、安装替换或用户原进程重启。测试数字来自当前共享工作区，包含其他已存在的改动；本次没有修改这些改动的业务范围。
