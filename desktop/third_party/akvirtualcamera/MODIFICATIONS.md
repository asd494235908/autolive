# GpAutoLive AkVirtualCamera 修改说明

本目录没有改写已锁定的上游归档；上游提交仍固定为
`9cf77ae6379e5f635255f4b377478d388a46a3b2`。GpAutoLive 通过独立 GPL sidecar
承接原始帧，不把 AkVirtualCamera C API 链接进 Rust 主程序。

当前叠加文件实现以下边界：

- `sidecar/src/main.cpp` 从应用目录按固定文件名加载上游 `vcam_capi.dll`，只查找
  精确描述为 `GpAutoLive Camera` 的设备，验证安装器已配置 `mmap`、direct mode，
  并提交 YUY2 `1280×720@30fps`；不在运行时调用需要 UAC 的配置 setter，也不添加、
  删除或修改其他设备。安装器在提升阶段一次性执行这两项配置，sidecar 因而可保持
  当前用户权限并支持无提示恢复。sidecar 同时通过
  `vcam_clients` 查询下游客户端数量，以 `GPAKVC_CLIENTS` 状态行回报父进程，
  不传递 PID 或路径。
- sidecar 只接受 `--session-token-stdin`，从父进程继承的 stdin 一次性读取 32 位十六进制
  令牌；兼容 Windows PowerShell 5 重定向流自动附加的 UTF-8 BOM，但仍严格要求其后为
  32 位 ASCII 十六进制字符。令牌不进入 argv 或环境变量。它使用该令牌派生当前用户 ACL
  的 Named Pipe，并设置 `PIPE_REJECT_REMOTE_CLIENTS`。帧头、代际、序列、尺寸、payload
  长度和保留字段均固定校验，旧代际/旧序列只丢弃，不分配无界缓冲。
- `cmake/enable-gpautolive-sidecar.cmake` 和 `build-sidecar.ps1` 通过离线
  `CMAKE_PROJECT_INCLUDE` 将 sidecar 作为 x64 独立目标接入上游 CMake；脚本不下载、
  不安装系统设备、不签名，并在输出中明确 `release_ready=false`；构建结果同时把
  `vcam_capi.dll` 放到 sidecar 同目录，满足 `LOAD_LIBRARY_SEARCH_APPLICATION_DIR`。
- `patches/0002-loopback-service-socket.patch` 在两个 Windows 构建脚本解包后应用，
  将上游 C API/Service 的 TCP 监听和连接限制到 `127.0.0.1`，避免原版
  `INADDR_ANY` 暴露到局域网。该上游协议仍未增加随机认证，因此不能把这一项误报为
  “当前用户 ACL + 随机认证”完整实现；正式发布前必须继续完成命名管道迁移或随机认证
  握手，并通过安全审计。
- `build-directshow.ps1` 只构建上游 `VirtualCamera_dshow` 目标，支持 `x86`/`x64` 两种
  MSVC 平台，不调用注册表或 `regsvr32`，并输出未签名的开发构建哈希。
- `installer/akvirtualcamera-components.nsh` 在卸载前要求用户退出 GpAutoLive 生产者和
  摄像头下游；用户取消会中止卸载，不强杀其他进程，确认后才移除本产品拥有的设备和
  DirectShow 注册。安装阶段在调用 `regsvr32` 前记录待回滚的 x86/x64 注册状态，
  即使注册工具返回错误或只完成部分注册，也会尝试注销残留。安装/卸载继续通过注册表
  所有者检查保护其他 AkVirtualCamera 实例。
- `run-test-pattern.ps1` 预计算 8 帧移动彩条 YUY2 环形样本，再按 30fps 发送，避免在发送
  循环内重复生成约 1.8 MiB 帧，同时让 CPU test-pattern 能验证下游收到的是变化帧。

本机已用 Visual Studio 2022 BuildTools/CMake 完成 x64 sidecar+C API 构建和 x86
DirectShow DLL 构建；结果保留在仓库发布边界之外的临时开发目录，未复制到发布资源，
也未安装系统设备。仍需经过 DirectShow 设备注册、ACL/边界测试、Authenticode 签名、GPL
对应源码和法务审核后，才能写入正式资源清单。

2026-09-10：`0008-directshow-yuy2-packed-stride.patch` 修正 DirectShow YUY2 行填充泄漏。
出口逐行复制有效像素，媒体类型、allocator 和实际 sample 长度统一为紧密排列大小；
保留上游内部 SIMD 对齐和其他格式。新增实际媒体类型及行 padding 毒值原生回归，
覆盖 1080×1920、1080×1919、1280×720 和 1920×1080。

2026-09-09：独立 sidecar 新增有界 v2 源尺寸帧（保持 v1 720p 兼容），并向 C API
传递真实宽高/行宽。显式开发模式仅修改已安装专用设备的第一个格式宽高；正式模式
不修改注册配置。开发安装器只为该格式键授予用户 SetValue，失败恢复旧宽高，
下游占用时拒绝重配；首次 C API 发送成功后才确认输出。新增纯帧格式和进程内注册表
替身测试。未修改上游算法、未取消正式签名要求、未将下游兼容门禁标为通过。

## 2026-09-10: DirectShow registration COM apartment

`patches/0003-directshow-com-apartment.patch` applies to pinned upstream 9cf77ae6379e5f635255f4b377478d388a46a3b2. Registration and removal reuse an already initialized STA on RPC_E_CHANGED_MODE and call CoUninitialize only for their own successful CoInitializeEx call. RegisterFilter failures propagate and the returned moniker is released. Both native build entry points apply and reverse-check the patch through the existing patch mechanism. No Media Foundation, device format, transport or signing policy changes.

The read-only `test-directshow-registration.ps1` compiles the actual patched registration/removal methods, replacing registry-mutating calls with injected HRESULTs. It verifies STA/MTA/uninitialized-thread COM ownership and RegisterFilter rejection. It does not register a device or establish downstream output readiness.

## 2026-09-10: DirectShow capture-pin category

`patches/0004-directshow-capture-pin-category.patch` adds the standard read-only IKsPropertySet interface to the existing output Pin. Get reports PIN_CATEGORY_CAPTURE and supports size queries, QuerySupported reports GET only, and Set returns E_NOTIMPL. Unsupported property sets/IDs and short/null buffers fail explicitly. This fixes consumers such as FFmpeg that require the category before considering an output pin; formats and frame transport are unchanged. `test-directshow-pin-category.ps1` compiles the actual methods for boundary regression without device writes.

## 2026-09-10: cancellable IPC and idle black

- `0005-ipc-cancel-deadline-wakeup.patch`: frameRequired releases the broadcasts mutex while waiting, copies its frame under the frame mutex, and is woken on stop. Each Windows broadcast slot supplies an atomic cancellation token to explicitly asynchronous MessageClient work. The existing Socket helpers use nonblocking readiness waits for controlled client connections, with cancellation checks at most 50ms apart and one 5-second deadline across each complete request/reply. Blocking server callers retain their existing call form and wire format; received payloads are capped at 64 MiB (above supported frame sizes). Stop cancels and joins before closing shared memory or erasing the slot. Destructor enumerates owned active slots rather than a potentially changed device preference list. No Assistant replacement, new user setting, or generic task framework is required.
- `0006-directshow-yuy2-idle-black.patch`: the existing DirectShow direct-mode YUY2 pin emits legal limited-range black when no current active frame is available, instead of random bytes. The GPU conversion and frame transport are unchanged. The dedicated native black-frame test covers the supported size cases and buffer guards.

The corresponding-source manifest includes both patches and their regression sources. Release and real end-to-end readiness remain separate from these source-level checks.

## 2026-09-10: Assistant dispatch and listener lifetime

`0007-service-listener-wakeup.patch` releases the MessageServer handler-registry mutex before invoking a copied handler, so a waiting listen call does not prevent a broadcast call from supplying its frame. Service listen records are deduplicated by client ID. Removing a broadcaster clears its retained frame and notifies waiting listeners of the inactive state. This is limited to the three reproduced same-path defects; the running Assistant must use the updated build for them to take effect. `test-ipc-service-wakeup.ps1` retains before-fail/after-pass coverage of all three.

Repeated-build correction: the two build entry points refresh only the fixed 0006-generated yuy2_black_frame.h before archive extraction, ensuring patch replay is consistent when reusing a build root. They do not recursively clean source directories or remove unknown files. The static build-entry regression verifies this ordering and restriction.
