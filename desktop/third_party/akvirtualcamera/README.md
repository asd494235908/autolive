# AkVirtualCamera 发布边界

本目录锁定唯一上游 `webcamoid/akvirtualcamera`，提交为
`9cf77ae6379e5f635255f4b377478d388a46a3b2`，许可证身份为
`GPL-3.0-only`。不可变来源、构建目标和发布硬门禁记录在
[`upstream.lock.json`](./upstream.lock.json) 中。

当前仓库已提交经过 SHA-256 校验的上游源码归档和 CycloneDX SBOM，但**不包含构建产物、
DLL、EXE 或安装包**。法务审查、GPU 长测和兼容矩阵目前仍是待审状态，因此这仍不是
可发布的虚拟摄像头资源；源码对应件、x86/x64 DirectShow、独立 sidecar、签名及全部
发布证据齐全前必须保持阻断。

本机开发构建入口（需要已安装的 MSVC/CMake，且设置
`$env:CMAKE_GENERATOR = 'Visual Studio 17 2022'`）：

```powershell
.\desktop\third_party\akvirtualcamera\build-directshow.ps1 -Architecture x86
.\desktop\third_party\akvirtualcamera\build-directshow.ps1 -Architecture x64
.\desktop\third_party\akvirtualcamera\build-sidecar.ps1
```

这些脚本只读取锁定归档并输出未签名开发构建哈希，不安装系统设备；sidecar 构建会
把 `vcam_capi.dll` 复制到其同目录。开发构建结果不能直接填入发布 `artifacts/`。

取得受信任代码签名证书后，可运行
`sign-akvirtualcamera-artifacts.ps1 -CertificateThumbprint <40位指纹>` 为锁定的 7 个
产物签名并生成 Authenticode 证据；没有有效证书时脚本会 fail-closed。

运行以下命令可查看门禁报告：

```text
node desktop/tools/verify-akvirtualcamera-lock.mjs
node desktop/tools/verify-akvirtualcamera-lock.mjs --require-release-ready
```

第二条命令在当前状态应以退出码 `2` 失败。AkVirtualCamera 作为独立 GPL
sidecar/安装组件承接原始帧和 DirectShow；Rust 主程序在法务审核前不得直接
链接其 C API。首版固定 Windows 10/11 DirectShow、YUY2 `1280×720@30fps`，
Windows 11 Media Foundation 仅保留实验门禁，`zero_copy=false`。sidecar 的 stdout
仅回报受限的 `GPAKVC_CLIENTS <数量>` 状态行，供桌面端区分 `Ready`（无下游客户端）
和 `Streaming`（至少一个客户端）。会话令牌通过继承 stdin 一次性传递，sidecar 不接受
命令行或环境变量中的令牌；Named Pipe 继续使用当前用户 ACL 并拒绝远程连接。

设备验收前可用 `run-test-pattern.ps1` 发送 CPU 生成的 YUY2 `1280×720@30fps`
测试帧并生成证据 JSON；该工具不安装设备，未检测到正式设备时必须保持 `blocked`。
