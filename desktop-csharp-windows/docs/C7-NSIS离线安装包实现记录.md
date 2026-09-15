# C7 NSIS 离线安装包实现记录

## 范围

`installer/windows/GpAutoLive.nsi` 是 C# WPF 客户端的首次安装入口。它直接消费通过 `verify-release-package.ps1` 的目录包，不复用依赖 .NET 与 PowerShell 7 的 `GpAutoLive.Setup.exe` 维护壳，也不携带 C# 未使用的 WebView2。

安装器只负责：

- Windows 10 2004+/x64 准入；
- 离线安装或修复微软 `.NET 10.0.11 Windows Desktop Runtime x64`；
- 把完整应用载荷安装到 `%ProgramFiles%\GpAutoLive\app`；
- 运行中拒绝升级/卸载，同卷 staging 激活失败时恢复旧 `app`；
- 通过应用目录内 `GpAutoLive.control-plane-profile` 区分 `production-v1` 与 `cloud-test-v1`，不写 Windows 全局环境变量；
- 创建开始菜单、桌面快捷方式和标准卸载登记；
- 卸载安装器拥有的程序目录，保留 `%LocalAppData%` 配置、Credential Manager 凭据和用户媒体。

## 构建与核验

```powershell
.\tools\build-csharp-windows-nsis.ps1 `
  -PackageRoot .\artifacts\csharp-windows-controller-20260907-v98 `
  -Version v98 `
  -DevelopmentUnsigned `
  -ControlPlaneProfile CloudTest

.\tools\verify-csharp-windows-nsis.ps1 `
  -InstallerPath .\artifacts\GpAutoLive-Setup-v98-TEST-UNSIGNED-LEGAL-REVIEW.exe
```

构建入口固定校验 .NET Runtime 的 SHA-512 与微软 Authenticode，使用 NSIS 3.12+ 和 solid LZMA。正式模式还要求目录包已签名且 `mpv-runtime-manifest.json` 的法律审核、对应源码和第三方告知全部通过。无证书或法律审核未完成时只能显式使用 `-DevelopmentUnsigned`，输出名强制携带 `UNSIGNED-LEGAL-REVIEW`。

## 当前证据与限制

- v98 云测试安装器：169,556,947 bytes（161.70 MiB），SHA-256 `f5559c3d189e4cdb49db4ec7951258b2ff5d27ea75db68d047824e3a7c0dc53a`；
- `CloudTest` 口味名称强制携带 `TEST`，默认连接固定云测试地址；`offline` 可强制禁网，任意其他远端地址不能覆盖固定值。测试凭据前缀为 `GpAutoLive.CSharp.Windows.Test/`，不读取生产 Refresh Token、待撤销 Token 或设备身份；
- `Production` 口味写入 `production-v1`，忽略遗留的 `test/development` 环境配置，只接受显式 HTTPS 地址；
- v95 首次安装会在解压后把 NSIS 当前输出目录留在 `.staging-v95`，Windows 因此拒绝把当前目录重命名为 `app`；v96 在激活前显式切回 `$INSTDIR`，并由静态顺序回归门禁覆盖；
- `.NET 10.0.11` 离线安装器：60,001,888 bytes，SHA-512 与微软发布元数据一致，Authenticode `Valid`；
- NSIS 编译无警告，安装器元数据与 CRC 核验通过；
- 当前外层安装器与应用 PE 未签名；mpv 清单仍为法律审核阻断，FFmpeg 第三方告知与对应源码材料仍不完整；
- 当前 Codex 进程没有管理员令牌，未弹出 UAC 干扰用户，因此真实静默安装/升级/卸载、快捷方式和注册表回收仍待管理员会话与干净 Windows 10/11 验收。

以上限制未解决前，v98-TEST 只能使用低权限测试账号做内部联调，不能称为正式生产安装包；v95 已知无法完成目录激活，不得继续交付。
