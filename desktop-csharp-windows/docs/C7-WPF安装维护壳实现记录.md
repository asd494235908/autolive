# C7 WPF 安装维护壳实现记录

## 范围

`GpAutoLive.Installer` 是独立的 Runtime 已就绪维护壳，发行入口为 `GpAutoLive.Setup.exe`。它不引用 `GpAutoLive.App/Core/Contracts/Media/Windows`，不加载媒体运行库，也不修改 Rust/Tauri 参考目录。

它只编排以下固定脚本：

| 操作 | 脚本 | 写入语义 |
| --- | --- | --- |
| Runtime 探测 | `bootstrap-csharp-windows-runtime.ps1` | 本界面只传 `-RequiredMajor 10`，不联网、不安装 |
| 发布包核验 | `verify-release-package.ps1` | 只读；可要求 `-RequireSigned` |
| 安装状态 | `get-csharp-windows-install-state.ps1` | 始终只读，解析 `status/code` 而非只看退出码 |
| 安装/升级 | `install-csharp-windows-package.ps1` | 版本化 staging 与原子活动指针由脚本所有 |
| 回滚 | `rollback-csharp-windows-package.ps1` | 只切换到已验证版本；版本留空使用 `previous_version` |
| 删除旧版本 | `uninstall-csharp-windows-package.ps1` | 先复用状态核验，只允许非活动且非当前回滚候选的已验证版本；保留用户配置与安装指针 |

不支持完整安装根目录卸载、在线 Runtime 下载、签名证书选择、默认客户端切换或媒体资源清理。

## 文件职责

- `GpAutoLive.Installer.csproj`：独立 framework-dependent WPF 发行边界，并把六个既有脚本作为内容复制到 `tools/`。
- `InstallerOperation.cs`：操作到脚本/参数数组的唯一映射，负责路径、版本、目录形状和重解析点前置校验。
- `PowerShellInstallerRunner.cs`：定位 PowerShell 7、限制脚本白名单、无 shell 启动、并行有界读取、超时、只读/预演取消和进程树回收。
- `InstallerResultFormatter.cs`：解析脚本 JSON，投影稳定状态并对错误中的本地/网络路径做有界脱敏。
- `MainWindow.xaml(.cs)`：输入、操作选择、强制预演、二次确认、串行执行、状态复核和关闭门禁。
- `GpAutoLive.Installer.Tests`：纯逻辑、结果投影和进程取消测试，不执行真实安装/回滚/卸载。

安装、回滚和卸载脚本另外共同加载 `tools/csharp-windows-install-transaction-lock.ps1`，用立即失败的 `Local\\GpAutoLive.CSharp.Windows.InstallTransaction.v1` 命名互斥串行化跨维护壳/命令行事务；详见 [`C7 安装事务锁审查记录`](./C7-安装事务锁审查记录.md)。

## 安全与取消

- 脚本路径只能来自 `AppContext.BaseDirectory/tools`，目录和脚本都不能是重解析点；未知脚本立即拒绝。
- 参数通过 `ProcessStartInfo.ArgumentList` 传递，不拼接 PowerShell 命令，不接受任意脚本路径、下载地址、证书或凭据。
- 默认安装目录为 `%LocalAppData%/GpAutoLive.CSharp.Windows.Install`，与现有客户端和 C# 用户配置目录分离；选择已有目录时只允许 `current.json` 与 `versions/`。
- `RequireSigned` 默认开启。真实变更必须先通过相同参数的 `-WhatIf`，然后用户二次确认。
- stdout/stderr 各自最大 256 KiB，单次运行默认 10 分钟。只读检查与预演可以取消；取消会终止进程树并提示重新刷新状态。
- 真实写入不开放手动取消或关窗。现有脚本没有协作式取消点，强杀可能留下 `.staging-*`、`.current-*.tmp` 或部分删除目录；这是本轮保留的安全限制。
- 真实变更返回后，无论脚本成功或失败都重新执行安装状态核验；状态不是 `healthy` 时不显示可继续操作的结论。
- 删除旧版本在脚本内部再次调用安装状态核验；`current.json` 缺失/损坏、活动版本、当前回滚候选或包校验失败均在删除前 fail-closed。
- 安装在读取已有 `current.json` 时校验 schema、活动版本、相对路径和上一版本；版本目录已移动但活动指针写入失败或取消时，会清理本次新版本目录并保留旧活动指针。

## 本机验证

在 `desktop-csharp-windows/` 下执行：

```powershell
.\.tools\dotnet\dotnet.exe restore .\GpAutoLive.Windows.slnx --use-lock-file
.\.tools\dotnet\dotnet.exe build .\GpAutoLive.Windows.slnx -c Release --no-restore
.\.tools\dotnet\dotnet.exe test .\GpAutoLive.Windows.slnx -c Release --no-build
.\.tools\dotnet\dotnet.exe format .\GpAutoLive.Windows.slnx --verify-no-changes --no-restore
.\.tools\dotnet\dotnet.exe publish .\src\GpAutoLive.Installer\GpAutoLive.Installer.csproj -c Release -r win-x64 -p:Platform=x64 --self-contained false --no-restore
```

结果：

- Release 构建：0 警告、0 错误。
- 自动化：417/417 通过，其中 Installer 14/14；主线程审查补充了未知状态/非对象 JSON 的 fail-closed 测试，以及无 `status` 的发布包清单成功投影回归。
- framework-dependent publish：11 个文件、319,324 bytes；`GpAutoLive.Setup.exe` 162,816 bytes，`GpAutoLive.Setup.dll` 64,000 bytes。
- 窗口冒烟：`WaitForInputIdle=True`、窗口标题 `GpAutoLive 安装维护助手`、顶层窗口存在、`CloseMainWindow=True`、退出码 0。
- 既有安装夹具：对当前 `previous_version` 执行删除 WhatIf 返回拒绝，版本目录前后保持 `v83-test,v83-test-3` 不变。
- 发布包结果投影：核验器成功返回的清单 JSON 没有 `status` 字段；维护壳按 schema 1 和四个必需非空清单组判定通过。
- `GpAutoLive.exe` 仍为 162,816 bytes；未因维护壳增加引用或内容。

## 未验收

- 维护壳是 framework-dependent，缺少 .NET 10 Desktop Runtime 的裸机无法启动；Runtime 按钮只能用于已能启动维护壳后的状态复核。
- 现有脚本依赖 PowerShell 7；Windows PowerShell 5.1 不兼容。本轮没有扩大范围去重写原子移动、进程和下载边界。
- Installer/脚本 Authenticode、固定发布位置 ACL、真实 UAC/拒绝提升/3010 重启、安装中断恢复、完整安装根卸载、开始菜单入口与干净 Windows 10/11 在线/离线矩阵未验收。
- 当前 v83 候选未签名，保持“代码已接入·待发布验收”；不得据此切换默认客户端。
- C7 锁竞争脚本 `tools/test-c7-install-transaction-lock.ps1` 已验证安装、回滚和卸载在外部事务占用时均 fail-closed，且 WhatIf/锁竞争均未创建安装目录。
- C7 边界脚本 `tools/test-c7-install-boundaries.ps1` 已验证损坏指针拒绝、有效 WhatIf 只读、指针激活失败清理新版本且保留旧 `current.json`。
