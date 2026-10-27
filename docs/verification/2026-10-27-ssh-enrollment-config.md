# SSH 接入目标与密钥配置回归

日期：2026-10-27。

## 问题与修复

接入目标为 `Administrator@100.69.126.63` 时，旧脚本将整个目标写入 `Host`。显式 `ssh -i` 检查能返回远端电脑名，但后续 Herdr/后台 SSH 没有 `-i`，无法从配置选中密钥，再次询问密码或认证失败。

新脚本为显式用户名目标写入 `Match originalhost <host> user <user>`，为普通别名保留 `Host <alias>`。精确迁移本脚本旧条目（包括用户按此前建议修正过的 Host），保留密钥、目标身份及其他用户配置；人工修改导致无法识别时保留现场并报告配置错误。新增默认配置下的非交互 SSH 校验，只有通过后才执行 Herdr 注册。

## 验证

- 修改前：隔离配置下使用真实 Windows OpenSSH `ssh -G`，复现 `Host user@host` 无法选中专用密钥；新回归运行旧脚本时，显式用户名目标在模拟 Herdr 注册处报告 `Herdr would reconnect without the dedicated key`。
- 修改后：`powershell -NoProfile -ExecutionPolicy Bypass -File scripts/test-enroll-windows-machine.ps1`，9 类场景通过：用户名/IP、用户名/DNS、普通别名、用户名/别名、旧条目重试、LF 旧条目、已手动修正条目、人工改动条目保护、隐式认证失败。
- 成功场景同时检查重复接入不改变配置、其他主机及其他用户名不选中本次密钥、原有配置保留。失败场景检查不启动 Herdr 注册。
- `cargo test --lib machine_enrollment::tests`：2 项通过，包括现有授权进程结果观察和新增 PowerShell 回归入口。
- `rustfmt --check src-tauri/src/machine_enrollment.rs` 通过；本次代码文件的 `git diff --check` 通过。

测试使用临时用户目录，网络认证、远端写入和 Herdr 注册为替身；配置解析使用本机真实 `ssh.exe`。没有修改实际用户 SSH 配置、公钥或 Herdr profile，没有执行新任务或重发提示词。

## 验收边界

尚未通过真实桌面输入密码完成整套接入，也未重新打包或安装。当前运行的安装版不会因修改源码而更新；重新构建后的版本重试接入时才会应用迁移。已有的真实远端认证成功日志不能替代本次完整接入验收。
