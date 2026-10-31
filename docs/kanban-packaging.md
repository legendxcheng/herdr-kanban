# Kanban Windows 打包

在仓库根目录双击 `build_kanban.bat`。成功后自动打开 `release\kanban\<版本号>\`，分发其中的 `Herdr-Kanban-<版本号>-windows-x64-setup.exe` 即可；同目录提供使用说明和 SHA-256 校验文件。控制台结束时保留窗口，失败日志位于 `release\kanban\logs\`。

## 构建机器准备

- Windows x64、Node.js 22.12 或以上受支持的 LTS 版本、npm。
- Rust MSVC 工具链，包含 `x86_64-pc-windows-msvc` target。
- Visual Studio Build Tools 的“使用 C++ 的桌面开发”及 Windows 10/11 SDK。
- 首次构建及缺少缓存时需要网络：npm、Cargo、Tauri 的 NSIS 构建工具可能需要下载。
- 自动取名使用 DeepSeek：在仓库根目录创建 `.secrets/deepseek-api-key.txt`，只填裸 API Key，不加引号或 `Bearer`。`.secrets/` 已被 Git 忽略；勿提交、截图或复制到 Issue。正式打包入口在构建前检查缺失、空白及格式错误，错误不会回显文件内容。

Key 由 Rust 构建脚本读取并编译进程序，不通过前端、运行时配置或 Cargo 输出传递；用户安装后无需配置。修改或移除私密文件会触发重新构建。普通开发构建允许缺少文件，自动标题保留摘要并提示未配置 Key；真实 API 测试需配置后重新编译。更换/撤销 Key 后须重新构建和分发。私密文件会随源码快照复制到被忽略的 `release/kanban-source/.secrets/`，编译缓存及二进制也包含该凭据，这些产物不应公开分享。安装包只包含内置值，不分发源私密文件；内置 Key 仍可从二进制提取。现有 SSH helper 使用同一二进制，正常远端操作部署 helper 时也会复制含 Key 的程序；标题请求本身仍只在看板电脑发起。

脚本调用 Windows PowerShell 5.1，无需安装 PowerShell 7；启动时检查依赖。先将当前源码镜像到专用的 `release\kanban-source\`（排除 node_modules、dist、target、.git、release 和 .scratch，避免将副本再次复制进自身），在其中使用 `npm ci` 恢复锁定依赖，Tauri Release 构建使用 Cargo `--locked`，不自动升级依赖。构建缓存位于 `release\kanban-build\`，不会覆盖正在运行的 debug 可执行文件或开发目录依赖。上述目录由脚本管理，不要在里面编辑源码。重复双击打包入口会被构建锁拒绝。

自动化调用：`build_kanban.bat -NoPause -NoOpen`。`-NoPause` 需放第一个参数；失败返回非零退出码。构建失败不会把中间产物复制为本次安装包，先前成功的安装包仍保留；只有看到本次 `SUCCESS` 才代表本次成功。

默认最多同时运行 4 个 Cargo 编译任务，减少打包对开发机器的压力；显式设置的 `CARGO_BUILD_JOBS` 会被尊重。

2026-10-25 修正为 WezTerm 直接运行官方 Herdr 客户端。beforeDev/beforeBuild 分别执行 npm run dev/npm run build；不再构建或打包 terminal bridge。WezTerm 与 Herdr 本身均不打入安装包。

程序在 PATH、Program Files/WezTerm 和 LocalAppData/Programs/WezTerm 查找 wezterm-gui.exe；可用 KANBAN_WEZTERM_EXECUTABLE 显式指定。WezTerm 正常读取用户配置，Kanban 仅覆盖无边框和隐藏 tab bar，不写专用鼠标/按键配置。Herdr 使用 connect-only 的 client 入口连接已运行 session。当前任务定位要求支持 endpoint generation 1 的客户端范围 pane.focus；本次验证 Herdr 0.9.5、WezTerm 20240203-110809-5046fc22。进程内专用管道只原样传输官方协议帧并加入首次定位，不额外打包 bridge。详细证据见[独立导航验收](verification/2026-10-25-herdr-client-navigation.md)。

Herdr 入口优先级为 `KANBAN_HERDR_EXECUTABLE`、PATH、`%LOCALAPPDATA%/Programs/Herdr/bin/herdr.exe`。Windows PATH 支持 PATHEXT 中的 `.exe/.com/.cmd/.bat`；保留自定义 launcher 设置的 Herdr 配置及数据目录。同步、socket 查询和终端使用同一入口，Workspace 数据目录通过该入口报告的 session/socket 核验后读取。PATH 必须对 Kanban 进程可见；修改系统/用户 PATH 后应从已更新环境重新启动看板。无需配置远端机器即可使用本地 Workspace。

## 发布新版本

1. 在仓库根目录执行 `npm version 0.1.1 --no-git-tag-version`（替换为实际版本），同步更新 package.json/package-lock.json。
2. 将 `src-tauri/tauri.conf.json` 的 version 和 `Cargo.toml` 的 package.version 改为同一版本。
3. 在 `src-tauri/` 执行 `cargo check`，同步 Cargo.lock；提交这些版本文件。打包脚本会拒绝版本不一致或过期的 Rust 锁文件。
4. 双击根目录 `build_kanban.bat`，验收安装包后分发。脚本本身不会自动增加版本或发布到网络。

## 安装身份与原路径更新

固定使用 NSIS、当前用户安装。已有安装身份保持为 productName `kanban`、publisher `administrator`、identifier `com.administrator.kanban`；不要因改展示名称而随意更改这些值。安装包文件名使用 Herdr Kanban，现有系统安装项和快捷方式继续使用 kanban。

Tauri NSIS 模板在安装完成时将目录写入 `HKCU\Software\administrator\kanban` 的默认值；下次启动安装器时 `RestorePreviousInstallLocation` 读取它，并填入安装目录。卸载登记使用 `HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\kanban`。同版本重装、新版本升级均复用此身份，不使用含版本号的默认安装目录。首次默认为 `%LOCALAPPDATA%\kanban`，可自行选择目录。正常更新沿用原路径；用户手动修改路径或显式传 `/D=` 仍可覆盖选择。

这个保证适用于同一 Windows 用户且安装记录仍存在。跨用户安装、手工删除注册表、旧 MSI 的迁移或彻底卸载后的新装不属于“读取上次目录”的验收范围。程序数据保存在 `%APPDATA%\herdr-kanban`，不随二进制更新删除；数据库迁移仍由应用启动时执行。

WebView2 缺失时安装器联网下载；已有运行时直接复用。目标用户不需要编译工具，但需自行准备 Herdr、Codex CLI、WezTerm 及账户配置。只产出 Windows x64 安装包，不产出 MSI、ARM64 或其他平台安装包。当前未配置代码签名证书，产物为未签名安装包。

## 验证

打包成功与桌面使用验收分开记录。至少覆盖首次自选目录、同版本重装、新版升级、安装目录登记与文件一致、任务数据保留。真实安装验证及其限制见 [Windows 打包验收记录](verification/2026-10-24-kanban-packaging.md)。
