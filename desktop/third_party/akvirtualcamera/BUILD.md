# AkVirtualCamera 构建与发布门禁

## 2026-09-09 C# 本地开发测试路径

`desktop-csharp-windows/tools/build-virtual-camera-development.ps1` 校验锁定源码归档，
复用本地 MSVC/CMake 构建，并将六项运行组件放到
`desktop-csharp-windows/artifacts/virtual-camera-development/akvirtualcamera`。
独立 `development-manifest.json` 只保存测试组件 SHA-256，采用无 BOM UTF-8；
不会生成 `release-ready.json` 或改写正式产物验收事实。

首次注册必须在管理员 PowerShell 运行：

```powershell
./desktop-csharp-windows/tools/install-virtual-camera-development.ps1 `
  -PackageRoot E:\aotlve\desktop-csharp-windows\artifacts\virtual-camera-development
```

脚本验证本地普通路径、六项哈希和安装所有权，拒绝覆盖未知或第三方配置。
为专用 `GpAutoLiveCamera` 创建单个 YUY2/30fps 格式后，只向指定开发用户授予
该设备 `Formats\1` 注册表键的 `SetValue`，不开放安装路径、其他摄像头或 HKLM 根。
上游 x86/x64 Preferences 均读取 64 位视图，因此只有该视图拥有格式事实。
卸载使用同一命令加 `-Action Uninstall`；执行前应关闭所有摄像头下游。

显式开发启动只在 C# 测试信任策略通过后给 sidecar 增加 `--development-format`。
sidecar 从首帧读取实际源宽高，检查偶数宽、边长不超过 4096、像素不超过
4096×2160，写入前验证固定设备、格式和所有者；写失败恢复旧宽高，恢复失败
单独报错。生产无此开关，只允许已经配置匹配的设备尺寸。协议 v1 保持 1280×720，
v2 使用真实源尺寸且每个会话固定；帧最终以真实 width/height/stride 调用
`vcam_stream_send`。首次调用成功才发 `GPAKVC_OUTPUT_READY`，管道连接不算输出成功。

**切换分辨率需要关闭并重新打开下游摄像头。** 上游 direct mode 只接受第一个注册格式，
不能通过列出多个格式解决自动重协商。检测到下游连接时开发输出拒绝重配（退出码 6），
格式/权限不匹配为退出码 7；本次不宣称下游无缝切源。

已执行 native 帧边界测试、进程内 HKLM 替身的真实 Win32 格式读写测试、六项 PE 位数/
哈希测试及脚本语法检查。当前会话无管理员权限，尚未执行设备注册和真实 DirectShow
收帧，不把编译或 C API 接收确认等同于下游真实业务验收。以下原有正式签名发布流程
仍适用于正式包，不能用开发清单代替。

本文件描述本机构建所需的边界，不触发下载，也不把源码放到服务器构建。
当前仓库已保存不可变上游源码归档，但尚无签名发布产物；构建与发布仍必须
fail-closed。

## 固定输入

1. 读取 [`upstream.lock.json`](./upstream.lock.json)，只接受其中的仓库、提交、
   归档大小和 SHA-256。归档应在本机受控输入目录中准备，并先做大小和哈希校验。
2. 使用 MSVC/CMake 分别构建 x86 与 x64 DirectShow 组件；构建网络必须为 `none`。
3. 独立构建 x64 GPL sidecar，通过当前用户 ACL 的 Windows Named Pipe 与桌面端
   控制；不得把 AkVirtualCamera C API 链接进 Rust 主程序。sidecar 的源码、CMake
   注入文件和离线构建脚本位于本目录，可用 `build-sidecar.ps1` 调用本机 MSVC/CMake。
   两个构建脚本会在解包后应用 `patches/0002-loopback-service-socket.patch`，把上游
   C API/Service 的遗留 TCP 控制面限制到 `127.0.0.1`。这只是降低远程暴露面；上游
   消息协议仍没有随机认证，命名管道迁移或认证握手完成前不能进入正式发布门禁。
4. 仅允许 DirectShow 作为 Windows 10/11 首版端点。Windows 11 Media Foundation
   的 `MFCreateVirtualCamera` 不在默认构建和安装路径中。

## NSIS 组件生命周期

通过发布门禁的组件在打包前复制到 `desktop/src-tauri/akvirtualcamera/` 资源根；对外分发所需的
`COPYING`、修改说明、对应源码清单和 SBOM 同步暂存。签名证据、法务审核、GPU 基准和兼容
矩阵保留在 `desktop/third_party/akvirtualcamera/`，不复制进运行时资源树：
DirectShow 文件位于 `x86/`、`x64/`，Assistant/Manager 位于 `x64/`，sidecar 和 C API
位于 `bin/`，并随根目录携带 `release-ready.json`。桌面端运行时只从该受信任资源根解析
`bin/akvirtualcamera-sidecar-x64.exe`；不会接受前端传入的任意路径。测试包只带资源目录
说明文件，因此保持未安装状态。

`desktop/src-tauri/windows/hooks.nsh` 引入 `installer/akvirtualcamera-components.nsh`。
只有安装包内存在发布门禁生成的 `akvirtualcamera/release-ready.json` 时，Hook 才会
继续执行；它随后校验固定的 x86/x64 DirectShow、x64 Assistant/Manager、sidecar
和 C API 文件，写入双注册表视图，分别调用对应位数的 `regsvr32`，创建或更新
`GpAutoLiveCamera`，并在任一步失败时回滚设备、注册和注册表所有权。卸载只移除
本产品的设备和过滤器，不执行全量设备删除或终止其他实例进程。测试包没有该标记，
因此保持 fail-closed，不会注册系统摄像头。

## GPU 边界

上游只接收 CPU 原始帧。产品链必须先在非 WARP D3D11/WGC 上完成最终窗口捕获、
缩放和 YUY2 转换，然后做一次有界 staging readback，再交给 sidecar。若硬件
Video Processor 不支持 YUY2 输出，先输出 BGRA，再由 D3D11 像素着色器在 GPU
上打包为 YUY2；CPU 只复制 staging 字节，不执行色彩转换。禁止 GDI 截图、CPU
重做 GPU 效果、第二播放器、逐帧编码或把该路径宣称为零拷贝。

### 720p30 技术样例

Rust 技术样例 `desktop/src-tauri/examples/virtual_camera_gpu_benchmark.rs` 只接收
最终效果窗口 HWND，运行 WGC→D3D11 Video Processor→GPU YUY2 pack→三槽 staging 捕获泵，输出
GPU 回读 P50/P95/P99、帧时间戳单调性、至少 95% 的目标帧覆盖率、interframe P95、实际
捕获 adapter 事实（LUID/名称/VendorId/DeviceId/Feature Level）和进程资源采样。它还会
在每秒采样当前 benchmark 进程的工作集与虚拟内存；正式 7200 秒证据要求至少两个采样，
工作集峰值相对首样本增长不超过 64 MiB、虚拟内存峰值增长不超过 256 MiB。帧数不足、
interframe P95 超过 `50,000µs` 或资源增长超出预算时报告失败，避免静止窗口、只收到少量
帧或资源持续增长却被误报为 30fps。它不启动 sidecar、不注册设备，也不修改本地播放。
示例命令（必须在 Windows 目标机执行）：

```powershell
cargo run --manifest-path desktop/src-tauri/Cargo.toml --example virtual_camera_gpu_benchmark -- `
  --hwnd 0x123456 --seconds 30 `
  --output C:\\Temp\\gpu-benchmark-720p30.json
```

只有使用真实 mpv/libplacebo 最终效果 HWND、在目标 GPU 上完成运行并人工复核
报告后，才能把报告复制到锁文件声明的 `gpu-benchmark-720p30.json`。本机 AMD
Video Processor 只提供 BGRA 输出，GPU pack 路径已在短测中通过，最新 3 秒测得
P50/P95/P99 回读约为 3.1/12.2/12.2ms；该结果不能替代 2 小时、Win10/11、多厂商
和下游兼容矩阵，也不能填充为发布门禁。不能改用 CPU 色彩转换或把短测报告冒充发布通过。

## 进入发布包前

必须把每个产物的 SHA-256 和 Authenticode 证据写入锁文件声明的路径，并补齐
`COPYING`、修改说明、对应源码清单、CycloneDX SBOM、法务审核、720p30 基准和
Windows 10/11 DirectShow 兼容矩阵。校验器还会检查 SBOM 的锁定提交、法务
`status: approved`、至少 7200 秒的 GPU 基准以及矩阵中每个必需条目的
`status: passed`；不能只放置空文件或未审查占位内容。缺任一项时执行：

```text
node desktop/tools/verify-akvirtualcamera-lock.mjs --require-release-ready
```

不能用 test pattern、空文件或未签名内部包把门禁标为通过。

## 本机 sidecar 构建

在已安装 Visual Studio C++ 和 CMake 的 Windows 开发机上，设置
`CMAKE_GENERATOR` 后执行：

```powershell
$env:CMAKE_GENERATOR = 'Visual Studio 17 2022'
.\desktop\third_party\akvirtualcamera\build-sidecar.ps1
```

脚本只读取仓库内锁定的源码归档和 overlay，输出到 `local-build/`，不会写入系统
设备注册表，也不会自动签名或把 `release_ready` 标成 true。x86/x64 DirectShow
组件仍须按上游构建脚本分别构建；sidecar 仅允许 x64。使用同一个 `BuildRoot`
重复执行时会先验证补丁是否已经应用，避免二次打补丁；如果源码既无法应用补丁也
无法验证为已应用，脚本会失败而不会继续构建。

## 设备验收

在安装正式签名包的 Windows 10/11 验收机上运行
`verify-akvirtualcamera-device.ps1 -Output <绝对路径>`。脚本只读检查固定的
`GpAutoLive Camera` PnP/DirectShow 端点、x86/x64 注册表所有者、`AkVCamManager`
枚举和固定组件路径；它不会调用 `regsvr32`、添加或删除设备，也不会终止其他进程。
报告中的 CPU test-pattern、下游应用可见性和退出/崩溃/卸载清理证据可分别通过
`-TestPatternEvidence`、`-DownstreamEvidence` 和 `-CleanupEvidence` 传入；三类证据
必须全部为 `status=passed`，否则脚本返回退出码 `2`，不能把“无设备”误报为通过。

CPU 原始帧 test-pattern 由 `run-test-pattern.ps1` 发送到独立 sidecar，固定生成
YUY2 `1280×720@30fps`，并输出可传给上述设备验收脚本的 JSON。该工具只启动指定的
sidecar、通过当前用户 Named Pipe 投递测试帧，不注册或卸载设备；sidecar 未安装正式
设备时会返回 `blocked`，不能把它当作设备兼容性通过证据：

```powershell
.\desktop\third_party\akvirtualcamera\run-test-pattern.ps1 `
  -SidecarPath C:\Program Files\GpAutoLive\akvirtualcamera\bin\akvirtualcamera-sidecar-x64.exe `
  -Seconds 10 `
  -Output C:\Temp\akvirtualcamera-test-pattern.json
```

## 正式产物签名

在已获得受信任代码签名证书（当前用户证书库、包含代码签名 EKU 和私钥）后，使用
本目录的 `sign-akvirtualcamera-artifacts.ps1`。脚本只读取锁定的 7 个产物，使用
SHA-256 Authenticode 签名并执行 `signtool verify /pa /all`，随后为每个产物生成
签名证据 JSON；没有完整产物或有效证书时直接失败，不会生成占位证据。签名完成后
仍需人工复核并把产物和证据 SHA-256 写回 `upstream.lock.json`，再运行发布校验器。

### DirectShow COM regression (2026-09-10)

Both build entry points now apply `patches/0003-directshow-com-apartment.patch` after the loopback patch. To test a staged upstream source without administrator rights or registry writes:

```powershell
./test-directshow-registration.ps1 -SourceDirectory <staged-upstream-root> -OutputDirectory <local-test-output>
```

Requires local Visual Studio MSVC Build Tools. Tests compile the real registration/removal bodies with registry mutations replaced by injected results; actual COM initialization and FilterMapper creation remain real. Clean upstream fails; patched source passes. Real elevated device registration and downstream capture are separate acceptance steps.

The fixed source also receives `0004-directshow-capture-pin-category.patch`. Run `./test-directshow-pin-category.ps1 -SourceDirectory <staged-upstream-root> -OutputDirectory <local-test-output>` to validate the existing Pin's read-only category interface and buffer boundaries. The check compiles actual source methods, and does not open a capture session or register devices.

### IPC wakeup regression (fixed by 0005)

`./test-ipc-frame-wakeup.ps1 -SourceDirectory <staged-upstream-root> -OutputDirectory <local-test-output>` fails on the unpatched pinned source: an empty-slot wait retains the global broadcasts mutex and delays both producer and stop about one second. With 0005 it verifies prompt producer/stop wakeup and that the slot and shared memory remain alive until the callback is joined.

### Cancellable client IPC (0005) and idle black (0006)

Both local build entry points apply 0005 and 0006 after the existing patches. Run:

```powershell
./test-ipc-frame-wakeup.ps1 -SourceDirectory <staged-upstream-root> -OutputDirectory <local-wakeup-test-output>
./test-ipc-socket-lifecycle.ps1 -SourceDirectory <staged-upstream-root> -OutputDirectory <local-network-test-output>
```

The second test runs actual MessageClient/Socket source against a private loopback listener, covering request/reply, cancelled receive/send, the shared 5-second reply deadline and joined workers. It does not contact the registered Assistant or camera. The existing sidecar CMake also builds AkVirtualCameraBlackFrameTest for 0006. These checks do not substitute for real source-size capture, stop-black and application shutdown acceptance.
### Assistant dispatch wakeup (0007)

A real capture with only updated clients still stalled, so both build entry points now additionally apply 0007. The running Assistant must be updated and restarted through the approved administrator installation/update step; client-only replacement is insufficient. No device re-registration or wire-format change is required. `./test-ipc-service-wakeup.ps1 -SourceDirectory <staged-upstream-root> -OutputDirectory <local-test-output>` compiles actual dispatch/service methods and verifies broadcast wakeup while a listener waits, one listener record per connection, and immediate inactive notification on broadcaster disconnect. All three fail before 0007 and pass after it. This check does not restart the installed Assistant.

### Replaying patches in an existing build directory

Before re-extracting the fixed upstream archive, both entry points remove only the known patch-generated `windows/dshow/BaseFilter/src/yuy2_black_frame.h` inside the fixed upstream source directory. Archive extraction overwrites upstream files but does not remove newly added files; without this refresh a second 0006 application would fail. No source directory is recursively deleted, and unrelated files remain untouched. Both entry points were verified through two consecutive archive extraction/0002–0007 patch cycles in independent validation directories.
