# Windows 打包与原路径更新验收

日期：2026-10-24。范围：`build_kanban.bat`、`scripts/build-kanban.ps1`、Tauri Windows 安装配置及分发说明。保留工作区已有图标、业务源码和结构文档改动；本次没有修改业务 IPC 或数据库迁移。

## 结果

- 从根目录运行 `build_kanban.bat -NoPause -NoOpen` 完整成功，退出码 0。生产前端、Rust Release 和 NSIS 安装包均实际生成。
- 分发包：`release/kanban/0.1.0/Herdr-Kanban-0.1.0-windows-x64-setup.exe`，3,113,513 字节，约 2.97 MiB。同目录包含 `README.txt` 和 `SHA256SUMS.txt`。
- SHA-256：`2db7d7b8621c1febaf40f9dfe9bce13669b80eaf1824853633eb904e9431edcc`。
- 成功构建日志：`release/kanban/logs/build-20261024-135543-104960.log`。产物目录与构建目录被现有 release 忽略规则排除，不提交二进制。
- 打包源码副本中的 81 项前端测试通过。生产构建包含 TypeScript 检查；保留既有前端 chunk 大于 500 kB 提示。
- 已确认开发目录依赖版本与 package-lock 一致，版本文件未被脚本修改。

## 真实安装与升级

使用真实安装器的 `/S /NS` 静默安装，不创建快捷方式；并非模拟注册表或文件复制。测试前确认 HKCU/HKLM 的 32/64 位视图中均无该应用安装记录，且默认安装目录不存在，避免覆盖个人安装。

| 场景 | 实际结果 |
| --- | --- |
| 0.1.0 首次安装 | 通过 `/D=` 选择 `.scratch/kanban-packaging-acceptance-verified/custom path 安装目录`；安装器退出 0，程序文件、记忆目录和卸载登记一致 |
| 同版本重装 | 不传 `/D=`；自动复用上述中文、空格目录，登记版本仍为 0.1.0，无 `%LOCALAPPDATA%/kanban` 副本 |
| 0.1.1 安装器升级 | 不传 `/D=`；复用相同目录，登记版本变为 0.1.1，无默认目录副本 |
| 数据保留 | 用 `KANBAN_DATA_DIR` 指向隔离目录；先由已安装程序初始化真实 SQLite，再插入测试项目、任务和提示词。重装与升级前后数据库 SHA-256 一致 |
| 安装后启动 | 首次与升级后均启动了实际安装的 Release 程序，进程保持运行并打开 SQLite；再次查询原任务及提示词仍存在 |
| 测试卸载 | 测试包自带卸载器退出 0，主程序和卸载登记已移除；独立任务数据库仍保留 |

0.1.1 是通过 `tauri bundle --config` 构建的**仅用于验证的新版安装器**，装载同一份 0.1.0 应用二进制。它证明安装器按版本升级及读取目录的行为，不代表 0.1.1 业务版本或新数据库迁移已经发布。此测试包只保存在 `.scratch/kanban-packaging-acceptance/upgrade-test-only-0.1.1.exe`，不在分发目录。

Tauri 默认卸载会保留记忆目录，只有勾选删除应用数据等特定路径才删除该记录。验收结束检查该记录仍精确指向测试目录后，清除了本次测试创建的记忆目录键；未改动其他安装记录。启动采用隔离数据目录及专用不存在的 Herdr session，没有向个人 pane 发送任务；仅终止本次创建的两个验收进程。

可复查证据：`.scratch/verify-kanban-packaging.py`、`.scratch/kanban-packaging-acceptance-verified/results.json`。测试脚本遇到已有正式安装记录会拒绝执行，不可在正式已安装环境直接强行重跑。

## 打包脚本边界检查

- 构建进行期间再次调用 BAT：构建锁拒绝第二次运行，退出码 1。
- 在含中文和空格的独立仓库副本中制造 Cargo/前端版本不一致：在 npm 安装和编译前拒绝，退出码 1，没有发布目录产物。
- 试跑发现开发服务占用原目录原生依赖，最终流程改为独立源码与依赖目录；开发服务无需退出。
- 首次全并发 Rust 编译曾出现编译器 `STATUS_ACCESS_VIOLATION`；默认限制为 4 个 Cargo 编译任务后完整 Release 构建通过。失败会正常退出，不伪装为成功。
- Windows PowerShell 5.1 对 lockfile 空键的解析及当前环境缺少 `Get-FileHash` 的问题已处理：分别使用 Node 读取锁文件版本、系统 .NET 计算 SHA-256。
- 最终日志适配单独在 Windows PowerShell 5.1 验证：标准输出和错误流进度均可读，空错误流不显示伪异常，原生命令退出码 7 会正确转为失败。

## 未覆盖项

- 原生安装向导逐页点击、目录输入框视觉效果、快捷方式与任务栏缓存未做 GUI 验收；路径复用由真实静默安装和注册表/文件断言验证。
- 当前机器已有 WebView2，未在完全缺少 WebView2 的干净系统验证在线下载；配置与生成脚本包含官方下载分支。
- 未覆盖 Windows ARM64、跨用户、旧 MSI 迁移、删除安装记录、故意更换安装目录、断电或磁盘耗尽场景。
- 不把安装后进程/SQLite 启动检查当作完整 WebView2 界面或真实 Herdr/Codex 任务生命周期验收。
- 未配置签名证书，安装包未签名；未上传、发布或配置自动更新服务。
