# C15 AkVirtualCamera sidecar 协议实现记录

日期：2026-09-03

## 本轮落地

- `GpAutoLive.Windows/WindowsVirtualCameraSidecarProtocol.cs` 对齐 Rust/Tauri `sidecar_protocol.rs`：协议版本 1、`GPAKVC01` magic、52 字节固定帧头、YUY2 `1280×720×2` 固定 payload。
- `TryEncode` 接收调用方提供的 `Span<byte>`，在固定目标缓冲中写入 little-endian 帧头和 payload，不在编码路径创建第二份完整帧数组。
- `TryDecode` 只在完整固定帧已经到达后复制一个 payload，返回本次已消费字节数，保留后续流数据；短输入、错误 magic/版本/头长、规格、保留字段、时间戳、代际和序列都会 fail-closed。
- Named Pipe 只接受 `\\.\pipe\GpAutoLive-AkVirtualCamera-` 加 16 字节非零随机令牌的 32 位十六进制后缀；协议层不接受任意路径、不创建管道、不启动 sidecar。

## 本轮增量：Windows Named Pipe 传输客户端

- 新增 `GpAutoLive.Windows/WindowsVirtualCameraSidecarClient.cs`，只使用 .NET BCL `NamedPipeClientStream` 连接由上层/sidecar 创建的本机管道；连接、写帧、停止和释放均由单一 `SemaphoreSlim` 生命周期边界串行化。
- 客户端复用一个 `ArrayPool<byte>` 固定帧缓冲（约 1.84 MB），将 `VirtualCameraFrame.Timestamp90Khz` 按 Rust 参考实现的整数规则转换为 `Timestamp100Ns = timestamp90Khz × 1000 / 9`，超出 `Int64` 范围时 fail-closed；不把令牌、管道名或异常正文放进快照。
- 连接超时最多 30 秒，单帧写入默认 500ms；取消、断管道、写超时会关闭当前流并进入可重试失败态，停止操作幂等且归还池化缓冲。固定帧较大时由接收端并发读取，避免无界缓存和管道背压假成功。
- 客户端不启动进程、不读取 stdin token、不创建 Named Pipe ACL，也不执行 WGC/D3D11 捕获；sidecar 进程、当前用户 ACL、Job Object、WGC/D3D11 和 DirectShow 仍由后续真实链路负责。

## 验证增量

- 新增 5 项 Windows 测试：不可信管道名前置拒绝、未连接写入拒绝、本机 Named Pipe 固定帧 round-trip、时间戳溢出拒绝、停止/释放幂等。
- 全量自动化测试为 **356 项通过**（Contracts 25、Core 75、Media 91、Windows 132、App 33）；Release 构建 0 警告/0 错误，`dotnet format --verify-no-changes --no-restore` 通过。

## v66 发布复验

- 正式安装候选：`artifacts/csharp-windows-controller-20260903-v66`，根目录 8 个运行文件、1,285,863 bytes；`GpAutoLive.exe` 162,816 bytes。
- 独立符号包：`artifacts/csharp-windows-symbols-20260903-v66`，7 个文件、436,011 bytes；外置媒体运行时 13 个文件、352,365,694 bytes，资源清单 5/5 大小与 SHA-256 匹配。
- 使用仓库锁定 `.tools/dotnet` 设置 `DOTNET_ROOT` 启动/关闭成功，退出码 0、无 `GpAutoLive/mpv/ffmpeg/ffprobe` 残留；5 秒空闲基线 4 个样本，私有工作集峰值 85,348,352 bytes、工作集峰值 146,055,168 bytes、CPU 峰值 1.65%，GDI 17、User 42。
- 以上仅验证 C# 客户端发布和本机假 sidecar 回环；真实 sidecar、Named Pipe ACL、WGC/D3D11、DirectShow、签名/许可证、目标 GPU 和 30 分钟长稳仍待验收。

## 本轮增量：受控 sidecar 启动计划

- 新增 `WindowsVirtualCameraSidecarLaunchPlan` 与构造器，固定 sidecar 文件名为 `akvirtualcamera-sidecar-x64.exe`，仅接受绝对路径、普通文件和非 Reparse 父目录；文件大小上限 64 MiB，启动预算默认 5 秒并限制在 100 ms～30 秒。
- 启动参数只有 `--session-token-stdin`。16 字节非零会话令牌由内存中的请求传给受管宿主，经标准输入交付，不进入命令行、环境变量、普通 JSON/INI 或日志；管道名仍由既有协议层按令牌派生和校验。
- 该模块只生成可审查的启动描述（可执行路径、工作目录、固定参数和超时），不自行启动进程、不创建 ACL、不捕获 WGC/D3D11 帧；真实 sidecar 宿主接入必须继续经过 Job Object、签名、许可和下游兼容门禁。

## v68 发布复验

- 新增启动计划边界测试后，全量自动化测试为 **360 项通过**（Contracts 25、Core 75、Media 91、Windows 136、App 33）；Release 构建 0 警告/0 错误，格式检查和 `desktop/` 作用域检查通过。
- 正式安装候选：`artifacts/csharp-windows-controller-20260903-v68`，根目录 8 个运行文件、1,293,031 bytes；`GpAutoLive.exe` 162,816 bytes。
- 独立符号包：`artifacts/csharp-windows-symbols-20260903-v68`，7 个文件、436,679 bytes；外置媒体运行时 13 个文件、352,365,694 bytes，资源清单 5/5 大小与 SHA-256 匹配。
- 使用锁定 `.tools/dotnet` 设置 `DOTNET_ROOT` 启动/关闭成功，`CloseMainWindow=True`、退出码 0 且无 `GpAutoLive/mpv/ffmpeg/ffprobe` 残留；5 秒空闲基线 4 个样本，私有工作集峰值 99,823,616 bytes、工作集峰值 162,435,072 bytes、CPU 峰值 4.33%，GDI 5～17、User 21～40。
- 以上只证明 C# 启动计划和本机客户端发布边界可复验；真实 sidecar 运行、Named Pipe ACL、WGC/D3D11、DirectShow、签名/许可证、目标 GPU 和 30 分钟长稳仍待验收。

## v69 增量：真实 x64 PE 头部校验

- 启动计划在固定 `akvirtualcamera-sidecar-x64.exe` 文件名之外，读取有界 PE 头并要求 `MZ`、`PE\0\0`、`AMD64 (0x8664)` 与 `PE32+ (0x20b)`；文件名、大小或路径通过不再等同于架构通过。
- 新增合法最小 PE 夹具和 x86 架构拒绝测试；不会执行或加载该夹具，真实 sidecar 仍需独立签名和兼容门禁。
- 全量自动化测试为 **361 项通过**（Contracts 25、Core 75、Media 91、Windows 137、App 33）。
- 正式安装候选：`artifacts/csharp-windows-controller-20260903-v69`，根目录 8 个运行文件、1,293,543 bytes；`GpAutoLive.exe` 162,816 bytes。
- 独立符号包：`artifacts/csharp-windows-symbols-20260903-v69`，7 个文件、437,159 bytes；外置媒体运行时 13 个文件、352,365,694 bytes，资源清单 5/5 大小与 SHA-256 匹配。
- 使用锁定 `.tools/dotnet` 启动/关闭成功，`CloseMainWindow=True`、退出码 0 且无媒体进程残留；5 秒空闲基线 6 个样本，私有工作集峰值 100,352,000 bytes、工作集峰值 163,983,360 bytes、CPU 峰值 6.60%，GDI 17、User 40。

## v70 增量：受管 sidecar 宿主

- 新增 `WindowsVirtualCameraSidecarHost`，消费 x64 PE 校验通过的启动计划；隐藏启动 sidecar，将令牌编码为 32 个 ASCII 十六进制字符加换行后写入 stdin，并立即关闭 stdin。
- 宿主要求 Job Object 归属，限制 stdout/stderr 各 64 KiB，所有取消、停止、输出越界、读取异常和进程退出均通过稳定错误码投影；进程树在有限预算内终止并 Join。
- `ConnectClientAsync` 在不暴露管道名给 UI 的前提下复用内部会话管道；宿主不创建 ACL 管道、不执行 WGC/D3D11、不注册 DirectShow 设备，仍由真实 sidecar/安装链负责。
- 宿主边界测试加入后，全量自动化测试为 **364 项通过**（Contracts 25、Core 75、Media 91、Windows 140、App 33）。
- 正式安装候选：`artifacts/csharp-windows-controller-20260903-v70`，根目录 8 个运行文件、1,315,047 bytes；`GpAutoLive.exe` 162,816 bytes。
- 独立符号包：`artifacts/csharp-windows-symbols-20260903-v70`，7 个文件、442,215 bytes；外置媒体运行时 13 个文件、352,365,694 bytes，资源清单 5/5 大小与 SHA-256 匹配。
- 使用锁定 `.tools/dotnet` 启动/关闭成功，`CloseMainWindow=True`、退出码 0 且无媒体进程残留；5 秒空闲基线 6 个样本，私有工作集峰值 99,950,592 bytes、工作集峰值 163,012,608 bytes、CPU 峰值 5.13%，GDI 17、User 40～41。

## 明确边界

本轮仍未接入 WGC/D3D11 捕获、Named Pipe ACL 创建、AkVirtualCamera DirectShow 端点、GPL sidecar 启停或安装器注册。令牌不得进入命令行/普通配置；真实 sidecar 必须在独立进程、当前用户 ACL、Job Object、签名、许可证与下游兼容门禁全部完成后接入。

## v71 增量：资源包固定路径探测与宿主取消竞态加固

- 新增 `WindowsVirtualCameraSidecarLocator`，只读探测 `virtual-camera/bin/akvirtualcamera-sidecar-x64.exe`；默认使用应用目录，也可通过 `AUTOLIVE_AKVIRTUALCAMERA_ROOT` 指定安装根目录。探测结果区分 Windows 不适用、根目录无效、文件不存在和 sidecar 无效，不把“文件存在”当作可执行授权。
- 定位过程复用固定文件名、普通文件、非 Reparse 目录和 PE32+ AMD64 校验；宿主启动前仍二次校验，防止探测与执行之间的文件替换绕过边界。
- `WindowsVirtualCameraSidecarHost` 的输出读取回调改为安全取消，Stop/Dispose 与后台 drain 并发时不会因已释放 CTS 冒出未观察 `ObjectDisposedException`。
- 新增 5 项定位测试；全量自动化测试为 **369 项通过**（Contracts 25、Core 75、Media 91、Windows 145、App 33）。

## v71 发布复验

- 正式安装候选：`artifacts/csharp-windows-controller-20260903-v71`，根目录 8 个运行文件、1,318,631 bytes，`GpAutoLive.exe` 162,816 bytes；PDB/XML 独立 symbols 包 7 个文件、442,895 bytes；媒体运行时 13 个文件、352,365,694 bytes，继续使用资源清单校验和硬链接复用。
- 锁定 `.tools/dotnet` 的 Release 构建、全量测试和格式检查通过；启动关闭冒烟 `CloseMainWindow=True`、退出码 0 且无 `GpAutoLive/mpv/ffmpeg/ffprobe` 残留；5 秒空闲基线 7 个样本，私有工作集 84,815,872～100,261,888 bytes，工作集 141,918,208～163,397,632 bytes，CPU 峰值 5.49%，GDI 17，User 40～42。
- 真实 sidecar、当前用户 Named Pipe ACL、WGC/D3D11、DirectShow 注册/卸载、签名/许可证、目标 GPU/下游兼容和 30 分钟长稳仍待验收。

## v72 增量：D3D11 硬件前置探测

- 新增 `WindowsD3D11CapabilityProbe`，在 sidecar 已被定位器确认后才由 WPF 异步执行 `D3D11CreateDevice`；使用硬件 DriverType 和 BGRA 支持，拒绝 WARP，设备和 immediate context 在 `finally` 立即释放。
- 结果只返回脱敏分类和 Feature Level，不返回原始 HRESULT、设备句柄或异常正文；“前置通过”仍明确不代表 WGC、GPU 转换或虚拟摄像头设备已运行。
- WPF 初始卡片在 sidecar 缺失时不加载 D3D11；sidecar 存在时显示“正在检查/前置通过/前置未通过”，避免启动阶段无条件加载 GPU 驱动。
- 新增 1 项 Windows 探测测试；全量自动化测试为 **370 项通过**（Contracts 25、Core 75、Media 91、Windows 146、App 33）。

## v72 发布复验

- 正式安装候选：`artifacts/csharp-windows-controller-20260903-v72`，根目录 8 个运行文件、1,322,215 bytes，`GpAutoLive.exe` 162,816 bytes；PDB/XML 独立 symbols 包 7 个文件、443,711 bytes；媒体运行时 13 个文件、352,365,694 bytes，5/5 资源大小与 SHA-256 匹配。
- 锁定 `.tools/dotnet` Release 发布、启动关闭冒烟 `CloseMainWindow=True`、退出码 0 且无 `GpAutoLive/mpv/ffmpeg/ffprobe` 残留；5 秒空闲基线 7 个样本，私有工作集 85,098,496～100,839,424 bytes，工作集 142,184,448～163,123,200 bytes，CPU 峰值 4.78%，GDI 17，User 40～43。
- 真实 WGC/D3D11 纹理捕获、sidecar ACL/DirectShow、签名/许可证、多 GPU/Win10/11、下游兼容和 30 分钟长稳仍待验收。

## v73 增量：最终效果 HWND 绑定代际契约

- 新增 `WindowsVirtualCameraSurfaceBinding`，只维护最终效果视频表面的 HWND、generation 和关闭状态，不捕获像素、不创建 WGC 资源。
- 同一 HWND 重复绑定返回 `Unchanged`；HWND 变化、解除绑定或关闭都会递增 generation。后续 WGC/sidecar 回调必须在提交前通过 `IsCurrent`，避免旧窗口结果污染新会话。
- WPF 在最终效果窗口显示后绑定表面，在窗口关闭和应用退出时解除/释放；不创建第二窗口、第二播放器，也不把 HWND 绑定误报为设备已运行。
- 新增 3 项 Windows 绑定测试；全量自动化测试为 **373 项通过**（Contracts 25、Core 75、Media 91、Windows 149、App 33）。

## v73 发布复验

- 正式安装候选：`artifacts/csharp-windows-controller-20260903-v73`，根目录 8 个运行文件、1,327,847 bytes，`GpAutoLive.exe` 162,816 bytes；PDB/XML 独立 symbols 包 7 个文件、444,887 bytes；媒体运行时 13 个文件、352,365,694 bytes，5/5 资源大小与 SHA-256 匹配。
- 锁定 `.tools/dotnet` Release 发布、启动关闭冒烟 `CloseMainWindow=True`、退出码 0 且无媒体进程残留；5 秒空闲基线 7 个样本，私有工作集 84,451,328～100,298,752 bytes，工作集 141,713,408～163,991,552 bytes，CPU 峰值 5.26%，GDI 17，User 40～41。
- 真实 WGC、D3D11 转换、sidecar ACL/DirectShow、签名/许可证、Win10/11 与多 GPU、下游兼容和 30 分钟长稳仍待验收。

## v74 增量：真实 WGC HWND frame-pool 会话

- 新增 `WindowsGraphicsCaptureCapabilityProbe`，按 Windows 版本和 `GraphicsCaptureSession.IsSupported()` 做 fail-closed 前置探测；WPF 仅显示脱敏的 D3D11/WGC 前置状态。
- 新增 `WindowsGraphicsCaptureWindowSession`：在专用线程初始化 WinRT，通过 `IGraphicsCaptureItemInterop` 绑定最终效果 HWND，建立硬件 D3D11 设备、`Direct3D11CaptureFramePool` 和 `GraphicsCaptureSession`，观察内容尺寸/时间戳并校验绑定代际。
- 会话的启动、取消、停止、关闭和线程 Join 均有界；捕获线程不做 CPU 色彩转换、不复制完整帧、不启动 sidecar，也不创建第二播放器。无可见桌面合成时不伪造 `FrameCount`。
- 新增 3 项能力分类测试和 7 项会话测试（其中 1 项创建 Win32 窗口执行真实 frame-pool 启停）；全量自动化测试为 **382 项通过**（Contracts 25、Core 75、Media 91、Windows 156、App 35）。

## v74 发布复验

- 正式安装候选：`artifacts/csharp-windows-controller-20260903-v74`，根目录 8 个运行文件、1,348,437 bytes；`GpAutoLive.exe` 162,816 bytes。
- `runtime/winrt/` 独立放置 `Microsoft.Windows.SDK.NET.dll`、`WinRT.Runtime.dll` 和对应 `manifest.json`；2 个 DLL 共 27,848,816 bytes，manifest 记录版本、大小和 SHA-256。应用启动时由程序集白名单 resolver 按需加载，缺失时 fail-closed。PDB/XML symbols 包为 7 个文件、451,103 bytes；媒体运行时 13 个硬链接文件、352,365,694 bytes，资源清单 5/5 大小与 SHA-256 匹配。
- 使用锁定 `.tools/dotnet` 设置 `DOTNET_ROOT`/`DOTNET_ROOT_X64` 启动/关闭成功，`CloseMainWindow=True`、退出码 0 且无媒体进程残留；5 秒空闲基线 8 个样本，私有工作集峰值 85,962,752 bytes、工作集峰值 148,541,440 bytes、CPU 峰值 1.96%，GDI 17、User 40～41。
- WGC GPU→BGRA/YUY2 转换、固定规格三槽回读、当前用户 Named Pipe ACL、sidecar DirectShow、签名/许可证、目标 GPU/下游兼容、真实可见帧和 30 分钟长稳仍待验收。

## 验证

- v56 协议层历史验证：`dotnet test GpAutoLive.Windows.slnx -c Release --no-restore --logger "console;verbosity=minimal"` 为 Contracts 21、Core 64、Media 91、Windows 116、App 25，合计 **317 项通过**；当前客户端增量的最新全量结果见上方“验证增量”（356 项）。
- 协议测试覆盖固定管道名、单帧 round-trip/消费长度、截断拒绝、保留字段/身份拒绝和小目标缓冲拒绝；传输客户端测试覆盖本机 Named Pipe 背压下的并发读写和资源回收。

## v56 发布复验

- 正式安装候选：`artifacts/csharp-windows-controller-20260903-v56`，根目录 8 个文件、1,188,583 bytes；`GpAutoLive.exe` 162,816 bytes。
- 独立符号包：`artifacts/csharp-windows-symbols-20260903-v56`，7 个文件、396,477 bytes；外置媒体运行时复用 v54 的 13 个硬链接文件、352,365,694 bytes。
- 5 项媒体资源大小与 SHA-256 全部匹配；锁定 `.tools/dotnet` 启动/关闭冒烟退出码 0，无媒体进程残留。
- 5 秒空闲基线为 4 个样本，私有工作集峰值 84,017,152 bytes、工作集峰值 145,698,816 bytes、CPU 峰值 1.76%；该数据不等价于 30 分钟 GPU 长稳门禁。

## v75 增量：同设备 GPU→YUY2 输出边界

- `WindowsGraphicsCaptureWindowSession` 的同步回调现在携带同一 WGC frame pool 使用的硬件 D3D11 设备上下文；避免把 WGC 纹理跨设备复制，也不把软件适配器当作 GPU。
- `WindowsGraphicsCaptureGpuYuy2Converter` 使用 Vortice.Direct3D11/D3DCompiler 编译最小全屏三角形 shader，在 GPU 上完成 1280×720 缩放、BT.601 limited-range 转换和两个像素一组的 YUY2 打包。中间纹理为 640×720 RGBA，三槽 staging 以 `MapFlags.DoNotWait` 回读，未完成时直接返回 `Pending`。
- `WindowsVirtualCameraGpuOutputSession` 串接 HWND generation、WGC、GPU 转换和 `VirtualCameraOutputManager`；只有收到真实转换帧后才标记 `Ready`，sidecar 客户端仍保持独立可替换边界。
- `WindowsD3D11HardwareContextFactory` 通过 DXGI 读取真实 adapter LUID、厂商 ID、设备 ID、名称和 Feature Level，构造 `GpuCaptureFacts`，不允许猜测 GPU 事实。
- Vortice 运行库外置到 `runtime/gpu/` 并由 resolver 白名单按需加载；其版本、许可、哈希和大小必须与发布 manifest 一起校验。此举保留小 EXE，增加的运行时仅在启用 GPU 输出链时装载。

## v75 验证边界

- 新增 GPU 硬件上下文分类测试和常量 BGRA 纹理 GPU→YUY2 测试；该夹具实际检查 limited-range Y/U/V 字节和 1280×720×2 固定负载。
- v75 全量自动化测试为 **385 项通过**（Contracts 25、Core 75、Media 91、Windows 158、App 36）；Release 构建、格式检查和 `desktop/` 作用域检查通过。发布候选根目录 8 个运行文件、`1,379,568` bytes，`GpAutoLive.exe` `162,816` bytes；GPU DLL 7 个、`1,208,320` bytes，WinRT DLL 2 个、`27,848,816` bytes，均由独立 manifest 校验；symbols 包 7 个、`456,171` bytes。
- 锁定 `.tools/dotnet` 的发布包启动关闭冒烟 `WaitForInputIdle=True`、`CloseMainWindow=True`、退出码 0 且无媒体进程残留；4 秒空闲基线 6 个样本，私有工作集 `84,709,376～85,929,984` bytes，工作集 `141,668,352～146,436,096` bytes，CPU 峰值 `2.44%`，GDI 17，User `40～41`。
- 真实 WGC 可见帧、三槽长期回读、sidecar 当前用户 ACL、AkVirtualCamera DirectShow、签名/许可证、Win10/11 多 GPU、下游兼容和 30 分钟长稳仍需在目标设备上验收；无合成帧环境不宣称已完成真实摄像头输出。

## v76 增量：首帧状态顺序修正与发布复验

- `WindowsVirtualCameraGpuOutputSession` 在首次 GPU 回读后先验证真实 D3D11 adapter 事实并调用 `MarkReady`，再提交 `VirtualCameraFrame`；避免 `Starting` 状态下提前 `SubmitFrame` 被 Core 所有者拒绝，首帧路径保持 `Starting → Ready → latest-wins` 的单向顺序。
- 修正只影响状态提交顺序，不放宽 generation、固定负载、GPU 缩放/色彩转换、三槽 `DO_NOT_WAIT` 回读或 sidecar/DirectShow 独立边界。
- v76 全量自动化测试为 **385 项通过**（Contracts 25、Core 75、Media 91、Windows 158、App 36）；Release 构建和格式检查通过，`tools/verify-scope.ps1` 确认 `desktop/` 未修改。
- 正式候选 `artifacts/csharp-windows-controller-20260903-v76` 根目录 8 个运行文件、`1,382,640` bytes；GPU DLL 7 个、`1,208,320` bytes，WinRT DLL 2 个、`27,848,816` bytes；symbols 包 7 个、`457,979` bytes；媒体运行时 13 个硬链接文件、`352,365,694` bytes，三个 manifest 的大小/SHA-256 校验通过。
- 锁定 `.tools/dotnet` 启动关闭冒烟 `WaitForInputIdle=True`、`CloseMainWindow=True`、退出码 0 且无 `GpAutoLive/mpv/ffmpeg/ffprobe` 残留；4 秒空闲基线 6 个样本，私有工作集 `84,545,536～85,749,760` bytes，工作集 `141,676,544～146,472,960` bytes，CPU 峰值 `2.00%`，GDI `17`、User `40～41`。
- 真实 WGC 可见帧、sidecar 当前用户 ACL、AkVirtualCamera DirectShow、签名/许可证、Win10/11 多 GPU、下游兼容和 30 分钟长稳仍需目标设备验收；当前不将合成纹理夹具或 frame-pool 启停证据写成真实摄像头已可用。

## v77 增量：sidecar 固定 30fps 输出泵与完整组合生命周期

- 新增 `WindowsVirtualCameraSidecarOutputWriter`：复用一个固定大小黑帧缓冲，以 30fps 首帧立即发送，暂停/无效画面发送 YUY2 limited-range 黑帧，有效画面从 `VirtualCameraOutputManager` 取 latest-wins 帧；重复发送只创建小型帧记录，不复制 1,843,200 bytes payload。
- 新增 `WindowsVirtualCameraOutputCoordinator`：按 sidecar 宿主→Named Pipe 客户端→WGC/GPU 会话→输出泵顺序启动，按 writer→GPU→client→sidecar 顺序停止；使用生命周期信号量、取消和有限 Join，失败时回收已启动组件，不改变本地播放所有权。
- sidecar stdout 改为固定缓冲、64 KiB 总上限、4 KiB 行上限的 `GPAKVC_CLIENTS N` 解析；只把 0～1024 的客户端数量以脱敏事件交给组合层，其他日志丢弃，异常不能破坏进程树回收。
- 新增输出泵黑帧/latest-wins/连接门禁/停止幂等、组合安装门禁和状态行解析测试；v77 全量自动化测试为 **392 项通过**（Contracts 25、Core 75、Media 91、Windows 165、App 36）。
- v77 正式候选 `artifacts/csharp-windows-controller-20260903-v77` 根目录 8 个运行文件、`1,414,384` bytes；GPU DLL 7 个、`1,208,320` bytes，WinRT DLL 2 个、`27,848,816` bytes；symbols 包 7 个、`471,231` bytes；媒体运行时 13 个硬链接文件、`352,365,694` bytes；GPU/WinRT/媒体 manifest 大小与 SHA-256 校验通过。
- 锁定 `.tools/dotnet` 启动关闭冒烟 `WaitForInputIdle=True`、`CloseMainWindow=True`、退出码 0 且无残留；4 秒空闲基线 6 个样本，私有工作集 `84,504,576～85,823,488` bytes，工作集 `141,459,456～146,329,600` bytes，CPU 峰值 `1.72%`，GDI `17`、User `40～41`。
- 真实 sidecar 当前用户 ACL、DirectShow 注册/卸载、GPU→sidecar 实机连续帧、签名/许可证、Win10/11 多 GPU、下游兼容和 30 分钟长稳仍需目标设备验收；没有把本地 Named Pipe 回环测试写成真实设备证据。

## v78 增量：Windows 安装门禁探测与 WPF 刷新入口

- 新增 `WindowsVirtualCameraInstallationProbe`：只读检查 HKLM 64/32 位 `Webcamoid\\VirtualCamera` 安装所有者、固定 x64/x86 组件、`AkVCamManager.exe` 和当前存在的 `GpAutoLive Camera` PnP 设备；不调用安装器、`regsvr32`、Manager 或 sidecar，也不向 UI 返回路径/实例 ID。
- 只有双注册表视图路径一致、组件文件完整且 PnP 设备存在时才返回 `Available`；sidecar 文件探测和安装状态严格分开，避免把资源包存在误报成系统摄像头已安装。
- WPF 虚拟摄像头卡片新增“刷新探测”，启动和手动刷新均在后台合并安装、D3D11、WGC 结果，未通过门禁时不启动进程、不注册设备、不改变本地播放。
- 新增 2 项安装探测边界测试；v78 全量自动化测试为 **394 项通过**（Contracts 25、Core 75、Media 91、Windows 167、App 36）。
- v78 正式候选 `artifacts/csharp-windows-controller-20260903-v78` 根目录 8 个运行文件、`1,421,040` bytes；GPU DLL 7 个、`1,208,320` bytes，WinRT DLL 2 个、`27,848,816` bytes；symbols 包 7 个、`471,879` bytes；媒体运行时 13 个硬链接文件、`352,365,694` bytes；三个 manifest 的大小/SHA-256 校验通过。
- 锁定 `.tools/dotnet` 启动关闭冒烟 `WaitForInputIdle=True`、窗口标题 `GpAutoLive`、`CloseMainWindow=True`、退出码 0 且无残留；4 秒空闲基线 6 个样本，私有工作集 `85,704,704～86,974,464` bytes，工作集 `143,773,696～147,521,536` bytes，CPU 峰值 `1.47%`，GDI `17`、User `40～41`。
- 真实 DirectShow 双位数安装、签名/许可证、当前用户 Named Pipe ACL、GPU→sidecar 连续帧、WGC 可见帧、多 GPU/Win10/11、下游兼容和 30 分钟长稳仍需目标设备验收。

## v79 增量：WPF 输出启停接线与媒体生命周期收敛

- `MainWindow` 虚拟摄像头卡片新增启动/停止/刷新动作。启动前联合检查安装双视图、固定组件/PnP 设备、sidecar、D3D11、WGC 与最终效果 HWND；任何门禁失败都保持禁用，不自动安装、注册或启动外部组件。
- 启停统一通过播放命令串行器进入 `WindowsVirtualCameraOutputCoordinator`，保持 sidecar host、Named Pipe、WGC/GPU、30fps writer 的可取消启动/逆序停止；窗口关闭和媒体池导入/拖放/替换/删除/清空均先停止正在运行的摄像头输出，再解除 HWND generation 或提交新媒体池。
- 新增 `WindowsVirtualCameraInstallationProbe` 只读检查 HKLM 双视图、固定 x86/x64 文件与 `GpAutoLive Camera` PnP 设备；安装探测结果与 sidecar 文件探测分离，仍不返回路径、实例 ID 或原始系统错误。
- v79 候选 `artifacts/csharp-windows-controller-20260903-v79` 根目录 8 个文件、`1,428,208` bytes，EXE `162,816` bytes；GPU DLL 7 个 `1,208,320` bytes，WinRT DLL 2 个 `27,848,816` bytes；symbols 7 个 `473,235` bytes；媒体运行时 13 个硬链接文件、逻辑大小 `352,365,694` bytes，manifest 大小/SHA-256 全部匹配。
- 全量自动化测试 **394 项通过**；Release 构建 0 警告/0 错误，格式和 `desktop/` 作用域检查通过。发布包锁定 `.tools/dotnet` 启动关闭冒烟通过（`WaitForInputIdle=True`、`CloseMainWindow=True`、退出码 0、无残留）；4 秒空闲基线 6 个样本，私有工作集 `85,475,328～86,773,760` bytes、工作集 `142,733,312～147,476,480` bytes、CPU 峰值 `2.74%`。
- 真实 sidecar ACL、GPU→sidecar 连续帧、WGC 可见帧、DirectShow 安装/卸载、签名/许可证、多 GPU、下游兼容和 30 分钟长稳仍待目标设备验收；本轮仅完成 WPF 编排与生命周期门禁接线。
