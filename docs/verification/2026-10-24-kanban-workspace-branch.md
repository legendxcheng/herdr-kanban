# Workspace Git 分支显示验收

日期：2026-10-24。范围：按用户确认的方案，将左侧 Workspace 第二行的目录替换为当前 Git 分支，保留 Workspace 名称和路径悬浮提示。

## 实现

- `ProjectSidebar` 显示分支图标、分支名或明确的读取/异常状态；长文本省略，完整标签及路径保留在 title 中。原未关联状态继续优先显示。
- 独立 `project_git_statuses(projectIds)` IPC 从数据库读取当前可见项目目录，释放锁后在后台查询，不依赖 Herdr 在线。分支投影不持久化，无数据库迁移。
- Git 读取按目录去重，每批最多四路；单目录两条可能的 Git 命令共用两秒截止时间，超时终止并回收进程，输出限制 16 KiB。只读查询未产生提交的分支、普通分支及游离 HEAD，支持子目录和 worktree，不修改 Git 配置或分支。
- 前端复用启动、目录变化、焦点、前台 15 秒及手动刷新。请求串行合并，迟到/过时请求不能覆盖当前项目和路径；分支刷新保留项目、任务及终端选择。

## 自动化结果

- `npm test`：17 个文件、110 项通过。新增六项组件回归覆盖显示/悬浮提示、分支变化时保留卡片和终端、Herdr 离线焦点和手动刷新、前台/后台定时器、Herdr 查询仍在等待时独立更新 Git、游离/非仓库/错误恢复、路径变化与连续焦点下的迟到响应隔离。旧项目切换用例改为按项目名定位，不再要求已移除的路径文本。
- `cargo test --offline --locked --lib`：111 项通过、7 项已有真实 Herdr/Agent 用例忽略。新增五项使用真实临时 Git 仓库验证未提交分支、中文分支改名、子目录、独立 worktree、游离 HEAD、非仓库、目录/Git 缺失、批量身份及进程超时回收。
- `npm run build`：TypeScript 与生产构建通过，保留既有大 chunk 提示。
- `cargo fmt --all --check` 与 `git -c core.whitespace=cr-at-eol diff --check`：通过。工作区已有 CRLF，后者允许 CR 行尾；不改写其他工作的文件格式。
- 独立 debug 桌面程序通过 `cargo rustc --offline --locked --bin kanban --features tauri/custom-protocol -- -o E:/ContextSwitcher/.scratch/workspace-branch/kanban-branch.exe` 构建。输出命名产生两条 rustc 参数提示，不影响构建成功。

上述数字来自共享工作区，包括执行前已存在及并行落地的其他改动，不代表只统计本次增量。

## 原生桌面与真实 Git

使用独立 SQLite、WebView2 数据目录与新构建程序，缓存中预置七个项目；Herdr 可执行路径故意设为不存在，确保本地分支读取独立于 Herdr 在线。未向真实 Herdr 创建或关闭 Workspace、pane、Agent，未派发任务。Windows 截图观察后，具体断言通过本实例 WebView2 调试连接、真实 DOM 和正式 Tauri IPC 完成，没有替换 Git 或 IPC。

1. 普通仓库及其子目录均显示 `main`；独立 worktree 显示 `feature/worktree`；游离 HEAD 显示真实短提交号；非 Git 目录与不存在目录分别显示“非 Git 仓库”“分支不可用”。
2. 七行 title 与完整标签、真实目录逐项匹配。长分支完整内容宽 388px，在 1920×1009 内容区可用 92px、在 1280×800 内容区可用 82px，均以 `text-overflow: ellipsis` 省略，侧栏布局保持。最终代码重新编译后的原生断言与截图对应 1280×800。
3. 在隔离仓库外部改分支后，点击原同步按钮更新普通仓库与子目录的分支，项目选择保持；Herdr 同步错误仍显示，Git 状态正常更新。
4. 再次改分支，不点击按钮，前台定时器约 15000ms 更新标签。
5. 再改分支，触发页面焦点事件后更新；该项验证应用事件链路，不等于真实鼠标切换应用焦点验收。
6. 正式 IPC 返回七项状态及对应项目/目录身份，顺序与缓存项目对应。中英文的非仓库、不可用及游离文案验证通过。未记录页面 JavaScript 错误。

测试仓库由沙箱用户创建，桌面实例以 Administrator 身份启动。第一次 Git 因仓库所有者不同而拒绝，界面正确显示“分支不可用”；为后续测试，仅在该隔离进程环境中给四个明确的测试仓库传入临时 `safe.directory` 项，没有写入用户或系统 Git 配置，产品代码也没有绕过 Git 检查。

原生按钮输入曾遇到 Computer Use `unknown screenshotId`，刷新观察后停止该输入路径；后续按钮行为由本实例 WebView2 DOM 测试执行，未将该失败的系统输入记为通过。

证据位于 `.scratch/workspace-branch/`：`prepare.py`、`verify_native.py`、`fixtures.json`、`native-results.json`、`native-branches.png`、独立 SQLite 和可执行文件。

## 边界

- 未替换用户当前应用，未重新打包或安装。
- 未重跑本次真实 Herdr/Codex 生命周期；该路径没有业务变更。
- 实际终端连接保留由组件回归验证，原生 fixture 没有运行 Agent。
- 缺失 Git 与查询超时由真实进程级 Rust 测试覆盖，未在原生实例中改变 PATH 或人为挂起 Git。
- 网络目录、系统 DPI 切换和其他平台没有验收。
