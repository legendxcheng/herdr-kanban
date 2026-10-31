# DeepSeek 自动标题验证

日期：2026-10-31。规格：[GitHub #45](https://github.com/legendxcheng/herdr-kanban/issues/45)。

## 实现范围

- TaskTitles 注入 DeepSeekTitles，本地／SSH 项目都从看板电脑请求固定官方 HTTPS 端点。固定 deepseek-flash，关闭思考，非流式 JSON 标题，最多 128 输出 token；不启动 Codex 或部署标题 helper。
- 保留先存摘要、两路 worker、2000 字符输入、40 字符单行标题、编辑／归档／删除保护，以及持久化重试／重启／手动重试。无 IPC、数据库迁移或正常开发路由变更。
- 临时网络／超时／429／5xx 等按既有预算重试；缺 Key、认证、余额和参数等永久错误立即停止，提供中英文提示。请求总时限 30 秒、响应上限 64 KiB，退出取消覆盖等待响应头和响应体。
- Git 忽略项目 `.secrets/`；Rust build.rs 读取裸 Key 并生成 OUT_DIR 常量，打印信息不包含 Key。正式打包入口在复制源码及编译前验证私密文件。真实 Key 由用户自行填入，未进入 Issue、源代码或前端。
- 删除旧 title_routing 模块；remote-helper 的旧 task_title/CLI 分支保留兼容，当前看板不再调用。

## 已执行检查

| 检查 | 结果与范围 |
| --- | --- |
| `cargo check --manifest-path src-tauri/Cargo.toml -j 4` | 通过，初次添加 HTTPS 依赖时同步锁文件；仅已有未使用代码警告 |
| `cargo test --manifest-path src-tauri/Cargo.toml --locked -j 4 --lib -- --test-threads=4` | 320 通过，34 项环境依赖/live 测试跳过；覆盖当前共享工作区的 Rust 常规套件 |
| `npm test -- --maxWorkers=4` | 42 个文件、311 项通过，含标题卡片/重试和中英文目录校验 |
| `npm run build` | TypeScript 与 Vite 生产构建通过；保留大于 500 kB bundle 的既有提示 |
| Windows PowerShell 5.1 执行 `scripts/test-deepseek-key.ps1` | 8 类检查通过：缺失、空白、Bearer 前缀、多行、超长、正常及 BOM/行尾；测试用假 Key，校验不输出凭据 |
| `git check-ignore .secrets/deepseek-api-key.txt` | 确认私密文件被忽略 |
| 凭据扫描 | 使用本机 Key 在内存中检查已跟踪及未被忽略的源文件、Issue 规格临时文件和 dist，未发现 Key；检查只输出结论 |
| `git -c core.whitespace=cr-at-eol diff --check` | 通过；保留仓库已有 CRLF 文件行尾 |

HTTP 测试直接接入生成器及真实内存 SQLite，验证：

- 本地和无可用 SSH 配置的远端项目使用同一请求契约；只传文字及引用显示名，无文件正文、图片或工具；请求期间数据库可用。
- 400/401/402/403/404/408/422/429/500/503 的状态分类、摘要保留、稳定错误码和重试计划；单次尝试没有客户端内层重试。
- 无效 JSON、空 choices、输出截断、工具调用、额外字段、空白／换行／Unicode 行分隔符不覆盖摘要。
- 带 Content-Length 和 chunked 的超大响应均拒绝；307 不跟随、不向目标发送认证请求。
- 缺失／无效 Key 和无效输入不发网络请求；服务端直接断连仅返回稳定 network 错误，不泄漏诊断。
- 响应头和响应体分别阻塞时，总超时有效；服务 shutdown 在测试的一秒界限内完成，持久化取消及已消耗尝试次数。
- 原有 SQLite 标题套件继续覆盖有限重试、人工修改、正文修改、删除／归档、优先级修改、重启及并发。

初次测试把 Windows 空闲端口假定为立即拒绝连接，实际返回超时，导致单项断言失败。网络错误映射本身符合预期；测试改为模拟服务接收请求后不返回 HTTP 头便断开连接，消除操作系统时序假设。完整套件随后通过。

## 真实 DeepSeek 调用

用户填入私密文件后，运行：

`cargo test --manifest-path src-tauri/Cargo.toml --locked -j 4 --lib title_deepseek::tests::live_deepseek_title_updates_real_sqlite -- --ignored --exact`

结果：1 通过。文件变更触发重新编译；一次真实付费 API 请求通过现有 TaskTitles worker 将隔离测试任务的状态更新为 succeeded，并写入非空、最多 40 字符标题。测试执行约 1.03 秒，仅为该单次样例时间，不作为普遍延迟保证。未使用用户任务数据库，未输出 Key 或服务响应正文。

## 验收边界

- 尚未执行真实 Tauri/WebView2 桌面点击、应用重启、实际 SSH Workspace 的 UI 全链路验收；模拟离线 SSH 项目不等于真实跨机桌面验收。
- 未构建 NSIS 安装包、安装或替换运行中的应用；构建入口的 Key 检查通过不代表带 Key 安装包已经验收。
- 内置 Key 可以从二进制提取，编译缓存和源码快照也含凭据；现有 SSH helper 是同一二进制。相关范围已记入打包说明。
- 验证基于共享工作区，保留已有 Herdr 提交链路及结构文档改动；未提交或推送。
