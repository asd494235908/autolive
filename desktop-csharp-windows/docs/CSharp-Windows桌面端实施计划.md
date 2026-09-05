# GpAutoLive C# Windows 桌面端实施计划

> 当前同步长任务的最新状态以 [`CSharp-Windows当前状态.md`](./CSharp-Windows当前状态.md) 和 [`CSharp与Rust桌面端功能同步长任务实施方案`](../../docs/superpowers/plans/2026-09-04-CSharp与Rust桌面端功能同步长任务实施方案.md) 为准。本文件较早的 Phase C3/C4 段落保留历史实施快照；2026-09-04 后，视频处理关闭统一走 Original，开启时按完整 shader 哈希选择 GPU83/CPU4，首次播放与动态更新共用系统生成快照。

> 实施进度（2026-09-03）：Phase C0/C1 已完成；Phase C2 已完成严格控制面合同、纯逻辑会话门禁、原子 INI/JSON 配置、Windows Credential Manager/DPAPI 边界、有界 `HttpClient` 传输层、冷启动 Refresh Token 恢复、单任务后台心跳和 WPF 登录/自动激活/Refresh/Logout 编排（默认未配置服务端时不发网络请求）；Phase C3 已接入纯逻辑媒体池、逐项有界导入/FFprobe 协调器、FFprobe 命令/输出边界、Windows `Process` 适配器、最小 Job Object 增强边界、手动导航、FileDrop 拖放、唯一最终效果窗口和“已验证资源→HWND→受管 mpv”播放控制器接线；Phase C4 已接入 mpv 单会话 JSON IPC、Windows 命名管道传输边界、外置媒体资源清单完整性校验、受限视频参数快照、活动源身份门禁、固定容量 PCM 环缓、三属性播放状态有界监视器、PortAudio 独立 DLL 设备枚举、预分配回调输出流和输入流边界、最终 PCM 双消费者总线及本地能量 VAD 门控边界，以及 Windows 受管 mpv 宿主和“进程→管道→会话”组合生命周期；Phase C5 已接入固定话术合同、单操作状态机、Windows SAPI STA/COM 适配边界、音频优先级/静音策略边界、RTMP 配置/参数合同、Windows FFmpeg 宿主生命周期和本机 H.264 编码器逐候选探测边界；当前纯音频 FFmpeg→环缓→PortAudio 播放/暂停/恢复/停止已接入 WPF，有限音轨支持有界排空，RTMP 画面及最终 PCM 声音会话已接入 WPF 开始/停止控制（真实 ZLMediaKit/RTMPS 网络和断线重试仍未验收）。v76 在 v75 基础上保留同一 WGC D3D11 设备上的 GPU→YUY2 转换器，并修正首次 GPU 回读时先 `MarkReady` 后 `SubmitFrame` 的状态顺序：GPU 完成固定 1280×720 缩放和 BT.601 limited-range 打包，三槽 staging + `DO_NOT_WAIT` 有界回读，并接入 `WindowsVirtualCameraGpuOutputSession` 的 generation/latest-wins 逻辑边界；Vortice 运行库按程序集白名单外置到 `runtime/gpu/`，避免主 EXE/安装根目录继续膨胀。v75 及更早版本保留为历史发布记录；本轮新增抖音 M1 WPF 本地配置/生命周期入口、非敏感配置版本化 JSON 原子存储、Windows Conda/上游文件/环境名/超时/PNG 输出校验的 sidecar 启动计划，以及 sidecar 脱敏事件解析、固定缓冲有界 stdout 读取、受管进程树生命周期和核心状态桥接，并补齐 Windows GDI/User GUI 资源计数与 Gen0/Gen1/Gen2 GC 计数快照；新增仅使用 BCL 的 AkVirtualCamera Named Pipe 固定帧传输客户端、固定 sidecar 启动计划、x64 PE32+ 架构校验、受管 sidecar 宿主、虚拟摄像头资源包固定路径/文件探测、D3D11 硬件前置探测、最终效果窗口 HWND 绑定代际契约和真实 WGC HWND frame-pool 会话边界。已有插话声音配置 JSON、22 个预设白名单、随机/周期选择器、固定缓冲 attack/release 过渡、AkVirtualCamera 固定规格/GPU 真实性/状态与 latest-wins 帧契约、固定 52 字节 sidecar 帧协议与会话管道名校验，以及抖音 M1 本地合同、扫码状态机、有界回复队列和脱敏状态投影继续保留。PortAudio 原生健康探测、回调健康快照和最多 3 次有界设备恢复、麦克风门控优先级和解码输出门控、插话文件目录快照与 PCM 混音、虚拟摄像头 WGC/D3D11/sidecar、抖音 sidecar 真实运行仍保持“代码已接入·待真实设备/发布门禁”；22 套预设真实 DSP、真实声卡听感、AEC/降噪/AGC/完整 VAD、真实设备拔插/睡眠唤醒、目标 GPU、控制面联调、SAPI 语音包实机、真实 RTMP/网络、AkVirtualCamera、抖音 QR/协议兼容、Rust/Tauri 复刻运行时和外部 sidecar 仍未全部验收。

> v82 还加入 `WindowsAuthenticodeProbe` 发布门禁探测；它只在 Windows 上对绝对普通文件调用 `WinVerifyTrust`，开发包的 `Unsigned` 结果保持待签名，不改变本地启动路径。

> v83 将 Authenticode 结果接入 AkVirtualCamera sidecar 探测与 WPF 启动门禁：未签名或无效签名的 sidecar 可以被诊断，但不会启用输出按钮或启动 sidecar。

> C7 安装骨架已新增 `install-csharp-windows-package.ps1`：发布包先核验，再写入版本目录并原子更新 `current.json`；重复版本、运行中进程和路径/重解析点异常均 fail-closed。

> C7 回滚/卸载边界已新增 `rollback-csharp-windows-package.ps1` 与 `uninstall-csharp-windows-package.ps1`：回滚只切换已验证的非活动版本指针，卸载只允许移除非活动版本并拒绝重解析点；两者均支持 `-WhatIf`，不触碰用户配置和 `desktop/`。

> C7 Runtime bootstrapper 已新增 `bootstrap-csharp-windows-runtime.ps1`：先探测指定主版本 `Microsoft.WindowsDesktop.App`，在线下载仅允许 HTTPS 官方 .NET 域名并要求 SHA-256/Authenticode，安装需显式 `-Install`，不把 Runtime 打进主 EXE。

> C7 代码签名流水线已新增 `sign-csharp-windows-package.ps1`：要求证书存储区私钥、固定 RFC3161 时间戳站点和 Windows SDK `signtool`，签名后重算外置运行库 Manifest 并执行 `-RequireSigned` 复核。

> C7 最小 WPF 安装维护壳已新增独立 `GpAutoLive.Setup.exe`：只通过固定白名单调用现有 Runtime 探测、发布包核验、安装/升级、状态核验、回滚和非活动版本删除脚本；真实变更先强制 `-WhatIf` 并二次确认，完成或失败后重新核验安装状态。当前仍是“.NET 10 Desktop Runtime + PowerShell 7 已就绪”环境下的维护壳，不冒充裸机在线 Bootstrapper 或完整卸载器。

> 日期：2026-09-03  
> 状态：实施中（C0/C1/C2 已完成，C3/C4/C5 核心边界已部分接入，v90 安装候选已生成）  
> 最新候选：v90 在 v89 基础上同步 C6 有界 UI 状态合并；麦克风与抖音后台快照采用 latest-wins 调度，C7 安装/回滚/卸载共用事务互斥，媒体与外置运行库布局保持不变。
> 目标目录：`desktop-csharp-windows/`  
> 参考实现：`desktop/`（严格只读，不在本计划中修改）

> 最新进度看板：[`CSharp-Windows当前状态`](./CSharp-Windows当前状态.md)。

> 状态覆盖：上方历史叙述中若出现 v83/v82/v81/v80/v79/v78/v77/v76/v75/v74/v73，仅表示此前发布记录；当前实施与验收口径以版本化安装事务、安装状态只读核验、发布启动/关闭冒烟、发布包只读核验、Runtime bootstrapper、代码签名流水线、WPF 安装维护壳、回滚/卸载边界、v83 sidecar 签名启动门禁、v82 Authenticode 探测、全局媒体/输出资源所有权门禁、AppUserModelID/安装门禁刷新、输出启停接线、WPF 媒体池职责拆分、C6 UI 状态更新合并、C7 安装事务锁审查记录和本文件第 110 节为准。

当前增量补充：固定话术已从 Windows SAPI 边界接入 WPF 工作台，RTMP 已建立配置/参数/Windows 宿主边界并接入画面/最终 PCM 声音会话的开始/停止按钮，画面推流启动前已按固定顺序完成本机 H.264 一帧探测；PortAudio 已接入独立 DLL 枚举、WPF 输入/输出设备刷新、固定容量 PCM 环缓消费的长期输出流和显式输入流边界，最终 PCM 已建立本机输出/RTMP 双消费者总线与有界分流泵；音频优先级、PCM 混音、FFmpeg 音频解码、单一音频播放控制器和 N/N+1 预载调度边界已接入，WPF 纯音频播放/暂停/恢复/停止已接线，并已接入有限单项会话的 EOF 顺序推进；PortAudio 原生健康探测、250ms 观察器和最多 3 次有界重开已接入；WPF 设置窗口现已通过原子 INI 接入性能采样、快速参数卡展开和下次输出模式记忆，并保留 Ctrl+, 快捷键；视频 `time-pos` 已投影到进度条与当前位置/时长，并为视频源接入鼠标释放及方向键/Home/End 的受身份保护绝对 seek；麦克风已接入显式启用的本地能量门控、共享优先级和固定话术抢占通知；基础媒体 PCM 已在解码线程分片写入总线前按共享优先级执行静音/6 dB duck。真实音频候选、麦克风 AEC/降噪/AGC/完整 VAD/可听混音、设备拔插/睡眠唤醒、GPU 编码器矩阵、ZLMediaKit/RTMPS 网络和断线重试仍按 C5 门禁待验收。

## 1. 结论

新桌面端采用 **C# 14 + .NET 10 LTS + WPF**，仅支持 Windows 10/11 x64。它按现有 Rust/Tauri/React 桌面端的用户功能、状态机、错误语义和安全边界进行行为复刻，但不修改、链接或运行时依赖现有 `desktop/` 源码。

选择 WPF 而不是 WinUI 3 的主要原因不是视觉偏好，而是本项目当前更重视：

1. 去除 WebView2/React 常驻内存与前端轮询开销；
2. 使用成熟的 Windows 桌面窗口、HWND、键盘、DPI 和自动化能力；
3. 采用 framework-dependent 发布，让主 EXE 保持很小；
4. 以较低迁移风险承载高密度参数工作台和原生 mpv 子窗口。

微软把 WinUI 3列为新 Windows 应用的推荐框架，同时把 WPF定位为成熟的 .NET 桌面框架。本项目优先选择 WPF，是基于现有 Win32/mpv/WGC/DirectShow 集成、部署体积和长期稳定性的具体取舍，不把“更新”直接等同于“更适合”。参考：[Windows 应用开发框架概览](https://learn.microsoft.com/en-us/windows/apps/)、[WPF 启动性能](https://learn.microsoft.com/en-us/dotnet/desktop/wpf/advanced/application-startup-time)。

## 2. UI 设计基准与显示语义

当前实施以两张连续 v2 设计稿为正式 UI 基准，尺寸基准均为 `1586×992`：

1. 首屏工作台：[`gpautolive-csharp-windows-gui-v2-main-20260903.png`](./assets/gpautolive-csharp-windows-gui-v2-main-20260903.png)
2. 下滑续页：[`gpautolive-csharp-windows-gui-v2-scroll-20260903.png`](./assets/gpautolive-csharp-windows-gui-v2-scroll-20260903.png)

旧版 [`gpautolive-csharp-windows-gui-preview-20260902.png`](./assets/gpautolive-csharp-windows-gui-preview-20260902.png) 保留为历史对照证据，不再作为参数编辑区的实施基准。两张 v2 设计稿是授权态主工作台的视觉和信息架构基准，不代表当前代码已经全部实现。

### 2.1 首屏工作台

- 顶栏：GpAutoLive、设备授权、账号、授权到期、CPU/GPU/内存、设置和窗口控制。
- 操作带：主页/工作区、添加媒体、整理媒体、导入列表、保存配置、加载配置、开始推流和打开效果窗口。
- 左栏：媒体池搜索、媒体条目、类型/时长/尺寸或音频信息，以及“播放设置（本地有序循环）”卡片。
- 中栏上半区：实时预览、单一视频表面提示、适配窗口/16:9/静音预览/全屏、当前媒体、时间进度和播放状态。
- 中栏下半区：参数、播放队列、快捷键、日志四个 Tab；首屏显示“当前视频周期参数”只读快照、周期/轮次/GPU83/源素材摘要和滚动位置提示。
- 右栏：按 RTMP 推流、虚拟摄像头、麦克风插话、固定话术、抖音弹幕顺序显示五张输出/能力卡片，并投影各自真实状态。
- 底栏：播放、暂停、停止、上一条、下一条、时间轴、当前媒体、独立的视频处理/声音处理开关和音量。

### 2.2 下滑续页

下滑时保持顶栏、底栏以及左右工作区上下文稳定，主要滚动中央参数内容；中央滚动条进入较低位置，显示以下连续区域：

- 普通声音周期参数：系统随机生成的声音效果只读快照、周期/轮次/预设池/本周期预设、重新生成本周期参数。
- 可调整规则：仅允许调整预设池和周期范围等规则，不允许直接改写本周期的增益、动态范围、低频、高频、混响、压缩、降噪和音色结果。
- 高级视觉：视觉频段 `65Hz–20000Hz`、微扰状态、GPU83、失败回退 CPU4、当前源素材和实时频谱只读展示。
- 播放与设备状态：当前媒体、视频/声音处理、mpv 单窗口、PortAudio 声音总线和 RTMP 直推状态。
- 事件日志和说明卡：只显示有界、脱敏、可行动的状态摘要，不显示原始进程输出、完整地址或敏感凭据。

### 2.3 参数编辑边界

- 视频和普通声音的周期结果均由系统按周期随机/自动生成，UI 使用“自动生成 / 只读快照 / 已生效”状态徽标和文本卡片表达，不使用可拖动滑杆、可编辑数值框或伪装成禁用控件的结果编辑器。
- 用户可以编辑规则、范围、预设池、输入设备、输出地址等配置；这些配置不能与当前周期生成结果混为一谈。
- “重新生成本周期参数”是一个受控动作：提交当前规则后重新生成并刷新只读快照，不能让调用方传入任意效果值。
- “已生效”只有在真实播放/输出链路确认快照已经应用后才能显示；仅有模型、默认值或 UI 占位时必须显示“待接入/未生效”。
- 登录页不在这两张授权态工作台设计稿覆盖范围内，继续沿用独立登录门禁和现有登录逻辑验收口径；不得把工作台设计稿当作登录页的 1:1 基准。

预览区仍只能承载唯一视频表面：当最终效果独立窗口打开时，中部切换为状态摘要或已确认缩略图，禁止同时创建第二个播放器或第二个实时渲染表面。

界面保持三栏工作台：

- 左栏：本地有序媒体池、导入、原子编辑与条目状态；
- 中栏：单一视频表面/状态摘要、参数、播放队列、快捷键和日志；
- 右栏：RTMP、虚拟摄像头、麦克风插话、固定话术和抖音弹幕；
- 底栏：播放、暂停、停止、上一项、下一项、时间轴、视频/声音处理开关和输出音量；
- 顶栏：账号授权、CPU/GPU/内存、设置和窗口控制。

## 3. 本次功能边界

### 3.1 目标

- 在新目录内独立实现 Windows C# 桌面端；现有 `desktop/` 永久保持参考只读。
- 用户可见行为与现有桌面端一致：同一输入得到同一状态转换、错误类别和恢复结果。
- 保留一个活动媒体源、一个最终效果窗口、一个受管 mpv 视频会话、一个普通声音 Worker 的上限。
- 保留 GPU83 → CPU4 → Original 单向降级、独立视频/声音周期、PortAudio、插话、固定话术、RTMP、虚拟摄像头和抖音 M1 的真实能力口径。
- 优化启动、空闲内存、长时间运行内存、UI 响应和安装布局。
- 主 EXE 小型化，但不以把所有内容塞进单文件为目标；总体安装体积和运行内存分别治理。

### 3.2 非目标

- 不修改 `desktop/` 下任何 Rust、Tauri、React、配置、测试或资源文件。
- 不同时维护 macOS 版本，不引入 MAUI、Avalonia 或跨平台抽象层。
- 不在第一阶段重写编解码器、滤镜、RTMP 协议或虚拟摄像头驱动。
- 不恢复实时话术幻化、旧变体 MP4、旧报告 Worker、OBS、多平台分发或检测规避能力。
- 不为了“DLL 数量好看”把每个页面、控件或 DTO 拆成独立程序集。
- 不用 NativeAOT、激进 Trim、ReadyToRun 或单文件发布作为第一版优化手段；它们会增加兼容风险或体积，必须在功能复刻完成后用数据决定。

## 4. 现状体积证据

2026-09-02 对当前正式 Windows 包的只读检查结果：

| 项目 | 当前体积 |
| --- | ---: |
| 主程序 `autolive-desktop-core.exe` | 20.66 MiB |
| NSIS 安装 EXE | 357.44 MiB |
| portable ZIP | 143.63 MiB |
| `mpv.exe` | 114.21 MiB |
| `ffmpeg.exe` | 108.70 MiB |
| `ffprobe.exe` | 108.50 MiB |
| `d3dcompiler_43.dll` | 4.27 MiB |
| 媒体运行资源合计 | 335.69 MiB |

结论：GUI EXE 与媒体运行资源是两个问题。C# framework-dependent 发布可以把入口 EXE 压到很小并减少 WebView2 内存，但安装包的主要体积来自 mpv/FFmpeg/FFprobe，必须通过运行资源包独立治理，不能把“EXE 变小”冒充“整个产品变小”。

## 5. 目标技术栈

| 层 | 选择 | 约束 |
| --- | --- | --- |
| GUI | WPF / XAML | 只使用 Windows 桌面，不加载 WebView2 |
| 语言与运行时 | C# 14 / .NET 10 LTS | x64，framework-dependent 默认发布 |
| MVVM | CommunityToolkit.Mvvm | 只使用 `ObservableObject`、`RelayCommand`、`AsyncRelayCommand` 等实际需要能力 |
| JSON | `System.Text.Json` | DTO、版本化配置、资源清单、IPC；不引入第二套 JSON 库 |
| INI | `Microsoft.Extensions.Configuration.Ini` | 仅低敏感、简单键值配置 |
| HTTP | `HttpClient` | 单例/工厂化复用，统一超时、取消、重试边界 |
| 本机 IPC | `System.IO.Pipes` + 长度前缀 JSON | mpv 保持其 JSON IPC；其他 sidecar 使用版本化协议 |
| 媒体 | 现有成熟 mpv、FFmpeg、FFprobe、PortAudio 能力语义 | 独立发布资源，不嵌入主 EXE |
| Windows 原生 | Win32 Job Object、Credential Manager、WGC/D3D11 | 全部集中在 Windows 程序集，禁止散落 P/Invoke |
| 固定话术 | Windows 本地语音能力 | 必须保持本地声音、取消、抢占和恢复语义；不调用模型 |

依赖在实施前必须逐项记录版本、许可证、调用点、替代方案和移除路径。能用 .NET 标准库或 Windows 原生能力解决的，不新增第三方包。

## 6. 解决方案与 DLL 拆分

主客户端只保留 5 个生产程序集；C7 另有一个不引用主客户端任何程序集的独立安装维护壳。测试按边界拆分为 6 个项目，页面按功能文件夹组织，不按页面拆 DLL：

```text
desktop-csharp-windows/
  GpAutoLive.Windows.slnx
  Directory.Build.props
  global.json
  src/
    GpAutoLive.App/                 # WPF 入口、窗口、页面、ViewModel；生成小型 apphost EXE
      Features/
        Auth/
        Playback/
        Effects/
        Outputs/
        Interaction/
        Settings/
    GpAutoLive.Core/                # 状态机、用例、不可变快照、错误语义
    GpAutoLive.Contracts/           # Go API、本机 DTO、JSON schema 生成物边界
    GpAutoLive.Media/               # mpv/FFmpeg/FFprobe/PortAudio 调度与媒体生命周期
    GpAutoLive.Windows/             # HWND、Job Object、凭据、WGC/D3D11、DPI、单实例
    GpAutoLive.Installer/           # 独立 WPF 安装维护壳；只编排固定脚本，不引用主客户端程序集
  tests/
    GpAutoLive.Core.Tests/
    GpAutoLive.Media.Tests/
    GpAutoLive.App.Tests/
    GpAutoLive.Contracts.Tests/
    GpAutoLive.Windows.Tests/
    GpAutoLive.Installer.Tests/
  config/
  contracts/
  runtime/
  packaging/
  docs/
```

发布文件职责：

| 文件 | 职责 | 是否按需加载 |
| --- | --- | --- |
| `GpAutoLive.exe` | 小型 .NET apphost，只负责启动 | 否 |
| `GpAutoLive.App.dll` | XAML/BAML、View、ViewModel 和组合根 | 否 |
| `GpAutoLive.Core.dll` | 业务状态机和用例 | 否 |
| `GpAutoLive.Contracts.dll` | 版本化 DTO 和错误码 | 否 |
| `GpAutoLive.Media.dll` | 媒体管理器，首次进入工作台后加载 | 是 |
| `GpAutoLive.Windows.dll` | Windows 集成，按功能首次使用加载 | 部分 |
| `GpAutoLive.Setup.exe` / `GpAutoLive.Setup.dll` | C7 独立安装维护入口与固定脚本编排；不随主 GUI 加载 | 独立发行 |

程序集拆分的目的只是稳定依赖方向和避免主 EXE 膨胀；.NET 程序集被加载后不会因文件拆开就自动省内存。因此真正的内存优化依赖延迟创建 View、列表虚拟化、事件合并和资源生命周期，而不是继续增加 DLL 数量。

依赖方向固定为：

```text
App -> Core -> Contracts
App -> Media -> Core/Contracts
App -> Windows -> Media/Core/Contracts
Installer -> bundled tools/*.ps1（无主客户端 ProjectReference）
Contracts 不反向依赖任何层
```

禁止 `Core` 依赖 WPF、Win32、具体页面、mpv 或 HTTP 实现。

## 7. JSON、INI 与运行文件布局

```text
安装目录/
  GpAutoLive.exe
  GpAutoLive.*.dll
  config/defaults.ini                 # 只读安全默认值
  contracts/ipc-v1.schema.json        # 唯一本机消息契约
  runtime-resources.json              # 版本、大小、SHA-256、许可证索引
  runtime/media/<version>/bin/...     # mpv/ffmpeg/ffprobe/共享运行库
  runtime/media/<version>/licenses/...
  runtime/winrt/*.dll + manifest.json # WinRT 投影按需运行库，不进根目录
  runtime/gpu/*.dll + manifest.json   # D3D11/Vortice 按需运行库，不进根目录
  runtime/virtual-camera/<version>/...
  runtime/douyin/<version>/...

用户数据目录/
  config/app.ini                      # 窗口、主题、非敏感偏好
  profiles/media/<id>.json            # 媒体参数配置，带 schema_version
  profiles/interlude/<id>.json        # 插话配置，带 schema_version
  outbox/heartbeat.json               # 最多一条待补发心跳
  logs/<date>.jsonl                   # 脱敏、有上限、滚动日志
  cache/audio-sessions/...            # 受管临时文件
```

### 7.1 INI 只保存简单低敏感偏好

允许：窗口位置、窗口大小、主题、语言、最近选择的非敏感选项、参数面板展开状态、性能采样开关。

禁止：密码、Refresh Token、设备 Token、Cookie、完整带凭据 RTMP URL、媒体正文、播放池路径和当前播放位置。敏感凭据只进入 Windows Credential Manager；播放池继续只存在当前进程。

### 7.2 JSON 保存结构化且需要版本迁移的数据

- 所有文件包含 `schema_version`；未知未来版本 fail-closed。
- 写入使用同目录临时文件、Flush、原子替换；不得直接覆盖唯一副本。
- 每类文件有独立大小、数量和字段上限。
- 运行时高频状态只保存在内存，不每帧或每个 tick 写 JSON。
- JSON Schema 是跨进程/跨语言消息的唯一事实源；生成代码与手写代码分离。

### 7.3 DLL 与原生资源

- 不把 mpv、FFmpeg、FFprobe、PortAudio、shader、虚拟摄像头或 Python sidecar 嵌入 `GpAutoLive.exe`。
- 原生 DLL 从经过哈希校验的绝对目录加载，不依赖当前工作目录和不安全 PATH 搜索。
- WinRT 投影运行库 `Microsoft.Windows.SDK.NET.dll`/`WinRT.Runtime.dll` 由白名单 resolver 从 `runtime/winrt/` 按需加载，旁边的 `manifest.json` 固定版本、大小和 SHA-256；发布门禁核对清单，运行时缺失或加载失败时保持 fail-closed。
- GPU 运行库 `Vortice.Direct3D11`、`Vortice.DirectX`、`Vortice.DXGI`、`Vortice.D3DCompiler`、`Vortice.Mathematics`、`SharpGen.Runtime*` 由同一 resolver 白名单从 `runtime/gpu/` 按需加载；不静态合并进主 EXE，发布 manifest 固定版本、大小、SHA-256 和许可证入口，缺失时 GPU 输出链 fail-closed。
- PDB、测试程序集、基准工具和诊断符号进入独立 symbols 包，不进入正式安装目录。
- 可选能力使用独立资源包，但缺少必需资源时必须显示“未安装/不可用”，不得伪成功。

## 8. GUI 信息架构与便利度

### 8.1 不错乱原则

- `AppStateStore` 是 C# GUI 的单一投影源；媒体真实状态由对应 Manager 所有，ViewModel 不自行推算成功。
- 所有命令携带 `request_id`，播放相关命令同时携带 `playback_generation/source_revision/loop_index`；迟到响应不能覆盖新状态。
- 配置草稿、已保存配置、已应用配置、真实运行状态分开显示。
- “已接入 / 正式需求待实现 / 待确认”继续是能力事实，不由是否显示控件决定。
- 视频处理和声音处理保持独立开关、独立计划、独立计数，不创建联动目标。
- 参数修改采用显式应用或既有周期规则；禁止滑块拖动时每个像素都发送媒体命令。

### 8.2 操作便利度

- `Ctrl+O` 导入媒体，`Space` 播放/暂停，`Ctrl+Shift+S` 停止，`F11` 全屏最终效果，`Ctrl+,` 设置；文本输入聚焦时不抢快捷键。
- 播放池支持拖放、键盘上移/下移、删除确认和原子整批替换；失败保留旧池。
- 右栏输出卡只展示当前状态与主操作，详细设置进入对话框/抽屉式面板。
- 参数用分类导航、搜索和折叠；默认只显示常用项，所有正式参数仍可到达且状态不丢失。
- 全部字段有可见标签、单位、范围和就地错误；图标按钮提供可访问名称和 Tooltip。
- 支持 100%～250% DPI、960×680 最小窗口、键盘完整访问、可见焦点、高对比度和减少动画。
- 动画仅使用 120～180ms 的透明度/位移状态过渡；播放和监控页禁用装饰性循环动画。

为保持现有界面验收口径，首版窗口仍以 `1280×800` 启动、最小 `960×680`；顶栏保持 `36px`。在 `1728×1044` 基准视口下三栏为 `320px / minmax(0, 1fr) / 272px`，外边距和栏间距均为 `12px`；`≤1199px` 时按素材 → 视频/参数 → 声音/输出的键盘顺序降级，不能靠横向裁切隐藏功能。

## 9. Windows 性能设计

### 9.1 启动

- 首帧只创建 Shell、登录门禁和最小状态；媒体面板、输出面板、日志和设备枚举延迟到实际进入或首次展开。
- 启动路径不访问网络、不扫描媒体目录、不枚举全部声音设备、不启动 mpv/FFmpeg。
- 启动阶段不加载 `GpAutoLive.Media.dll` 中的重型类型；用 ETW/PerfView 验证实际程序集加载。
- 不默认启用 ReadyToRun，因为它常以更大文件换启动时间；完成基线后再做 A/B。

### 9.2 内存

- 播放池 `ListView` 启用 UI 虚拟化和容器回收；不要把它放入提供无限高度的外层 `ScrollViewer`。WPF 官方文档明确指出 UI 虚拟化和 Recycling 能降低列表项创建与内存成本：[WPF 控件性能](https://learn.microsoft.com/en-us/dotnet/desktop/wpf/advanced/optimizing-performance-controls)。
- 参数页按当前分类延迟创建，不一次实例化全部卡片；切页后释放订阅、计时器、图像和大缓冲。
- 缩略图解码到显示尺寸，限制缓存数量和总字节；不把原图或整段媒体读入内存。
- 高频状态通过容量 `1` 的 latest-wins 通道合并；日志、弹幕、诊断列表均有条目数和字节上限。
- 音频环形缓冲使用固定容量；禁止无限 `ObservableCollection`、无限 Channel 和全量历史快照。
- 事件订阅、窗口事件、CancellationTokenSource、Timer、Stream 和进程句柄必须对称释放。

### 9.3 UI 线程

- 文件探测、哈希、HTTP、JSON 大对象解析、设备枚举和进程等待全部异步且可取消，不在 Dispatcher 上执行。
- UI 状态推送按字段合并，普通状态最高 10Hz，资源指标最高 2Hz，日志视图最高 5Hz；错误和用户命令结果即时送达。
- 只更新变化的 ViewModel 属性，不周期性替换整棵对象树。
- 复杂参数卡不使用模糊、实时阴影、透明视频叠层和大面积渐变；WPF 布局成本随视觉树与布局重算增加，需保持扁平模板：[WPF 布局性能](https://learn.microsoft.com/en-us/dotnet/desktop/wpf/advanced/optimizing-performance-layout-and-design)。

### 9.4 Windows 原生媒体与进程

- mpv 继续以单一受管进程和持久 JSON IPC 工作，视频窗口使用受控 HWND；不因参数周期重启。
- FFmpeg/FFprobe 使用参数数组、隐藏窗口、超时、取消、stderr 尾缓冲和 Job Object；禁止 shell 拼接。
- WPF 只做 UI 合成；GPU83 继续由 mpv/libplacebo，WGC/D3D11 继续用于虚拟摄像头链，不在 C# UI 层复制逐帧像素。
- 退出顺序固定为：关闭新任务门禁 → 取消计划 → 停止输出 → 回收子进程树 → Join 读取任务 → 释放窗口/音频/管道句柄 → 退出。

## 10. 发布与小 EXE 策略

### 10.1 默认发布

采用 `win-x64` framework-dependent、非单文件、非 Trim、非 ReadyToRun：

- `GpAutoLive.exe` 只是小型 apphost；业务代码、XAML 和契约位于 DLL。
- 目标机需要 .NET 10 Desktop Runtime；小型在线安装器检查并从微软官方渠道安装。
- 同时保留完整离线安装包，供无网络环境使用；离线包大不等于主 EXE 大。
- .NET 支持 framework-dependent 与 self-contained 等发布模式；单文件会把托管 DLL 合并进 EXE，与“小 EXE、合理拆分”目标相反：[.NET 单文件部署](https://learn.microsoft.com/en-us/dotnet/core/deploying/single-file/overview)。

### 10.2 媒体资源包

第一版不改变现有媒体二进制身份，只把它们从 GUI 发布物中逻辑分离：

- `media-core`：mpv、FFmpeg、FFprobe、D3DCompiler、shader 和法律材料；
- `virtual-camera`：AkVirtualCamera、x86/x64 组件、sidecar、源码/补丁/许可证索引；
- `douyin-m1`：受管 sidecar 与锁定依赖；
- 每个包包含版本、SHA-256、大小、可执行位和许可证索引，安装采用 staging + 原子切换；
- C# 本地 staging 入口为 `tools/stage-media-runtime.ps1`，只读消费显式来源目录并把资源放到 `runtime/media/<version>/`；PortAudio 通过可选 `-PortAudioRoot` 单独放入资源包，不会修改 `desktop/`；
- 在线轻量安装器按需获取，离线完整安装器一次携带；功能页面只依据已验证资源状态启用。

后续体积优化先验证“共享 FFmpeg 运行库 + 小型 ffmpeg/ffprobe 前端 + 与 mpv 的 ABI 锁定”候选。只有功能、许可证、启动、GPU、30 分钟稳定性和跨机矩阵全部通过，且媒体包至少缩小 25%，才允许替换当前静态资源；否则保持现有成熟二进制。

## 11. 性能与体积验收预算

所有指标先记录 Rust/Tauri/React 基线，再比较同一机器、同一媒体、同一功能组合下的 C# 结果。目标不是无依据的绝对承诺。

| 指标 | C# 目标门禁 |
| --- | --- |
| 主 `GpAutoLive.exe` | ≤ 1 MiB |
| GUI 托管程序集总量 | ≤ 15 MiB，不含 .NET Runtime、媒体包和符号 |
| GUI 发布目录 | ≤ 25 MiB，不含 .NET Runtime、媒体包和用户数据 |
| 冷启动至可交互 | ≤ 2.5s，且不劣于现有基线 |
| 热启动至可交互 | ≤ 1.2s |
| 登录页空闲私有工作集 | ≤ 120 MiB，且较现有下降 ≥ 35% |
| 工作台空闲私有工作集 | ≤ 180 MiB，且较现有下降 ≥ 30% |
| 30 分钟无播放内存增长 | ≤ 20 MiB，GC 后回落稳定 |
| 30 分钟联合播放 GUI 内存增长 | ≤ 40 MiB，不包含 mpv/FFmpeg/sidecar 进程 |
| 空闲 CPU | P95 < 1%（同一测试机） |
| 普通交互输入到呈现 | P95 ≤ 50ms，无 >200ms 的可重复卡顿 |
| 播放池 100 项滚动 | 60Hz 屏幕无持续掉帧，容器保持虚拟化 |
| 状态队列 | 全部有界；满载时按既定 latest-wins/丢最旧策略 |

测量工具：Windows Performance Recorder/Analyzer、PerfView、dotnet-counters、Process Explorer、GPUView，以及现有媒体门禁脚本。报告必须记录 OS、CPU、GPU、内存、DPI、构建配置、素材和采样时间。

## 12. 功能复刻清单

以下项目必须逐项以“未开始 / 代码已接入 / 自动化通过 / 实机通过”标记，不能用页面存在替代功能验收：

1. 登录、Refresh、退出、账号偏好、Credential Manager、设备身份与激活门禁；
2. 心跳、离线播放、重连补发、用户/设备禁用；
3. 视频和纯音频统一播放池、1～100 项、选择/拖放、整批探测、原子 CRUD；
4. 单窗口、单活动源、顺序循环、单项自循环、seek、暂停、继续、停止、EOF 幂等；
5. mpv 原生表面、GPU83、CPU4、Original、同进程换源、PTS/FPS/EOF/健康门禁；
6. 视频/声音两个独立处理开关、参数三态、独立周期和真实变化次数；
7. 普通声音候选、PortAudio、设备选择、环缓、A/B 切换、恢复和音画时钟；
8. 插话文件递归池、22 套声音预设、多轨混合、duck、随机周期；
9. 麦克风插话、AEC/降噪/AGC/VAD、抢占、恢复和真实设备门禁；
10. 固定话术本地系统朗读、1～500 字、取消、静音/恢复，不调用模型；
11. 唯一最终效果窗口、普通/全屏、关闭重开、错误只回主窗口；
12. RTMP/RTMPS 音画选择、GPU/编码器降级、重试、脱敏与取消；
13. AkVirtualCamera 固定规格、WGC/D3D11、黑帧、latest-wins、安装/卸载与许可门禁；
14. 抖音 M1 扫码、单直播间、`WebcastChatMessage`、本地回复池、有界队列、自回显和风控停发；
15. 本地配置迁移、资源清单、缓存清理、结构化日志、统一退出和单实例。

导入白名单保持一致：视频为 `.mp4/.mov/.mkv/.avi/.webm/.m4v/.ts/.m2ts/.flv/.wmv/.3gp`，纯音频为 `.mp3/.wav/.m4a/.aac/.ogg/.flac`。扩展名只用于选择过滤，最终以 FFprobe 的真实流、时长和可读性探测为准。

声音优先级固定为 `麦克风插话 > 固定话术 > 插话文件 > 普通声音/原媒体`。麦克风开始说话时停止或数字静音低优先级声音；麦克风结束后不自动续播已经被截断的插话文件或固定话术，避免重复内容。

### 12.1 当前成熟度不得因 C# 复刻抬升

| 能力 | C# 初始展示口径 |
| --- | --- |
| 播放池、登录、基础播放、参数界面 | 依据逐项对照结果标记，不从旧页面存在直接推断已通过 |
| GPU83 | 当前开发可达能力与未完成参数保持原状态；不得写成 83/83 全部生效 |
| 麦克风插话 | 已接入 WPF 显式启停、受管 PortAudio 输入流、本地能量门控和共享优先级抢占通知；保持“代码已接入·待真实设备验收”，待 AEC/降噪/AGC/完整 VAD、可听混音与长稳门禁 |
| RTMP/RTMPS | 已接入配置校验、脱敏合同、FFmpeg 参数计划、Windows 宿主、H.264 本机逐候选探测及 WPF 画面/声音开始/停止控制；保持“代码已接入·待验收”，真实网络/最终时序未通过前不宣称可推流 |
| AkVirtualCamera | 已接入固定规格/GPU 事实与状态、52 字节 sidecar 协议、BCL Named Pipe 传输客户端、x64 PE 校验和受管 sidecar 宿主；保持“代码已接入·待真实设备/发布门禁”，仍需签名、注册、ACL、DirectShow 和兼容矩阵 |
| 抖音弹幕 M1 | 当前只允许按真实探针/接入状态展示；不得因 C# 有面板就标记可用 |
| 固定话术本地系统朗读 | 已接入 `GpAutoLive.Windows` SAPI 适配边界；保持“代码已接入·待验收”，未通过 Windows 10/11 语音包、取消延迟和音频设备门禁前不得标记可用 |
| 实时话术幻化、旧变体/报告、OBS | 不属于当前版本，不创建入口 |

最终效果窗口只能消费播放快照和必要媒体控制，不获得登录、HTTP、Credential Manager、任意文件选择、虚拟摄像头安装/修复等权限。主窗口与各 Manager 的权限边界必须通过测试锁定。

## 13. 分阶段实施计划

### Phase C0：冻结参考基线

- 给 `desktop/` 建立只读审计清单：功能、命令、DTO、错误码、状态机、测试、资源和截图。
- 在同一 Windows 机器记录启动、私有工作集、CPU、句柄、线程、GDI/USER 对象和 30 分钟趋势。
- 输出功能复刻矩阵和“禁止修改 desktop/”的 CI 路径门禁。

退出：基线数据可重复，差异矩阵有唯一编号；没有 C# 功能代码。

### Phase C1：最小 WPF 壳与发布骨架

- 安装并锁定 .NET 10 SDK，创建上述 5 个生产项目和 5 个测试项目。
- 完成自定义标题栏、登录壳、主题 token、DPI、键盘焦点、单实例和全局异常边界。
- 建立 framework-dependent 发布、轻量在线安装器和空离线包骨架。

退出：空壳 EXE、程序集和启动内存达到预算；没有媒体依赖。

### Phase C2：契约、配置与控制面

- 固定 JSON Schema/OpenAPI 生成边界，接入 `System.Text.Json` 严格校验。
- 实现 INI/JSON 原子读写、版本迁移、Credential Manager、登录/冷启动恢复/Refresh/Logout、激活与单任务后台心跳。
- 完成离线、禁用、重试、幂等和敏感信息日志测试。

退出：控制面功能与现有桌面端对照通过；所有外部输入有上限。

### Phase C3：播放池与唯一窗口

- 实现 FFprobe 有界探测、统一媒体池、原子 CRUD 和播放状态机。
- 建立唯一最终效果 WPF 窗口与 mpv HWND 宿主；完成播放、seek、EOF、换源、关窗重开和退出。
- 复刻 generation/revision/loop 身份，迟到回调 fail-closed。

当前实现状态（2026-09-02）：播放池纯逻辑所有者已接入 `GpAutoLive.Core`，完成 17 种 Windows 媒体扩展名过滤、最多 100 项、32 KiB UTF-8 路径边界、规范化路径、探测 DTO 元数据边界校验、媒体类别一致性校验、原子替换/追加/单项替换/重排/上移/下移/删除/清空，以及 `Ready/Playing/Paused/Stopped` 与 `playback_generation/source_revision/source_media_index/loop_index` 身份门禁。手动上一项/下一项通过带四维身份的 `SelectAt`、`Previous`、`Next` 纯逻辑入口完成：切源递增 generation、更新 source_media_index、重置 loop_index，source_revision、播放池回绕计数和播放状态保持稳定，迟到身份 fail-closed。`GpAutoLive.Media` 已接入 FFprobe 固定参数、路径/扩展名校验、超时/取消/输出上限、JSON 根对象校验、JSON 到 `SourceMediaDto` 的纯逻辑解析，以及 `MediaImportCoordinator` 的逐项串接：整批成功后才原子 `ReplaceAll/Append/ReplaceAt`，探测失败、重复路径、超限或取消均保留旧快照；`GpAutoLive.App` 已接入 WPF 原生多选文件框、固定安装目录媒体清单校验和已验证 FFprobe 组合根，并提供选中项上移/下移/移除/清空操作。媒体池拖放使用 Windows `FileDrop` 保留系统顺序后按 `Append` 送入同一协调器，移除/清空使用原生确认框，`Delete` 键复用选中项移除路径。`GpAutoLive.Core.Processes` 已承载通用进程合同，`GpAutoLive.Windows.WindowsExternalProcessRunner` 已接入参数数组、双流有界异步读取、超时/取消和 `Process.Kill(entireProcessTree:true)` 清理，并在 Windows 上增加 Job Object 创建/分配/终止增强边界；Job Object 不可用时保留原有受限 Process 清理回退。`WindowsMpvProcessHost` 已把经过 `MpvLaunchPlan` 校验的 mpv 进程接入隐藏启动、Job Object 优先回收、退出事件和有界停止；`WindowsMpvPlaybackRuntime` 再将进程宿主、命名管道客户端和会话身份串成单一组合生命周期。`GpAutoLive.Media` 不再引用 `GpAutoLive.Windows`，依赖方向无循环。`GpAutoLive.App` 已接入单一最终效果窗口、脱敏快照、播放控制事件和 Windows HWND 预留。当前仍未完成发布资源包/签名、真实 mpv/PortAudio、seek/EOF/换源实机流程和视频/纯音频混排验收；因此 C3 整体继续标记为“代码已接入·待实施/未验收”。

退出：视频/纯音频混排、100 项、单/多文件循环与现有行为一致。

### Phase C4：视频与普通声音

- 复刻持久 mpv JSON IPC、GPU83、CPU4/Original、参数快照和健康降级。
- 复刻声音候选、PortAudio、N/N+1、PCM 环缓、设备切换与音画纠偏。
- 参数页面按分类懒加载并完成全量正式参数三态。

当前实现状态（2026-09-02）：`GpAutoLive.Media` 已接入固定 mpv JSON IPC 命令/响应契约、Windows 命名管道客户端、会话—传输组合网关、响应字段与 `request_id` 校验、属性白名单、有限 GPU83/CPU4 参数快照、单一活动视频源以及 generation/revision/index/loop 迟到响应门禁；`MpvLaunchPlan` 已把经过资源清单验证的 `mpv.exe`、视频路径、宿主 HWND、命名管道和三种视频模式收敛成有界不可变启动参数；`AudioPcmRingBuffer` 已接入固定容量交错 PCM 环缓，满载丢弃旧帧、关闭排空和状态快照均有边界测试。另已接入版本化外置媒体资源清单、固定资源白名单、安装目录锚定、大小/SHA-256 校验和已验证 FFprobe 创建入口；`WindowsMpvProcessHost` 已实现 Windows 隐藏启动、Job Object 优先回收、退出事件、有界取消/停止；`WindowsMpvPlaybackRuntime` 已将进程、命名管道和会话身份按“启动→连接→停止”顺序组合；`WindowsPortAudioDeviceEnumerator` 已建立 PortAudio DLL 动态加载和设备枚举边界，`WindowsPortAudioOutputStream` 已建立固定容量 PCM 环缓消费、预分配回调、启动/停止/关闭与欠载统计边界；`FfmpegPcmDecodePlanBuilder`、`WindowsFfmpegPcmDecoder` 与 `WindowsAudioPlaybackController` 已建立首条音频轨道解码到本机输出的单一可取消组合边界。当前命名管道、启动计划、宿主生命周期、PCM 环缓、资源校验、Job Object、PortAudio 输出流和音频控制器仍是受测边界；真实 mpv 资源/首帧/EOF/HWND 绑定、GPU83/CPU4/Original 生效、健康降级、PTS/FPS、PortAudio 音频总线/设备恢复和 30 分钟实机验收仍待实施；WPF 纯音频播放按钮已接入，但真实设备恢复与音画时钟仍待验收。详见 [`C4 mpv 边界实现记录`](./C4-mpv边界实现记录.md)、[`C4 mpv 命名管道运行时边界`](./C4-mpv命名管道运行时边界.md)、[`C4 音频环缓边界`](./C4-音频环缓边界.md)、[`C4 PortAudio 输出流实现记录`](./C4-PortAudio输出流实现记录.md)、[`C4 FFmpeg PCM 解码边界`](./C4-FFmpeg%20PCM解码边界实现记录.md)、[`C4 Windows 音频播放控制器`](./C4-Windows音频播放控制器实现记录.md) 与 [`C4-运行资源清单边界`](./C4-运行资源清单边界.md)。

补充：`AudioCyclePrewarmCoordinator` 已建立 N/N+1 预载的纯逻辑边界；它只产生 `Prepare/Commit/Expire` 动作，不启动解码、不拥有 PCM 队列。`AudioPcmTrackSwitchOutputSource` 进一步提供单一输出源内的固定双槽边界：只有显式提交且 N 关闭并排空后才切换到 N+1，临时欠载不会触发切换；该边界尚未接入 Windows 播放控制器或 RTMP 分流，真实候选预载仍保持“代码已接入·待实施”。详见 [`C4 N/N+1 预载接入审查记录`](./C4-NN1预载接入审查记录.md)。

### Phase C5：插话与输出能力

- 插话文件、麦克风插话、固定话术；再接 RTMP、虚拟摄像头和抖音 M1。
- 每项沿用已有安全、许可、队列、超时、取消、黑帧和发布门禁。
- 未完成真实门禁的能力继续显示实际状态，不因迁移抬升。

退出：15 类功能复刻矩阵无缺项；所有 sidecar 可取消、Join 并由 Job Object 兜底。

### Phase C6：性能与资源包优化

- 用基线工具定位启动、布局、分配、GC、句柄和 IPC 热点；只修真实热点。
- 完成列表虚拟化、状态合并、缩略图缓存上限、日志上限和长稳泄漏修复。
- 对共享媒体运行库候选做独立体积/ABI/许可证/稳定性实验，达不到门禁则拒绝替换。

退出：第 11 节全部指标有原始报告；无“感觉更快”的结论。

### Phase C7：并行发布与切换

- C# 与现有桌面端使用不同安装目录、AppUserModelID、用户配置版本和进程互斥名。
- 先内部灰度；同一设备同一时刻只允许一个客户端拥有媒体和输出资源。
- C# 连续通过功能、升级/卸载、签名、Windows 10/11、GPU/声卡和 30 分钟长稳门禁后，才讨论默认切换。
- 回滚只切换启动入口，不修改或删除现有 `desktop/`。

退出：用户确认切换，且仍保留可验证回滚路径。

## 14. 验证命令规划

实现后至少执行：

```powershell
dotnet restore desktop-csharp-windows/GpAutoLive.Windows.slnx --locked-mode
dotnet format desktop-csharp-windows/GpAutoLive.Windows.slnx --verify-no-changes
dotnet build desktop-csharp-windows/GpAutoLive.Windows.slnx -c Release --no-restore
dotnet test desktop-csharp-windows/GpAutoLive.Windows.slnx -c Release --no-build
dotnet publish desktop-csharp-windows/src/GpAutoLive.App/GpAutoLive.App.csproj -c Release -r win-x64 --self-contained false
tools/verify-release-package.ps1 -PackageRoot <candidate-root> -SymbolsRoot <symbols-root>
```

另需建立：

- 路径门禁：C# 任务的 diff 不得包含 `desktop/`；
- 发布门禁：主 EXE、程序集、媒体资源、符号和许可证清单逐项校验；
- UI 自动化：键盘、DPI、焦点、窗口恢复、单实例、错误态；
- 媒体实机：现有 mpv、PortAudio、RTMP、虚拟摄像头和抖音专项矩阵；
- 性能门禁：冷/热启动、内存、CPU、句柄、线程、GC、30 分钟趋势。

## 15. 风险与决策门

| 风险 | 处理 |
| --- | --- |
| 仅换 GUI，媒体资源仍大 | GUI 与媒体包分开度量，后续共享运行库必须独立验收 |
| C# 重写状态机导致错序 | 以 generation/revision/loop 和黑盒对照测试锁定行为 |
| WPF 视觉树过重 | 参数懒加载、扁平模板、虚拟化、更新合并、ETW 验证 |
| framework-dependent 缺少运行时 | 在线 bootstrapper + 完整离线包两种发行物 |
| DLL 拆太多反而增加加载成本 | 生产程序集固定 5 个，新增必须证明真实边界 |
| 单文件/Trim 破坏 XAML 或反射 | 第一版禁用，功能稳定后单独实验 |
| 固定话术脱离 WebView2 后行为变化 | Windows 本地语音专项对照，未通过前不切换 |
| 原生窗口/虚拟摄像头兼容问题 | HWND、WGC、D3D11、DirectShow 全部实机矩阵验证 |
| 双客户端争抢设备或输出 | 不同互斥名 + 全局媒体所有权锁 + 灰度期间单端运行 |

## 16. 开始编码前门禁

1. 用户确认本计划与预览图方向；
2. 安装 .NET 10 SDK（当前机器只有 .NET 8.0.19 Runtime，没有 SDK）；
3. 确认最低 Windows 版本与首批 GPU/声卡测试矩阵；
4. 确认在线轻量安装器与离线完整安装器均保留；
5. 建立 `desktop/` 路径只读 CI 门禁；
6. 先完成 Phase C0 基线，再创建 C# 解决方案。

本计划不授权删除现有 Rust/Tauri/React 文件，也不执行未使用代码清理。若后续在新 C# 目录发现未使用代码，只报告清单，获得用户明确确认后再删除。

## 17. 当前实施增量：C3 最终效果窗口边界（2026-09-02）

本增量实现 C3 的核心边界，不宣称真实媒体播放已接入：

- `GpAutoLive.Contracts/MediaPoolRules.cs` 与 `GpAutoLive.Core/MediaPoolService.cs`：17 种扩展名、100 项上限、路径/文件/元数据校验、原子播放池 CRUD、手动 `SelectAt`/上一项/下一项、循环状态和四维迟到回调门禁。
- `GpAutoLive.Core/Processes/ExternalProcessContracts.cs`：平台无关的进程计划、结果状态、启动策略和执行器接口，避免 Media 与 Windows 互相引用。
- `GpAutoLive.Media/MediaProbeBoundary.cs`、`FfprobeCommandBuilder.cs`：FFprobe 参数数组、路径边界、超时/取消/输出上限和根 JSON 校验；只消费抽象执行器。
- `GpAutoLive.Windows/ExternalProcessBoundary.cs` 与 `GpAutoLive.Windows/WindowsJobObject.cs`：Windows `Process` 适配器使用 `ArgumentList`、隐藏无 Shell 启动、stdout/stderr 字节上限、超时/取消和 `Kill(entireProcessTree:true)`；Job Object 仅作为创建/分配成功时的 Windows 增强边界，失败时安全回退，不宣称真实 mpv 已接入。
- `GpAutoLive.Media/MpvPlaybackIpcGateway.cs`：把 `MpvPlaybackSession` 身份门禁与 `MpvNamedPipeClient` 串成单一发送入口；只分配 request_id、发送固定命令并拒绝切源后的迟到响应，不启动 mpv。
- `GpAutoLive.Windows/ControlPlaneHttpClient.cs`、`WindowsDeviceIdentity.cs`、`ControlPlaneAuthCoordinator.cs`、`ControlPlaneHeartbeatScheduler.cs`：控制面 HTTP 传输只使用 `HttpClient` 和 Contracts DTO，固定桌面路径、请求 ID、Bearer/幂等头、HTTPS/loopback 边界、超时/取消和有界严格响应解析；设备标识与 Refresh Token 进入 Credential Manager，冷启动恢复先 Refresh 再激活，WPF 登录/自动激活/Refresh/Logout/心跳已由串行认证所有者编排，后台心跳单任务运行并把暂态失败限制为一条无凭据 outbox。真实控制面、TLS、Credential Manager 权限和 Windows 长稳仍待实施验收。
- `GpAutoLive.Media/MpvLaunchPlan.cs`：将已校验的 mpv 资源、媒体、HWND、IPC 端点和 GPU83/CPU4/Original 模式编译成有界参数数组；不启动进程，不接受任意命令行。
- `GpAutoLive.Windows/WindowsMpvProcessHost.cs`：只消费不可变启动计划，使用隐藏 Windows 进程、退出事件、Job Object 优先和 `Process.Kill` 兜底完成有界宿主生命周期；启动失败和立即退出均清理 PID/句柄快照。
- `GpAutoLive.Windows/WindowsMpvPlaybackRuntime.cs`：把进程宿主、同一 IPC 端点和 `MpvPlaybackSession` 组合成启动、连接、固定命令分发和停止顺序；关闭时在有界预算内发送 `quit`，然后释放管道和进程树。
- `GpAutoLive.Media/AudioPcmRingBuffer.cs`：固定容量交错 PCM 环缓，满载丢弃旧帧、关闭后排空，不枚举设备或启动 PortAudio。
- `GpAutoLive.App/Features/Playback/FinalEffectProjection.cs`：单一控制器状态、脱敏播放快照、最小播放控制事件，以及关闭后可重新打开的状态恢复。
- `FinalEffectWindow.xaml`：独立最终效果窗口；不接收路径、凭据、HTTP 信息、文件选择权限或媒体处理参数。
- `ReservedVideoSurface.cs`：Windows 子 HWND 预留，目前仅创建黑色 `STATIC` 表面，不启动 mpv/FFmpeg；纯音频使用独立黑色 WPF 表面，不挂载视频封面。
- `MainWindow`：增加“打开最终效果/关闭窗口”显式入口；打开不会自动播放或启动媒体进程，主窗口关闭时对窗口和事件订阅做对称清理。
- `GpAutoLive.App.Tests/FinalEffectWindowControllerTests.cs`：覆盖单实例投影、纯音频表面、关闭重开、控制事件门禁和脱敏字段边界。

当前 C3 待实施：真实 FFprobe/mpv 资源签名与发布注入、真实首帧/EOF/seek/换源回调和视频/纯音频混排实机验收。`WindowsMpvPlaybackController` 已由 WPF 播放入口调用，只有资源清单完成校验且视频表面取得有效 HWND 时才启动 mpv；缺少资源、HWND 或 IPC 失败均保持播放池快照并显示可重试错误。Windows 宿主组合边界已接入，但仍需随真实媒体进程完成嵌套 Job、权限、首帧和孤儿进程矩阵验收。

## 18. 当前实施增量：已验证资源到 WPF 视频入口（2026-09-02）

本增量把前面已完成的边界接成一条可验收的 Windows 调用链，但不把“代码接线”写成“真实媒体已验收”：

- `GpAutoLive.Windows/WindowsMpvPlaybackController.cs` 是播放所有者，串行保护 `MediaPlaybackIdentity`、`MpvPlaybackSession`、`WindowsMpvPlaybackRuntime` 和启动/停止/暂停/seek/换源操作；错误只返回稳定分类和脱敏文本，不把路径、命令行或 IPC 原文交给 WPF。
- `FinalEffectWindow.xaml.cs` 暴露经 `ReservedVideoSurface` 创建的子 HWND；`MainWindow` 只在视频项播放时按“打开最终效果窗口→取得 HWND→校验外置资源→构造固定 `MpvLaunchPlan`→启动并连接命名管道”的顺序调用控制器。主 EXE 不内嵌媒体二进制。
- `FinalEffectWindow` 已提供单窗口无边框全屏切换：主窗口或最终效果窗口按 `F11` 进入/退出，最终效果窗口在全屏时按 `Esc` 退出；不会创建第二个窗口或渲染表面。
- 视频播放/暂停/停止和上一项/下一项入口已改为异步，控制器失败时不会强行把失败操作写入成功状态；播放池在异步启动期间发生身份变化时，旧会话会被关闭并 fail-closed。
- 纯音频现在必须经过已验证 FFmpeg/PortAudio 资源和用户选择的输出设备，播放/暂停/恢复/停止会先操作真实会话再提交播放池；没有设备或资源时仍显示不可用并 fail-closed。
- `GpAutoLive.App/Features/Settings/DesktopPreferencesCoordinator.cs` 已把低敏感窗口尺寸/位置偏好接到用户数据目录的原子 INI；配置损坏回退默认值，路径和文件内容不进入日志或媒体状态。

本增量的历史验收门禁是：`dotnet format --verify-no-changes`、Release 构建、五个测试项目共 156 个测试、`verify-scope.ps1`；v8 复验结果为五个测试项目合计 172 项通过，framework-dependent 发布根目录 15 个文件、1,038,499 bytes，`GpAutoLive.exe` 162,816 bytes。外置媒体二进制继续独立放在 `runtime/media/1.0.0/bin`，4 项资源合计 351,999,432 bytes（含清单和法律材料的 staging 树 352,053,564 bytes）。`mpv --version` 与 `ffprobe -version` 已在本机启动验证；真实 mpv 首帧、EOF、HWND 渲染、GPU83/Original 生效和音频设备仍需在 Windows 目标机上单独验收。

v8 启动冒烟在设置 `DOTNET_ROOT`/`DOTNET_ROOT_X64` 指向本地 .NET 10 运行时后保持窗口运行约 1.8 秒，通过 `CloseMainWindow` 正常退出，标题为 `GpAutoLive`、退出码 0；采样私有字节 78,163,968 bytes（约 74.5 MiB）、工作集 135,925,760 bytes（约 129.6 MiB）、22 个线程、1,079 个句柄，退出后无残留 `GpAutoLive/mpv/ffmpeg/ffprobe` 进程。framework-dependent 包在未安装 .NET 10 Desktop Runtime 的机器上不能直接启动；该数据包含 .NET/WPF 运行时，不作为 C6 冷启动性能基线。

## 19. 当前实施增量：C6 最小 Windows 性能采样边界（2026-09-02）

本增量只建立后续基线工具所需的可注入采样边界，不宣称性能指标已达标：

- `GpAutoLive.Windows/WindowsProcessPerformanceSampler.cs` 使用 Windows `Process` 读取当前进程的私有工作集、工作集、线程数、句柄数和可选 CPU 计数；快照不包含 PID、路径、命令行、用户名、主机名、媒体或凭据。
- `WindowsProcessPerformanceSampler` 每次最多执行一次源读取，没有后台循环、无界队列或文件写入；取消在源读取前后生效。内存计数上限为 1 PiB，线程/句柄上限为 1,000,000，异常只映射为固定的 `Unavailable` 状态。
- CPU 通过相邻样本和单调时钟计算，首样本为 `Warmup`，缺失计数器或超过 5 分钟间隔为 `Unavailable`，结果归一并限制在 0～100；一次采样不能证明性能达标。
- `GpAutoLive.Windows.Tests/WindowsProcessPerformanceSamplerTests.cs` 注入源覆盖首样本、取消、计数器边界、源不可用和 CPU 纯逻辑计算。
- 详细边界与后续实机门禁见 [`C6 性能采样边界`](./C6-性能采样边界.md)。仍需在同一 Windows 设备、同一构建和同一素材上形成 Rust/Tauri 与 C# 的冷/热启动、内存、CPU、句柄、线程、GDI/User、GC 和 30 分钟趋势原始报告。

## 20. 当前实施增量：C4 播放状态监视边界（2026-09-02）

`GpAutoLive.Media/MpvPlaybackStateMonitor.cs` 已接入纯逻辑播放状态监视入口：每轮固定、串行读取 `time-pos`、`eof-reached`、`pause` 三项白名单属性，`time-pos=null` 保留为可选毫秒值，非法数值或类型拒绝；轮询间隔限制为 50ms～5s，`WatchAsync` 可取消且不创建无界队列或无人管理后台任务。`GpAutoLive.Windows/WindowsMpvPlaybackRuntime.cs` 已通过 `PollPlaybackStateAsync` 与 `WatchPlaybackStateAsync` 暴露受管入口：轮询期间持有运行时生命周期锁，Watch 每轮校验监视器实例仍属于当前会话；停止/释放后在接触网关前返回 `RuntimeUnavailable` 并结束。监视开始、读取完成与快照应用均复核 `MediaPlaybackIdentity`；过期、缺失或重复身份不会污染当前状态，相同快照仅返回 `Changed=false`。此增量不启动真实 mpv、不修改 `MainWindow`，真实命名管道连续状态、首帧/暂停/EOF、`CompleteCurrent` 联调和进程断开健康门禁仍待 Windows 实机验收。详见 [`C4 播放状态监视边界`](./C4-播放状态监视边界.md)。

## 21. 当前实施增量：C5 固定话术与 Windows 语音边界（2026-09-02）

本增量先锁定固定话术的跨层合同和单操作状态机，不把“状态可用”误报成“Windows 已发声”：

- 本轮已把 `WindowsSystemSpeechAdapter` 接入 `MainWindow` 固定话术卡片，支持受限文本提交、单操作取消和脱敏状态投影；仍未接入 PortAudio/音频总线。

- `GpAutoLive.Contracts/FixedSpeechContracts.cs` 固定 `fixed-speech-command/status` v1、`speak/cancel` 动作、`operation_id` ≤128、文本按 Unicode code point 限制 1～500、错误摘要 ≤500，并复用严格 snake_case JSON 与未知字段拒绝。
- `GpAutoLive.Core/FixedSpeechStateMachine.cs` 作为唯一状态所有者，收敛 `starting → playing → completed/failed/cancelled`；新操作抢占旧操作，麦克风优先时拒绝新话术，迟到/重复回调返回 `Ignored`，状态快照不保存正文。
- `GpAutoLive.Contracts.Tests` 新增固定话术 wire/边界测试，`GpAutoLive.Core.Tests` 新增状态转换、抢占、取消幂等和迟到回调测试；`GpAutoLive.Windows.Tests` 新增 SAPI 适配器启动、完成、取消、超时、抢占、关闭和脱敏测试；另补充控制面调度器单任务生命周期和未授权门禁测试。固定话术增量完成时五个测试项目合计 172 项全部通过；当前总量随 RTMP 宿主增量已达到 183 项（Contracts 13、Core 39、Media 53、Windows 60、App 18）。

（历史记录，2026-09-02）Windows SAPI 已在 `GpAutoLive.Windows` 接入单一可取消适配器和 STA/COM 桥接，当前标记为“代码已接入·待验收”；当时 PortAudio、插话混音、麦克风设备、RTMP、AkVirtualCamera 或抖音 sidecar 尚未接入，后续增量已分别建立对应边界。必须完成 Windows 10/11、中文语音包、启动超时、取消延迟、长稳、设备输出和音频静音/恢复门禁后，才能把固定话术标记为可用。详见 [`C5 固定话术与声音实施边界评估`](./C5-固定话术与声音实施边界评估.md) 与 [`C5 固定话术 Windows 语音实现记录`](./C5-固定话术Windows语音实现记录.md)。

## 22. 当前实施增量：C2 冷启动恢复与后台心跳最小边界（2026-09-02）

- `ControlPlaneAuthCoordinator.RestoreAsync` 只读取 Windows Credential Manager 中的 Refresh Token；缺少凭据不触网。Token 仅在短暂内存缓冲中参与 Refresh，严格 UTF-8/合同校验后交给控制面，UI 快照、日志与普通配置均不包含 Token。
- 恢复必须经过“Refresh → 设备激活”两步；激活返回的账号/设备不匹配、未激活、禁用、撤销或过期均 fail-closed，不能以 `Offline` 绕过冷启动门禁，并按动作清理失效持久凭据。取消会清除当前 pending（包括嵌套激活），不会产生无人管理任务。
- `ControlPlaneHeartbeatScheduler` 只为 `Activated`/已授权 `Offline` 会话运行一个 30 秒循环，立即发送和周期发送共享同一串行闸门；`StopAsync/DisposeAsync` 先取消再等待循环 Join。断网/超时等暂态失败最多写一条按账号/设备绑定的无凭据 `heartbeat.json`，恢复后按原幂等键补发；永久失败或过期项丢弃。
- `IControlPlaneClock` 允许测试控制时间与取消延迟，测试不依赖真实周期睡眠。新增恢复与调度器单实例、Stop Join、未激活 SendNow、凭据清理和状态机取消覆盖。

该增量是可注入的纯逻辑/Windows 边界，不宣称真实控制面联调完成；Credential Manager 权限、TLS、服务端 Refresh/禁用/设备绑定响应矩阵、Windows 10/11 重启与退出长稳仍是实机门禁。

## 23. 当前实施增量：固定话术 WPF 接线（2026-09-02）

- `MainWindow` 的“声音与互动”卡片已接入受限文本框（最多 500 个字符）、朗读、取消和脱敏状态显示；按钮只调用 `WindowsSystemSpeechAdapter`，不读取凭据、不启动模型、不写入正文。
- 朗读操作使用唯一 `operation_id`，新操作由适配器按既有抢占规则处理；窗口关闭时先取消并有界释放 SAPI STA/COM 资源。
- UI 仍把固定话术标成“代码已接入·待验收”，麦克风插话、普通声音、PortAudio 与抖音 M1 的优先级/混音链路没有被假接入。
- v10 发布验证包重新生成并完成资源清单校验：发布根目录 15 个文件、1,047,971 bytes，`GpAutoLive.exe` 仍为 162,816 bytes；启动窗口标题为 `GpAutoLive`，约 1.8 秒后正常退出，退出码 0，无残留媒体进程。该启动采样不是性能达标证明。

## 24. 当前实施增量：RTMP/RTMPS 参数边界（2026-09-02）

- `GpAutoLive.Contracts/RtmpContracts.cs` 固定协议、轨道、尺寸、帧率、码率、错误和脱敏地址规则；不触网、不保存完整推流地址到状态快照。
- `GpAutoLive.Media/RtmpFfmpegCommandBuilder.cs` 把已探测媒体和配置转换为不可变 FFmpeg 参数计划，画面直接读取源媒体，声音明确预留最终 PCM `pipe:0`，并限制参数数量和 Windows 命令长度。
- `MainWindow` 右侧输出卡片已提供地址、画面/声音轨道、“校验输出配置”以及画面/声音/音画“开始推流/停止推流”入口；声音路径已由 `WindowsRtmpAudioSession` 接入最终 PCM 分流，资源或会话校验失败仍 fail-closed。
- 编码器候选按 `h264_nvenc → h264_amf → h264_qsv → h264_mf → libopenh264` 单向降级；后续已接入本机逐候选一帧探测，真实目标 GPU/网络门禁仍待验收。
- `GpAutoLive.Windows/WindowsRtmpOutputManager.cs` 已把参数计划接入 Windows 进程生命周期：隐藏启动、Job Object/进程树兜底、立即退出/启动失败分类、停止 2 秒 Join、stderr 有界排空、最终 PCM 单写入者和取消释放；WPF 已绑定画面开始/停止按钮，不自动重试。
- 新增 Contracts 4 项、Media 3 项、Windows 4 项测试，并补充含声音计划不关闭 stdin 的断言；当前全量测试为 183 项通过。真实 ZLMediaKit/RTMPS、PortAudio/最终 PCM 总线、断线重试和编码器实机门禁仍待实施。

## 25. 当前实施增量：RTMP Windows 宿主生命周期（2026-09-02）

- 宿主只接受 `RtmpFfmpegLaunchPlan`，不接受任意命令行；启动前仍由合同和媒体路径策略完成协议、轨道、路径与参数上限校验。
- `StartAsync` 只在 Windows 启动隐藏 FFmpeg，并优先绑定 Job Object；进程树、stdin、stderr 读取任务和退出事件均有所有者。启动后立即退出、启动异常、取消、重复启动和已关闭宿主分别映射稳定错误码。
- `WriteFinalPcmAsync` 只允许当前声音计划，单次最多 96,000 个 float，使用固定池化字节缓冲和串行写入；停止先阻止新写入并等待当前写入，再关闭 stdin。
- `StopAsync/DisposeAsync` 释放 stdin、stderr 读取任务、Job Object 和进程树，stderr 超过 64 KiB 后继续丢弃读取以避免管道反压；不保留 FFmpeg 原文日志。
- 本地测试只覆盖未启动停止、缺少 FFmpeg 资源的 fail-closed、未运行 PCM 写入和关闭后拒绝新启动，不访问网络；WPF 按已验证资源启动画面直推宿主，但真实 FFmpeg/编码器/PortAudio/ZLMediaKit 仍需目标 Windows 设备门禁。`desktop/` 未修改。
- v14 发布验证包已包含宿主程序集：发布根目录 15 个文件、1,114,706 bytes，`GpAutoLive.exe` 162,816 bytes；媒体 staging 树 352,053,564 bytes，资源清单 4/4 哈希匹配。启动约 1.8 秒后正常关闭，退出码 0 且无残留媒体进程；该冒烟不等价于真实推流或性能达标。

## 26. 当前实施增量：PortAudio 设备枚举与 WPF 刷新（2026-09-02）

- `GpAutoLive.Windows/WindowsPortAudioDeviceEnumerator.cs` 仅允许加载固定名称 `portaudio_x64.dll`，使用 `NativeLibrary` 动态解析 PortAudio v19 导出；初始化、设备数量上限、设备信息、Host API、默认输出设备和原生句柄释放均在一个有界调用内完成。
- 设备名称和 Host API 名称限制为 256 字符并清除控制字符；快照只返回设备索引、输入/输出通道数和有限采样率，不返回 DLL 路径、原生异常或日志正文。
- `MainWindow` 的“声音与互动”卡片新增输出设备下拉框和“刷新本机设备”按钮。只有登录、媒体资源清单校验且 PortAudio DLL 已验证时才枚举；失败不会回退 PATH，UI 不会无意启动音频流。
- `tools/stage-media-runtime.ps1` 新增可选 `-PortAudioRoot`，PortAudio DLL 与许可证材料独立进入媒体资源包；主 EXE 不内嵌该 DLL。
- 本机动态加载冒烟成功：PortAudio 可用，枚举 32 个设备，默认输出索引为 5；输出流另以 48 kHz/双声道/256 帧配置启动约 500 ms 后正常停止，欠载只输出静音并累计统计。自动化测试当前为 193 项通过（Contracts 13、Core 39、Media 53、Windows 70、App 18）。
- v16 资源包验证：5 项资源哈希匹配，发布根目录 15 个文件、1,140,822 bytes，`GpAutoLive.exe` 162,816 bytes；媒体 staging 树 352,365,694 bytes。启动约 1.8 秒后正常关闭，退出码 0 且无残留媒体进程。真实音频总线、PCM 时钟、设备切换恢复和插话混音仍待实施。

## 27. 当前实施增量：PortAudio 长期输出流边界（2026-09-02）

- 新增 `WindowsPortAudioNative.cs`，把 PortAudio v19 的枚举与输出流 ABI 收敛到一个动态加载器；只从资源清单验证后的固定 DLL 加载，不搜索 PATH，不把原生库链接进主 EXE。
- 新增 `WindowsPortAudioOutputStream.cs`，单一所有者负责 `OpenStream → StartStream → StopStream → CloseStream → Terminate` 生命周期；配置限制为设备索引、1～8 声道、8 kHz～384 kHz、16～4096 帧，错误不回显路径或原生正文。
- 输出回调只消费 `AudioPcmRingBuffer`，使用预分配 `float[]` 和 `Marshal.Copy`，无文件/网络 I/O；环缓不足时输出静音并统计 `UnderrunFrames`，回调异常返回 `Abort`，关闭、取消和重复停止均有界。
- `WindowsPortAudioDeviceEnumerator` 与输出流共用动态 ABI，探测通过 `SemaphoreSlim` 串行化，避免并行 `Pa_Initialize/Pa_Terminate` 破坏全局 PortAudio 状态。
- 新增 5 项输出流边界测试，覆盖路径、资源、配置、取消、停止幂等和关闭门禁；本机真实 PortAudio 输出流冒烟已成功，仍未连接普通声音候选、SAPI 静音/恢复、RTMP 最终 PCM 分流或设备丢失自动恢复。
- v18 本地验证包包含 5 项独立媒体资源且清单哈希全部匹配：发布根目录 15 个文件、1,159,006 bytes，`GpAutoLive.exe` 162,816 bytes；包含媒体 staging 的目录树 28 个文件、353,524,700 bytes。启动约 1.8 秒后标题为 `GpAutoLive`，退出码 0，无残留 `GpAutoLive/mpv/ffmpeg/ffprobe` 进程。

## 28. 当前实施增量：最终 PCM 双消费者总线与 RTMP 分流（2026-09-02）

- 新增 `GpAutoLive.Media/FinalPcmBus.cs`，把一份最终交错 PCM 原子地发布到本机 PortAudio 和 RTMP 两个固定容量环缓；发布上限 4096 帧，两个消费者分别统计可用帧和丢弃帧，不创建无界队列。
- 新增 `GpAutoLive.Windows/WindowsRtmpFinalPcmPump.cs`，单一有界任务从 RTMP 环缓串行写入 `WindowsRtmpOutputManager` 的 `pipe:0`，使用预分配分片、10ms 空闲退让、取消令牌和任务 Join 责任，不读取源媒体第二份声音。
- 新增 Media 4 项总线测试和 Windows 3 项分流泵测试，覆盖双路顺序、满载丢旧、形状/容量、关闭、缺宿主、宿主未运行和关闭来源排空。
- 自动化测试增至 200 项通过（Contracts 13、Core 39、Media 57、Windows 73、App 18）。当前总线仍未连接普通声音候选、SAPI 静音/恢复、插话优先级或 WPF 播放按钮；RTMP/PortAudio 的真实设备、编码器、网络和时钟门禁仍保持“待验收”。
- v19 本地验证包包含 5 项独立媒体资源且清单哈希全部匹配：发布根目录 15 个文件、1,176,962 bytes，`GpAutoLive.exe` 162,816 bytes；包含媒体 staging 的目录树 28 个文件、353,542,656 bytes。启动约 1.8 秒后标题为 `GpAutoLive`，退出码 0，无残留媒体进程。

## 29. 当前实施增量：PortAudio 麦克风输入流边界（2026-09-02）

- 新增 `GpAutoLive.Windows/WindowsPortAudioInputStream.cs`，仅在调用方显式传入固定 DLL、设备索引和配置后打开输入流；输入格式为交错 float32，采样率限制为 16/32/44.1/48 kHz，声道 1～2，帧块 16～4096。
- 输入回调使用预分配缓冲，将 PCM 写入指定固定容量环缓；不启动 ASR、LLM、TTS，不写文件、不上传、不读取用户目录列表；欠载/回调异常只记录脱敏计数并停止流。
- 新增 5 项输入流边界测试；本机短时冒烟选择默认输入设备，14 个输入设备可见，流启动约 500 ms 后停止成功，捕获 23,040 帧，未持久化音频。
- 新增 `MicrophoneInterludeGate` 能量门控纯逻辑及 4 项测试；历史增量当时为 211 项通过（Contracts 13、Core 39、Media 63、Windows 78、App 18）。
- 新增 `GpAutoLive.Media/MicrophoneInterludeGate.cs`，提供本地 RMS 能量阈值、迟滞和 250ms 默认 hangover 的脱敏 Speaking/Hangover/Armed 状态；它不是 SpeexDSP，未实现 AEC/降噪/AGC，不能替代完整 VAD。
- `GpAutoLive.Media/AudioPcmMixer.cs` 和 `FinalPcmBus.TryPublishMixed` 已接入预分配目标缓冲的有限值清理、duck、静音、限幅和双消费者发布边界；详见 [`C4 PCM 混音边界实现记录`](./C4-音频混音边界实现记录.md)。
- `FfmpegPcmDecodePlanBuilder` 与 `WindowsFfmpegPcmDecoder` 已接入首条音频轨道到固定容量 PCM 环缓的有界解码入口，详见 [`C4 FFmpeg PCM 解码边界`](./C4-FFmpeg%20PCM解码边界实现记录.md)。当前仍未接入 N/N+1 候选编排、WPF 播放按钮和可听时钟。
- 当前仍未接入 SpeexDSP/AEC/降噪/AGC、麦克风优先级抢占、固定话术/插话静音恢复或 WPF 监听开关，因此能力继续标记“正式需求·待实施/未接入”。
- v20 本地验证包包含 5 项独立媒体资源且清单哈希全部匹配：发布根目录 15 个文件、1,199,718 bytes，`GpAutoLive.exe` 162,816 bytes；包含媒体 staging 的目录树 28 个文件、353,565,412 bytes。启动约 1.8 秒后标题为 `GpAutoLive`，退出码 0，无残留媒体进程。
- v21 本地验证包包含 5 项独立媒体资源且清单哈希全部匹配：发布根目录 15 个文件、1,201,514 bytes，`GpAutoLive.exe` 162,816 bytes；包含媒体 staging 的目录树 28 个文件、353,567,208 bytes。启动约 1.8 秒后标题为 `GpAutoLive`，退出码 0，无残留媒体进程。
- v22 本地验证包包含 5 项独立媒体资源且清单哈希全部匹配：发布根目录 15 个文件、1,201,514 bytes，`GpAutoLive.exe` 162,816 bytes；包含媒体 staging 的目录树 28 个文件、353,567,208 bytes。启动约 2.2 秒后标题为 `GpAutoLive`，私有字节约 78.31 MiB、工作集约 134.85 MiB、21 个线程、1,076 个句柄，正常关闭且无残留 `GpAutoLive/mpv/ffmpeg/ffprobe` 进程。
- v23（包含音频优先级策略）本地验证包包含 5 项独立媒体资源且清单哈希全部匹配：发布根目录 15 个文件、1,215,548 bytes，`GpAutoLive.exe` 162,816 bytes；包含媒体 staging 的目录树 28 个文件、353,581,242 bytes。启动约 2.2 秒后标题为 `GpAutoLive`，私有字节约 90.89 MiB、工作集约 149.18 MiB、21 个线程、1,076 个句柄，正常关闭且无残留 `GpAutoLive/mpv/ffmpeg/ffprobe` 进程。
- v24（包含 PCM 混音边界）本地验证包包含 5 项独立媒体资源且清单哈希全部匹配：发布根目录 15 个文件、1,221,844 bytes，`GpAutoLive.exe` 162,816 bytes；包含媒体 staging 的目录树 28 个文件、353,587,538 bytes。启动约 2.2 秒后标题为 `GpAutoLive`，私有字节约 85.51 MiB、工作集约 141.15 MiB、21 个线程、1,076 个句柄，正常关闭且无残留 `GpAutoLive/mpv/ffmpeg/ffprobe` 进程。

## 30. 当前实施增量：音频优先级与静音策略边界（2026-09-02）

- 新增 `GpAutoLive.Core/AudioPriorityCoordinator.cs`，由单一策略所有者固定 `麦克风插话 > 固定话术 > 插话文件 > 原媒体` 层级。
- 固定话术期间普通媒体与插话被静音；插话文件期间普通媒体只 duck；麦克风开始说话立即截断固定话术/插话，结束后回到原媒体且不自动续播被截断内容。
- 策略只返回脱敏快照、抢占层和 generation，不处理 PCM、SAPI、PortAudio 或文件；`WindowsSystemSpeechAdapter` 与 WPF 主窗口共享该所有者，避免固定话术另建音频事实源。
- `WindowsAudioPlaybackController` 已把 FFmpeg PCM 解码、固定容量环缓和 PortAudio 输出组合为单一可取消会话；WPF 纯音频播放/暂停/恢复/停止已接线，仍待设备恢复/音画时钟门禁。
- 新增 `AudioPriorityCoordinator`、`AudioPcmMixer`、`FfmpegPcmDecodePlanBuilder`、`WindowsFfmpegPcmDecoder` 和 `WindowsAudioPlaybackController`，并补充 24 项边界测试；历史快照为 235 项，后续普通声音预载和性能采样已继续推进。真实插话候选编排、PCM 混音/duck 曲线、SAPI 音量静音恢复和麦克风 DSP 仍待实机门禁。
- v25（包含 FFmpeg PCM 解码与单一音频播放控制器）本地验证包包含 5 项独立媒体资源且清单哈希全部匹配：发布根目录 15 个文件、1,263,728 bytes，`GpAutoLive.exe` 162,816 bytes；包含媒体 staging 的目录树 28 个文件、353,629,422 bytes。启动约 2.2 秒后标题为 `GpAutoLive`，私有字节约 80.50 MiB、工作集约 137.35 MiB、22 个线程、1,079 个句柄，正常关闭且无残留 `GpAutoLive/mpv/ffmpeg/ffprobe` 进程。

## 31. 当前实施增量：普通声音 N/N+1 预载调度边界（2026-09-02）

- 新增 `GpAutoLive.Media/AudioCyclePrewarmCoordinator.cs`，把 N+1 候选计划限制为 `Planned → Preparing → Prepared → Committing` 四态；N+2 只由调用方保留元数据，不在此处创建计划。
- 计划使用绝对媒体时间，`planned` 立即触发一次 `Prepare`；`prepared` 根据环缓待播毫秒数和播放倍率触发 `Commit`；超过 500ms 媒体宽限期触发 `Expire`；`committing` 不重复提交。
- 基准时间、周期、当前时间、播放倍率和待播时间均有限值清理，目标时间饱和到 `long.MaxValue`；该切片不启动解码、不拥有 PCM 队列、不恢复旧候选。
- 新增 7 项纯逻辑测试，历史自动化测试为 245 项通过；暂停门控、PortAudio 输出流和 WPF 纯音频接线已在后续增量继续推进。真实 N/N+1 解码、A/B 切换、交叉淡化、设备恢复和可听 PTS 主时钟仍待实施。
- v26（包含 N/N+1 预载调度边界）本地验证包包含 5 项独立媒体资源且清单哈希全部匹配：发布根目录 15 个文件、1,268,624 bytes，`GpAutoLive.exe` 162,816 bytes；包含媒体 staging 的目录树 28 个文件、353,634,318 bytes。启动约 2.2 秒后标题为 `GpAutoLive`，私有字节约 77.77 MiB、工作集约 134.37 MiB、22 个线程、1,079 个句柄，正常关闭且无残留 `GpAutoLive/mpv/ffmpeg/ffprobe` 进程。

## 32. 当前实施增量：C6 本地进程基线采集（2026-09-02）

- 新增 `tools/collect-process-baseline.ps1`，按固定暖机、采样间隔和总时长启动指定 Windows EXE，采集私有工作集、工作集、CPU、线程和句柄趋势并输出版本化 JSON。
- v26 空闲工作台 4 秒冒烟生成 7 个样本，报告状态为 `completed`：私有工作集 64,339,968～82,157,568 bytes，工作集 119,189,504～141,537,280 bytes，CPU 峰值约 3.71%。该数据仅作为后续同机对照输入，不等价于播放、GPU、声卡或 30 分钟性能达标。
- 采集脚本不上传数据、不读取媒体正文，结束优先正常关闭窗口；脚本与 JSON 输出均只放在 C# Windows 目录内。
- `MainWindow` 已以 1Hz `DispatcherTimer` 低优先级投影该采样器的脱敏结果到标题栏；采样任务有并发门禁，窗口关闭时对称停止并取消，不影响登录或媒体状态机。
- 性能显示格式化新增 3 项 App 测试；当前自动化测试总数为 245 项通过（Contracts 13、Core 45、Media 79、Windows 87、App 21）。
- v27（包含 WPF 性能指标投影）发布根目录 15 个文件、1,272,352 bytes，`GpAutoLive.exe` 162,816 bytes；媒体 staging 树 28 个文件、353,638,046 bytes，资源清单哈希全部匹配。4 秒基线冒烟生成 7 个样本，私有工作集峰值 84,189,184 bytes、工作集峰值 145,793,024 bytes、CPU 峰值约 3.98%，退出后无媒体进程残留。

## 33. 当前实施增量：纯音频播放暂停与 WPF 接线（2026-09-02）

- `WindowsAudioPauseGate` 以一个可取消的门控信号暂停 FFmpeg 继续读取，保留固定容量 PCM 环缓和当前解码位置；PortAudio 输出流新增幂等 `Pause/Resume`，原生暂停/恢复失败会取消并有界回收整条会话。
- `WindowsAudioPlaybackController` 新增 `Paused` 状态、`PauseAsync/ResumeAsync`，有限音轨结束前最多等待 5 秒排空环缓，避免尾部 PCM 被关闭流程截断；停止和关闭会先释放暂停等待者，再取消进程并 Join。WPF 对普通声音采用有限单项会话，不把多项播放伪装成当前源自循环。
- `MainWindow` 纯音频播放路径已接入：播放前完成已验证媒体清单、FFmpeg/PortAudio 固定资源和用户选择的输出设备校验；播放、暂停、恢复、停止均先操作真实音频会话，再提交播放池快照。切换媒体项不复用旧解码会话，切到音频时重新建立会话，切到视频时无可用播放器则 fail-closed 停止。
- 新增暂停门控、播放控制器无会话暂停/恢复、PortAudio 无流暂停/恢复测试；当前自动化测试为 253 项通过（Contracts 13、Core 45、Media 79、Windows 95、App 21）。显式 FFmpeg + PortAudio fixture 已实际走通解码、设备输出、暂停/恢复、会话排空和清理。
- v28 发布候选根目录 15 个文件、1,289,024 bytes，`GpAutoLive.exe` 162,816 bytes；外置媒体 staging 树 28 个文件、353,652,726 bytes，5/5 资源大小与 SHA-256 全部匹配。使用本机 .NET 10 运行时启动约 3 秒后标题为 `GpAutoLive`、正常退出码 0，无残留媒体进程；4 秒基线 6 个样本，私有工作集 80,699,392～83,865,600 bytes、工作集 137,113,600～145,739,776 bytes、CPU 峰值约 2.76%。
- 待验收项仍包括真实 N/N+1 候选/A-B 淡化、设备丢失恢复、可听 PTS 主时钟、SAPI/插话 PCM 静音恢复、麦克风 DSP、RTMP/虚拟摄像头/抖音 sidecar 真实矩阵和 30 分钟性能门禁。

## 35. 当前实施增量：RTMP 画面直推 WPF 控制接线（2026-09-02）

- `MainWindow` RTMP 卡片新增“开始推流/停止推流”按钮；开始前必须通过登录门禁、RTMP 合同、已验证媒体资源清单和固定 `ffmpeg.exe` 资源校验，禁止回退到 PATH 或将完整推流地址写入 UI/日志。
- 画面轨道调用 `WindowsRtmpOutputManager.StartAsync`，声音或音画轨道调用 `WindowsRtmpAudioSession` 解码并启动唯一最终 PCM 分流泵；停止统一走有界 `StopAsync`，按钮状态由宿主快照单向投影，重复启动和关闭保持幂等。
- 推流源绑定当前视频媒体和四维 `RtmpSourceIdentity`，FFmpeg 画面直接读取源文件，不捕获桌面/窗口；宿主仍使用 `ArgumentList`、隐藏进程、Job Object/进程树回收、stderr 有界排空和取消清理。
- 固定话术/SAPI、纯音频播放、RTMP 宿主的生命周期互不持有对方锁；窗口关闭先取消 UI 命令再有界回收音频、视频和 RTMP 资源。固定话术适配器的操作释放增加单次 claim，防止关闭与监视器竞态造成 COM 操作二次释放。
- 媒体池导入、重排、移除、清空和上一项/下一项切换前会先停止活动 RTMP 宿主，避免旧 `RtmpSourceIdentity` 继续向外发布；宿主本身不做后台源监听，外部调用方仍必须使用身份门禁。
- 本增量未新增程序集或无界队列；当时全量自动化测试为 253 项通过，后续总线与会话增量已记录在第 36 节。`dotnet format --verify-no-changes` 与 `tools/verify-scope.ps1` 通过。真实编码器选择/降级、ZLMediaKit/RTMPS 网络和断线重试继续标记“代码已接入·待验收/正式需求·待实施”。
- v33 发布候选根目录 15 个文件、1,305,608 bytes，`GpAutoLive.exe` 162,816 bytes；外置媒体 staging 树 13 个资源文件、352,365,694 bytes，5/5 资源大小与 SHA-256 全部匹配。使用本机 .NET 10 运行时启动约 3 秒后标题为 `GpAutoLive`、退出码 0，无残留 `GpAutoLive/mpv/ffmpeg/ffprobe` 进程；4 秒基线生成 7 个样本，私有工作集峰值 97,173,504 bytes、工作集峰值 160,325,632 bytes、CPU 峰值约 7.44%。该样本用于同机对照，不等价于播放或 30 分钟性能达标。

## 37. 当前实施增量：RTMP H.264 编码器本机探测（2026-09-02）

- 新增 `GpAutoLive.Media/RtmpEncoderProbe.cs`，使用固定的本地 `lavfi` 黑帧，对 `h264_nvenc → h264_amf → h264_qsv → h264_mf → libopenh264` 逐项执行一帧编码探测；每个候选最多 5 秒、标准输出/错误各 16 KiB，不访问 RTMP 网络。
- `MainWindow` 在画面或音画推流启动前调用探测器，仅把选中的编码器名称传给 `WindowsRtmpOutputManager` 或 `WindowsRtmpAudioSession`；声音-only 不启动编码器探测，探测失败保持 fail-closed。
- 新增 4 项纯逻辑探测测试和 1 项显式 FFmpeg 夹具测试，覆盖首个可用候选、首选候选起点、全部失败、未知候选拒绝及本机真实一帧探测；全量自动化测试为 262 项通过（Contracts 13、Core 45、Media 83、Windows 100、App 21）。
- 编码器探测只证明本机 FFmpeg 能完成最小编码调用，不等价于目标显卡矩阵、驱动稳定性、ZLMediaKit/RTMPS 网络或 30 分钟推流门禁。
- 显式设置 `AUTOLIVE_TEST_FFMPEG` 与 `AUTOLIVE_TEST_PORTAUDIO_DLL` 后，FFmpeg 编码器探测和 FFmpeg→FinalPcmBus→PortAudio 夹具联合 2/2 通过；该验证仍不包含目标推流服务器网络。
- v36（含 H.264 编码器本机探测）发布候选根目录 15 个文件、1,335,892 bytes，`GpAutoLive.exe` 162,816 bytes；外置媒体 staging 树 13 个资源文件、352,365,694 bytes，5/5 资源大小与 SHA-256 全部匹配。4 秒基线生成 7 个样本，私有工作集峰值 97,320,960 bytes、工作集峰值 160,333,824 bytes、CPU 峰值约 7.08%；启动关闭冒烟标题为 `GpAutoLive`、退出码 0，私有工作集 83,836,928 bytes、工作集 144,896,000 bytes、21 线程、1,099 句柄，无 `GpAutoLive/mpv/ffmpeg/ffprobe` 残留。
- v37（含停止阶段取消隔离修复）发布候选根目录 15 个文件、1,335,892 bytes，`GpAutoLive.exe` 162,816 bytes；外置媒体 staging 树 13 个资源文件、352,365,694 bytes，5/5 资源大小与 SHA-256 全部匹配。4 秒基线生成 7 个样本，私有工作集峰值 96,739,328 bytes、工作集峰值 159,838,208 bytes、CPU 峰值约 7.50%；启动关闭冒烟标题为 `GpAutoLive`、退出码 0，私有工作集 83,427,328 bytes、工作集 143,679,488 bytes、23 线程、1,107 句柄，无 `GpAutoLive/mpv/ffmpeg/ffprobe` 残留。

## 38. 当前实施增量：WPF 可访问性与 v38 本地交付包（2026-09-03）

- `src/GpAutoLive.App/MainWindow.xaml` 为媒体池、快捷效果、RTMP 校验/推流、播放控制和本机音频设备刷新按钮补齐 `AutomationProperties.Name`；共检查 22 个按钮，未发现缺失名称。
- RTMP 状态文案改为明确区分“画面/声音会话已接入”和“真实网络待验收”，避免 UI 将本机接线误报为 ZLMediaKit/RTMPS 已通过。
- WPF 视觉结构、功能状态机和资源拆分未改变；本次只修正辅助技术入口与状态表达，不引入新程序集、无界队列或额外运行时依赖。
- 全量自动化测试保持 262 项通过（Contracts 13、Core 45、Media 83、Windows 100、App 21）；`dotnet format --verify-no-changes` 与 `tools/verify-scope.ps1` 均通过。
- v38 发布候选目录为 `artifacts/csharp-windows-controller-20260903-v38`：根目录 15 个文件、1,337,932 bytes，`GpAutoLive.exe` 162,816 bytes；外置媒体 staging 13 个文件、352,365,694 bytes，5/5 资源大小与 SHA-256 全部匹配。4 秒基线生成 7 个样本，私有工作集峰值 96,825,344 bytes、工作集峰值 159,805,440 bytes、CPU 峰值约 8.36%；启动关闭冒烟标题包含 `GpAutoLive`、退出码 0、私有工作集 3,268,608 bytes、工作集 21,893,120 bytes、8 线程、224 句柄，无 `GpAutoLive/mpv/ffmpeg/ffprobe` 残留。
- v38 仍只证明本机发布包、资源清单和空闲启动边界；真实 mpv 首帧/EOF、PortAudio/SAPI 设备恢复、RTMP/RTMPS 网络、目标 GPU 矩阵和 30 分钟性能门禁继续保持待验收。

## 39. 当前实施增量：视频 EOF 观察与媒体编辑生命周期收敛（2026-09-03）

- `WindowsMpvPlaybackController` 新增有界 `WatchPlaybackStateAsync` 转发入口；WPF 视频启动后消费 `time-pos/eof-reached/pause` 快照，只有当前四维 `MediaPlaybackIdentity` 仍匹配且 `eof-reached=true` 才调用 `MediaPoolOwner.CompleteCurrent`。
- 视频 EOF 后，下一项为视频时复用同一 mpv 进程执行受管换源；下一项为音频时先关闭 mpv，再启动 FFmpeg→FinalPcmBus→PortAudio 声音会话；单项池通过新身份重新加载同一源，不创建版本文件或无界队列。
- 状态监视发生 IPC/运行时错误时停止播放池并清理 mpv；手动停止、上下项切换、媒体导入/上移/下移/移除/清空、最终效果窗口关闭和主窗口关闭均取消并有界 Join 观察任务。
- 新增控制器“无运行时观察返回稳定错误”测试；全量自动化测试为 263 项通过（Contracts 13、Core 45、Media 83、Windows 101、App 21）。
- 本增量还修复媒体池编辑只停止 RTMP、未统一回收本地音频/mpv 的生命周期缺口；编辑前由 `StopMediaForMutationAsync` 串行停止 RTMP、PortAudio/FFmpeg 和 mpv，编辑失败仍保留原播放池快照。
- 该增量完成的是本地观察与资源生命周期接线，不等价于真实 mpv 首帧、HWND 渲染、EOF 连续换源和 GPU83/CPU4 实机门禁；这些仍需目标 Windows 设备验证。
- v39 发布候选目录为 `artifacts/csharp-windows-controller-20260903-v39`：根目录 15 个文件、1,350,344 bytes，`GpAutoLive.exe` 162,816 bytes；外置媒体 staging 13 个文件、352,365,694 bytes，5/5 资源大小与 SHA-256 全部匹配。4 秒基线生成 7 个样本，私有工作集峰值 96,579,584 bytes、工作集峰值 159,350,784 bytes、CPU 峰值约 7.09%；启动关闭冒烟标题包含 `GpAutoLive`、退出码 0、私有工作集 3,289,088 bytes、工作集 21,909,504 bytes、8 线程、224 句柄，无 `GpAutoLive/mpv/ffmpeg/ffprobe` 残留。

## 40. 当前实施增量：真实 mpv/命名管道/HWND 夹具（2026-09-03）

- 运行资源白名单补齐外置 `d3dcompiler_43.dll`，修复实际发布清单在 `LoadAndVerifyAsync` 阶段被错误拒绝的问题，并新增清单解析回归测试。
- `MpvIpcFrameParser` 的事件字段按 mpv JSON IPC 的公共字段和事件专属字段扩展（`playlist_entry_id`、`file_error`、`playlist_insert_*`、`prefix/level/text`、`args/result/hook_id`），继续严格拒绝响应帧未知字段；事件帧只被安全跳过，不参与命令响应配对。
- 新增 `WindowsMpvRealFixtureTests`：显式提供 v39 外置运行资源、FFmpeg 生成的 2 秒本地 MP4 和原生 Windows HWND 后，真实 mpv 通过命名管道完成启动/播放；`Original`、`Cpu4`、`Gpu83` 三种模式均观察到正的 `time-pos` 和 `eof-reached`，并在 finally 中验证受管关闭。
- 该夹具为单机 Windows 证据，不等价于目标显卡矩阵、首帧画面像素比对、30 分钟 GPU 长稳或远端 ZLMediaKit 网络验收；这些门禁仍保持待验收。

## 41. 当前实施增量：v41 发布候选与全量回归（2026-09-03）

- 基于最新源码重新执行 `dotnet publish`，生成 `artifacts/csharp-windows-controller-20260903-v41`；发布根目录 15 个文件、1,350,340 bytes，`GpAutoLive.exe` 162,816 bytes。媒体运行时继续独立位于 `runtime/media/1.0.0`，通过硬链接复用已验证资源，运行时树 13 个文件、352,365,694 bytes，未把 mpv/FFmpeg/PortAudio/D3D 编译器 DLL 内嵌进主 EXE。
- 资源清单包含 5 项资源，逐项大小与 SHA-256 校验通过；`d3dcompiler_43.dll` 已纳入白名单和清单验证，缺失或哈希不匹配仍 fail-closed。
- Release 构建 0 警告、0 错误；五个测试项目共 265 项通过（Contracts 13、Core 45、Media 84、Windows 102、App 21）。`dotnet format --verify-no-changes --no-restore` 与 `tools/verify-scope.ps1` 均通过，`desktop/` 未修改。
- 在 v41 资源包上显式运行真实 mpv/HWND/命名管道夹具，`Original`、`Cpu4`、`Gpu83` 均通过正 `time-pos` 与 `eof-reached` 观察，并在 finally 中完成受管关闭；这是单机运行时证据，不等价于首帧像素、目标 GPU 矩阵或网络推流门禁。
- v41 启动关闭冒烟使用本地 .NET 10 运行时根目录，窗口标题为 `GpAutoLive.exe`，`CloseMainWindow` 退出码 0，私有工作集 3,276,800 bytes、工作集 21,815,296 bytes、8 线程、224 句柄，退出后无 `GpAutoLive/mpv/ffmpeg/ffprobe` 残留。
- v41 4 秒空闲基线生成 7 个样本，状态为 `completed`：私有工作集峰值 83,668,992 bytes、工作集峰值 144,838,656 bytes、CPU 峰值约 3.50%。该数据仅用于同机对照，不能替代播放、音频设备、GPU、RTMP 或 30 分钟长稳验收。

## 42. 当前实施增量：v42 生命周期修复与发布复验（2026-09-03）

- 修复纯音频自然 EOF 观察者在释放其取消源后继续进入播放串行闸门的边界：自然完成回调只依赖四维播放身份和控制器终态校验，改用不可取消令牌进入串行门，手动停止/切源仍由观察者取消负责。
- 基于该修复重新发布 `artifacts/csharp-windows-controller-20260903-v42`：根目录 15 个文件、1,351,812 bytes，`GpAutoLive.exe` 162,816 bytes；媒体运行时保持独立 13 个文件、352,365,694 bytes，未内嵌到主 EXE。
- v42 资源清单 5/5 大小与 SHA-256 校验通过；启动关闭冒烟标题为 `GpAutoLive`，正常 `CloseMainWindow` 退出码 0，私有工作集 82,321,408 bytes、工作集 139,321,344 bytes、21 线程、1,074 句柄，退出后无媒体进程残留。
- v42 4 秒空闲基线生成 7 个样本：私有工作集峰值 84,377,600 bytes、工作集峰值 145,375,232 bytes、CPU 峰值约 3.25%。这仍是同机空闲基线，不等价于 30 分钟性能门禁。
- v42 外部资源包上的真实 mpv/HWND/命名管道夹具 `Original`、`Cpu4`、`Gpu83` 均通过正 `time-pos` 与 `eof-reached`，全量自动化测试仍为 265 项通过；真实 ZLMediaKit、设备恢复、AkVirtualCamera、抖音 M1 和跨 GPU 矩阵保持待验收。

## 43. 当前实施增量：正式安装包与符号包拆分（2026-09-03）

- 按发布策略将 PDB/XML 从正式安装目录移出，生成独立符号包 `artifacts/csharp-windows-symbols-20260903-v43`；正式安装包 `artifacts/csharp-windows-controller-20260903-v43` 根目录仅保留 8 个运行文件、1,027,815 bytes，`GpAutoLive.exe` 仍为 162,816 bytes。
- 外置媒体运行时继续单独放置 13 个文件、352,365,694 bytes；符号包 7 个文件、323,997 bytes，不会被主 EXE 或运行时加载。
- v43 资源清单 5/5 大小与 SHA-256 校验通过；启动关闭冒烟正常 `CloseMainWindow` 退出码 0、退出后无 `GpAutoLive/mpv/ffmpeg/ffprobe` 残留。由于同机启动采样存在冷启动抖动，本次冒烟样本为私有工作集 96,686,080 bytes、工作集 154,046,464 bytes、21 线程、1,074 句柄；以 4 秒基线作为对照更稳定。
- v43 4 秒空闲基线生成 7 个样本：私有工作集峰值 83,865,600 bytes、工作集峰值 145,395,712 bytes、CPU 峰值约 3.18%。该数据仅用于同机对照，不能替代 30 分钟性能验收。
- v43 外部资源包真实 mpv/HWND/命名管道夹具 `Original`、`Cpu4`、`Gpu83` 均通过正 `time-pos` 与 `eof-reached`；全量自动化测试、格式检查、作用域检查仍保持通过。

## 36. 当前实施增量：最终 PCM 总线接入纯音频会话（2026-09-02）

- `WindowsFfmpegPcmDecoder.DecodeAsync` 现在支持二选一输出目标：传统单一 PCM 环缓，或 `FinalPcmBus`；两者不能同时传入，声道数必须与计划一致。解码器只写预分配分片，不把完整音轨载入内存。
- `WindowsAudioPlaybackController.StartAsync` 增加可选最终 PCM 总线；会话成功后由控制器接管总线关闭，PortAudio 消费 `OutputBuffer`，另一固定容量 `RtmpBuffer` 保留给 RTMP 分流泵，停止/EOF/取消均沿同一生命周期关闭。
- WPF 纯音频路径已创建并传入最终 PCM 总线；启动失败由调用方立即关闭未接管的总线，成功会话由控制器释放，避免泄漏或双重所有权。RTMP 音频/音画路径由 `WindowsRtmpAudioSession` 创建独立最终 PCM 总线和分流泵，声音泵已由 UI 开始/停止控制。
- 显式 FFmpeg + PortAudio fixture 已实际验证总线会话：有限音频完成后 `PublishedFrames > 0`、RTMP 消费环缓存在关闭时仍有帧，且 PortAudio 会话正常排空和清理。该证据证明本机总线接线，不等价于 RTMP 网络发布通过。
- 本增量不新增程序集、不引入无界队列；RTMP 声音/音画会话编排已由 `WindowsRtmpAudioSession` 接入 WPF，真实音频候选混音、设备恢复、可听 PTS 主时钟和编码器/网络门禁继续按计划待验收。
- v33 发布候选根目录 15 个文件、1,305,608 bytes，`GpAutoLive.exe` 162,816 bytes；外置媒体 staging 树 13 个资源文件、352,365,694 bytes，5/5 资源大小与 SHA-256 全部匹配。4 秒基线生成 7 个样本，私有工作集峰值 97,173,504 bytes、工作集峰值 160,325,632 bytes、CPU 峰值约 7.44%；启动关闭冒烟退出码 0、无媒体进程残留。
- 新增会话边界测试覆盖声音关闭、无声音源、关闭后拒绝启动和未启动停止幂等；当前全量自动化测试为 257 项通过（Contracts 13、Core 45、Media 79、Windows 99、App 21）。
- v34（含最终 PCM 总线与实时 `-re` 解码）发布候选根目录 15 个文件、1,305,608 bytes，`GpAutoLive.exe` 162,816 bytes；外置媒体 staging 树 13 个资源文件、352,365,694 bytes，5/5 资源大小与 SHA-256 全部匹配。4 秒基线生成 7 个样本，私有工作集峰值 96,960,512 bytes、工作集峰值 159,940,608 bytes、CPU 峰值约 5.97%；启动关闭冒烟退出码 0、无媒体进程残留。
- v35（含 RTMP 声音/音画独立会话与 WPF 生命周期保护）发布候选根目录 15 个文件、1,324,624 bytes，`GpAutoLive.exe` 162,816 bytes；外置媒体 staging 树 13 个资源文件、352,365,694 bytes，5/5 资源大小与 SHA-256 全部匹配。4 秒基线生成 7 个样本，私有工作集峰值 110,788,608 bytes、工作集峰值 169,103,360 bytes、CPU 峰值约 8.76%；启动关闭冒烟标题为 `GpAutoLive`、退出码 0，私有工作集 83,922,944 bytes、工作集 144,990,208 bytes、21 线程、1,099 句柄，无 `GpAutoLive/mpv/ffmpeg/ffprobe` 残留。该样本为同机启动基线，不等价于 RTMP 网络发布或 30 分钟性能达标。

## 34. 当前实施增量：纯音频 EOF 顺序播放（2026-09-02）

- `MainWindow` 为纯音频会话增加单一完成观察者；只有控制器终态为 `Completed`、播放池仍为 `Playing` 且四维 `MediaPlaybackIdentity` 完全匹配时，才允许提交 `CompleteCurrent`。
- 多项播放按媒体池顺序推进，末项回到首项；单项通过 `CompleteCurrent` 递增 `loop_index` 后重新解码。下一项如果是视频则停止并明确提示，不让播放池状态先于真实输出。
- 手动停止、上一项/下一项、窗口关闭或播放池身份变化会取消观察者并阻止迟到完成回调；下一项重新建立 FFmpeg→PCM→PortAudio 会话，不复用旧解码进程。
- 本增量没有新增程序集或无界队列；现有 256 项测试继续通过。多项混合媒体、设备丢失恢复、N/N+1 真实预载、交叉淡化和可听时钟仍需 Windows 实机门禁。
- v29 发布候选根目录 15 个文件、1,294,032 bytes，`GpAutoLive.exe` 162,816 bytes；外置媒体 staging 树 28 个文件、353,659,726 bytes，5/5 资源大小与 SHA-256 全部匹配。启动约 3 秒后正常退出、无残留媒体进程；4 秒基线 6 个样本，私有工作集 80,498,688～83,357,696 bytes、工作集 136,839,168～144,465,920 bytes、CPU 峰值约 4.48%。

## 44. 当前实施增量：PortAudio 健康探测与有界设备恢复（2026-09-03）

- `WindowsPortAudioNative` 对 `Pa_IsStreamActive` 与 `Pa_IsStreamStopped` 使用可选动态导出；固定资源中的完整 PortAudio v19 会启用探测，缺少导出的旧版 DLL 返回 `Unknown`，不会把兼容性差异误报为设备故障。
- `WindowsPortAudioOutputSnapshot` 新增硬件状态、回调计数和最后一次原生状态旗标；回调继续使用预分配数组和固定容量环缓，不增加文件、网络或无界分配。
- 新增 `WindowsPortAudioOutputRecovery` 与 `WindowsPortAudioRecoveryPolicy`。非暂停会话观察到 `Stopped`、`Inactive` 或 `QueryError` 后，按最多 3 次、250/500/1000ms 退避重开同一设备配置；恢复失败取消解码并报告 `audio_device_lost`，不会无限重试或让 UI 停留在“播放中”。
- `WindowsAudioPlaybackController` 将健康观察器纳入会话所有权和停止 Join；暂停、自然完成、手动停止、切源和窗口关闭均会取消观察器，恢复操作使用同一 PortAudio 输出对象和 PCM 环缓。
- 新增健康快照、退避策略和真实 FFmpeg→PortAudio 会话回归；全量自动化测试为 **267 项通过**（Contracts 13、Core 45、Media 84、Windows 104、App 21）。Release 构建 0 警告/0 错误，`dotnet format --verify-no-changes` 通过。
- 显式 v44 外置资源上的真实 PortAudio 输出夹具通过；本轮验证证明健康观察器不会破坏暂停/恢复、有限音轨排空和会话清理，但尚未模拟真实拔出、睡眠唤醒、驱动重置或多声卡重新枚举，因此能力继续标记“代码已接入·待真实设备验收”。

## 45. 当前实施增量：v44 故障终态一致性与发布复验（2026-09-03）

- 修复 PortAudio 健康恢复耗尽后的 WPF 状态滞留：音频完成观察者现在区分 `Completed` 与 `Failed`，对 `audio_device_lost` 做四维身份校验、停止音频会话、停止播放池并更新设备状态，避免真实输出已停止而界面仍显示“播放中”。
- v44 正式安装候选为 `artifacts/csharp-windows-controller-20260903-v44`：根目录 8 个运行文件、1,037,031 bytes；符号包 `artifacts/csharp-windows-symbols-20260903-v44` 为 7 个文件、324,989 bytes；外置媒体运行时保持 13 个文件、352,365,694 bytes。
- v44 资源清单 5/5 大小与 SHA-256 校验通过；窗口标题 `GpAutoLive`，`CloseMainWindow`/等待退出成功，退出码 0，无 `GpAutoLive/mpv/ffmpeg/ffprobe` 残留；一次清洁冒烟采样私有字节 83,402,752、工作集 143,843,328、22 线程、1,103 句柄。4 秒空闲基线 6 个样本：私有工作集峰值 83,374,080 bytes、工作集峰值 144,965,632 bytes、CPU 峰值 2.32%。
- v44 外置资源上的真实 mpv `Original/Cpu4/Gpu83` 播放时间/EOF 夹具和 FFmpeg→PortAudio 健康观察夹具均通过；全量自动化测试 267 项、格式检查、`desktop/` 作用域检查均通过。真实拔插/睡眠唤醒、目标 GPU/声卡矩阵、ZLMediaKit/RTMPS、AkVirtualCamera、抖音 M1 和 30 分钟长稳仍待验收。

## 46. 当前实施增量：WPF 设置与本地偏好（2026-09-03）

- 新增 `SettingsWindow.xaml[.cs]`，标题栏齿轮和 `Ctrl+,` 共用单实例模态入口；窗口只编辑低敏感 UI 偏好，提供键盘 Enter 保存、Esc 取消和 UI Automation 名称。
- `UserPreferences.WithUiSettings` 在 Core 统一完成主题、语言和输出模式白名单校验/规范化；`DesktopSettingsDraft` 隔离编辑态，设置窗口不接触媒体路径、RTMP 地址、音频内容或凭据。
- 性能采样偏好默认保持开启；关闭后停止 1Hz `DispatcherTimer` 和采样任务。快速参数卡展开偏好保存后即时应用；下次输出入口仅记忆 `preview`/`rtmp`/`virtual_camera`，不会自动启动外部输出，主窗口标题旁会显示已记忆入口。
- 设置和主窗口几何沿同一 `IniUserPreferencesStore` 原子写入边界保存；保存失败返回脱敏提示，不取消当前媒体会话。主题/语言暂固定为深色/简体中文，避免展示尚未实现的即时切换。
- 新增 Core/App 设置边界测试；全量自动化测试为 **271 项通过**（Contracts 13、Core 47、Media 84、Windows 104、App 23）。Release 构建 0 警告/0 错误，`dotnet format --verify-no-changes --no-restore` 与 `tools/verify-scope.ps1` 均通过。
- v45 正式安装候选为 `artifacts/csharp-windows-controller-20260903-v45`：根目录 8 个运行文件、1,050,343 bytes；符号包 `artifacts/csharp-windows-symbols-20260903-v45` 为 7 个文件、331,566 bytes；外置媒体运行时仍为 13 个文件、352,365,694 bytes，清单 5/5 哈希匹配。
- v45 启动关闭冒烟成功（私有工作集 96,739,328 bytes、工作集 158,756,864 bytes、21 线程、1,096 句柄，退出码 0、无残留进程）；4 秒空闲基线 4 个样本，私有工作集峰值 82,419,712 bytes、工作集峰值 142,786,560 bytes、CPU 峰值 0.89%。真实 mpv `Original/Cpu4/Gpu83` 夹具各 1/1 通过，FFmpeg→PortAudio 健康会话 7/7 通过。真实设备拔插/睡眠唤醒、目标 GPU/声卡矩阵、ZLMediaKit/RTMPS、AkVirtualCamera、抖音 M1 和 30 分钟长稳仍待验收。
## 47. 当前实施增量：WPF 视频播放进度与绝对 seek（2026-09-03）

- `MainWindow` 为视频源显示当前位置/时长，并将受当前四维 `MediaPlaybackIdentity` 保护的 mpv `time-pos` 映射到 `PlaybackProgress`；源切换、播放池修订、停止和音频源会清除旧投影，避免迟到观察污染新媒体。
- 视频源具有合法 `DurationMs` 时启用进度条；鼠标释放和方向键/Home/End 走同一串行播放命令，按 `0..1` 比例计算有界毫秒值，调用 `WindowsMpvPlaybackController.SeekAsync`。控制器/会话仍负责 IPC、身份校验和错误分类，WPF 不拼接命令行。
- 新增 `PlaybackTimeFormatter` 纯逻辑边界与 2 项测试；当前全量自动化测试为 **273 项通过**（Contracts 13、Core 47、Media 84、Windows 104、App 25）。
- v46 发布候选已包含该接线：正式安装根目录 8 个运行文件、1,054,439 bytes；PDB/XML 独立符号包 7 个文件、332,758 bytes；媒体运行时 13 个文件、352,365,694 bytes。资源清单 runtime 1.0.0 的 5/5 大小与 SHA-256 匹配；真实 mpv `Original/Cpu4/Gpu83` 各 1/1、FFmpeg→PortAudio 健康会话 1/1、启动关闭和 4 秒性能基线均通过。
- 本增量仍不把音频源伪装成可 seek；真实首帧像素/暂停恢复、目标 GPU/声卡矩阵、设备热插拔/睡眠唤醒、RTMP/RTMPS 网络、AkVirtualCamera、抖音 M1 与 30 分钟长稳继续按总计划门禁执行。
- v46 最终复验重新采样：启动关闭冒烟私有工作集 82,235,392 bytes、工作集 139,481,088 bytes、22 线程、1,099 句柄，退出码 0 且无残留；4 秒空闲基线 3 个样本峰值私有工作集 97,497,088 bytes、工作集峰值 158,420,992 bytes、CPU 峰值 3.68%。

详见 [`C8 WPF 视频播放进度与 seek 实现记录`](./C8-WPF视频播放进度与seek实现记录.md)。

## 48. 当前实施增量：WPF 麦克风本地能量门控接入（2026-09-03）

- `WindowsMicrophoneInterludeController` 组合已校验 PortAudio 输入流、固定容量 PCM 环缓、`MicrophoneInterludeGate` 和共享 `AudioPriorityCoordinator`；只发布状态、电平和计数快照，不发布 PCM 正文。
- WPF“声音与互动”卡片现在同时枚举输入/输出设备。输入流只在用户点击“启用门控”后启动，停止、登录退出和窗口关闭均有取消、2 秒 Join 和资源释放；无效 DLL、未选设备和关闭后启动均 fail-closed。
- 门控进入 `Speaking/Hangover` 时标记麦克风优先级，必要时取消固定话术；UI 明确显示“仅本地能量门控，不识别/上传”，不把尚未实现的完整麦克风插话伪装成已完成。
- 新增 3 项 Windows 控制器测试；当前全量自动化测试为 **276 项通过**（Contracts 13、Core 47、Media 84、Windows 107、App 25）。
- v47 正式安装候选根目录 8 个运行文件、1,074,919 bytes，符号包 7 个文件、336,906 bytes，外置媒体运行时 13 个文件、352,365,694 bytes；清单 5/5 哈希匹配，启动关闭冒烟退出码 0 且无残留；4 秒空闲基线 3 个样本峰值私有工作集 83,099,648 bytes、工作集峰值 142,663,680 bytes、CPU 峰值 1.79%。
- 详见 [`C9 WPF 麦克风本地能量门控实现记录`](./C9-WPF麦克风本地门控实现记录.md)。
- AEC/降噪/AGC、完整 VAD、可听 PCM 混音、设备拔插/睡眠唤醒和真实声卡门禁仍待实施/验收；抖音 M1、AkVirtualCamera、真实 RTMP/RTMPS、目标 GPU 矩阵和 30 分钟长稳继续保持原计划状态。

## 49. 当前实施增量：基础媒体音频优先级输出门控（2026-09-03）

- `AudioPcmMixer.TryApplyBasePolicy` 在解码线程分片写入固定容量环缓/最终 PCM 总线前原地应用基础媒体策略：固定话术或麦克风优先时静音，插话层级时降低 6 dB，普通状态保持 0 dB。
- `WindowsAudioPlaybackController` 与 `WindowsRtmpAudioSession` 共享同一个策略提供器；不引入第二份音频源，PortAudio 原生回调仍保持无分配、无业务锁。
- 新增 2 项混音边界测试；当前全量自动化测试为 **278 项通过**（Contracts 13、Core 47、Media 86、Windows 107、App 25）。
- v48 正式安装候选根目录 8 个运行文件、1,075,431 bytes，符号包 7 个文件、336,290 bytes，外置媒体运行时 13 个文件、352,365,694 bytes；清单 5/5 哈希匹配，启动关闭冒烟退出码 0 且无残留；4 秒空闲基线 3 个样本峰值私有工作集 82,997,248 bytes、工作集峰值 142,548,992 bytes、CPU 峰值 1.42%。
- 本增量不宣称视频 mpv 内部声音、插话候选池、麦克风 PCM 可听混音、AEC/降噪/AGC/完整 VAD 或可听 PTS 已完成；真实设备与网络门禁继续按计划执行。
- 详见 [`C10 WPF 音频优先级输出门控实现记录`](./C10-WPF音频优先级输出门控实现记录.md)。

## 50. 当前实施增量：WPF 递归插话文件池（2026-09-03）

- `GpAutoLive.Contracts/InterludeContracts.cs` 固定插话池边界：最多 1,000 个候选文件、单路径 UTF-8 4 KiB、快照路径总量 512 KiB，扩展名复用现有 17 种视频/音频媒体集合。
- `GpAutoLive.Core/InterludeFilePoolService.cs` 是插话目录快照的唯一所有者，使用有界 DFS 递归扫描；跳过 ReparsePoint，规范化根目录和文件路径，按规范化路径稳定排序。目录项访问失败只跳过该项，根目录失败则返回脱敏错误。
- 扫描完成前不修改快照；路径超限、候选超过 1,000、取消或根目录读取失败均保持旧快照。清空只清除内存快照，不删除磁盘文件。
- WPF“声音与互动”卡片增加“插话文件池（递归扫描）”目录选择、候选数量投影和清空入口；目录扫描在后台线程执行，窗口关闭或取消时有界退出，不启动 FFmpeg/PortAudio。
- 新增 5 项 Core 插话池测试；当前全量自动化测试为 **286 项通过**（Contracts 13、Core 52、Media 89、Windows 107、App 25）。
- 本增量仅完成 Rust/Tauri `interlude_player` 的目录池边界复刻；22 套声音预设、随机周期、duck 曲线、PCM 多轨/可听插话混音、固定话术/麦克风抢占恢复仍待后续真实音频门禁，不能标记为已接入。
- v49 正式安装候选为 `artifacts/csharp-windows-controller-20260903-v49`：根目录 8 个运行文件、1,090,791 bytes，`GpAutoLive.exe` 162,816 bytes；符号包 `artifacts/csharp-windows-symbols-20260903-v49` 为 7 个文件、347,032 bytes；外置媒体运行时 13 个文件、352,365,694 bytes，资源清单 5/5 大小与 SHA-256 匹配。最终启动关闭冒烟 `CloseMainWindow=True`、等待退出成功、退出码 0，无残留媒体进程；一次采样私有工作集 82,120,704 bytes、工作集 138,227,712 bytes、22 线程、1,078 句柄。最终 4 秒空闲基线 3 个样本：私有工作集峰值 83,283,968 bytes、工作集峰值 142,831,616 bytes、CPU 峰值 1.52%。

详见 [`C11 WPF 插话文件池实现记录`](./C11-WPF插话文件池实现记录.md)。

## 51. 当前实施增量：WPF 插话 PCM 混音输出（2026-09-03）

- `GpAutoLive.Media/AudioPcmOutputSource.cs` 将基础轨与插话轨收敛到一个有界输出源：使用固定 scratch 缓冲、同一 PortAudio 回调和既有 `AudioPcmMixer`，不建立第二条输出流、不把整段音频载入内存。
- `FinalPcmBus` 增加本机/RTMP 两组插话环缓；`WindowsFfmpegPcmDecoder` 的 `finalPcmOverlay` 只向插话消费者发布 PCM，普通声音解码合同保持不变。
- `WindowsAudioPlaybackController` 增加单插话任务的启动、停止、取消、暂停门控和有界 Join；主会话结束时先回收插话再关闭最终 PCM 总线。混音模式只在输出源应用基础策略，避免解码线程重复 duck。
- WPF 声音与互动卡片增加“插话试播/停插话”：目录快照首项在正在播放/暂停的纯音频项或 RTMP 声音会话上叠加；固定话术、麦克风说话、停止/切换媒体和清空插话池均会停止插话。插话完成后释放层级，主音频不自动重播。
- `WindowsRtmpAudioSession` 与 `WindowsRtmpFinalPcmPump` 同样消费混音输出源，RTMP 声音不会再另建插话队列或重复读取主媒体；网络/编码器实机门禁仍独立管理。
- 本轮新增 `AudioPcmMixingOutputSource`、最终总线插话消费者、PortAudio/RTMP 控制器插话生命周期和 WPF 入口测试；当前全量自动化测试为 **291 项通过**（Contracts 13、Core 52、Media 90、Windows 111、App 25）。
- 本轮仍不宣称视频 mpv 内部声音、22 套预设/随机周期、attack/release 曲线、麦克风可听 PCM、AEC/降噪/AGC 或真实声卡/RTMP 网络/30 分钟门禁完成；这些继续按“正式需求·待实施/未验收”管理。
- 详见 [`C12 WPF 插话 PCM 混音实现记录`](./C12-WPF插话PCM混音实现记录.md)。
- v50 发布复验：`artifacts/csharp-windows-controller-20260903-v50` 根目录 8 个文件、1,114,855 bytes，`GpAutoLive.exe` 162,816 bytes；符号包 7 个文件、353,084 bytes；外置媒体运行时 13 个硬链接文件、352,365,694 bytes，5/5 哈希匹配。使用仓库 `.tools/dotnet` 启动并关闭成功，退出码 0、无残留媒体进程；5 秒空闲基线 3 个样本峰值私有工作集 97,406,976 bytes、工作集峰值 159,248,384 bytes、CPU 2.65%。与前次同机采样存在启动时序波动；发布包为 framework-dependent，目标机需 .NET 10 Desktop Runtime，该基线不等价于 30 分钟性能门禁。

## 52. 当前实施增量：WPF 插话声音配置与预设选择（2026-09-03）

- `InterludeAudioConfig` 对齐 Rust/Tauri `InterludeConfig` 的 JSON 字段和范围：p01～p22 白名单、固定/随机选择、最多 4 条多轨、每次/周期变化、间隔、音量、duck、attack/release。
- `InterludeAudioSelector` 只在控制线程完成有界随机选择和周期复用；固定种子可测试，实时 PortAudio/RTMP 回调不调用随机或分配逻辑。
- `InterludeAudioConfigStore` 使用现有版本化 JSON 原子存储，写入 `%LocalAppData%\\GpAutoLive\\profiles\\interlude\\default.json`；WPF 目录选择/清空同步目录配置，音频正文和凭据不落盘。
- `CreateBaseAudioMixPolicy` 使用配置音量与 duck 深度；状态栏展示当前预设选择策略，并明确标注“DSP 待验收”。
- 本轮新增 9 项 Contracts/Core/Media 测试；当前全量自动化测试为 **301 项通过**（Contracts 16、Core 58、Media 91、Windows 111、App 25）。
- 22 套预设实际滤波映射、预设声音变化是否可听、真实声卡和 30 分钟长稳仍保持“正式需求·待实施/未验收”；固定 PCM 输出源的有界 attack/release 线性过渡已接入，但仍待实机听感门禁。
- 详见 [`C13 WPF 插话声音配置与预设选择实现记录`](./C13-WPF插话声音配置与预设选择实现记录.md)。
- v52 发布候选：`artifacts/csharp-windows-controller-20260903-v52` 根目录 8 个文件、1,140,455 bytes；独立符号包 `artifacts/csharp-windows-symbols-20260903-v52` 为 7 个文件、371,690 bytes；外置媒体运行时复用 v50 的 13 个硬链接文件、352,365,694 bytes，5/5 大小与 SHA-256 匹配。5 秒空闲基线 4 个样本峰值私有工作集 98,508,800 bytes、工作集 161,988,608 bytes、CPU 峰值 4.85%；使用仓库 `.tools/dotnet` 启动并关闭成功且无残留媒体进程。该样本存在启动时序波动，不等价于 30 分钟性能门禁。
- v53 发布候选：`artifacts/csharp-windows-controller-20260903-v53` 根目录 8 个文件、1,140,967 bytes；独立符号包 `artifacts/csharp-windows-symbols-20260903-v53` 为 7 个文件、372,662 bytes；外置媒体运行时复用 v50 的 13 个硬链接文件、352,365,694 bytes，5/5 大小与 SHA-256 匹配。5 秒空闲基线 4 个样本峰值私有工作集 98,447,360 bytes、工作集 161,882,112 bytes、CPU 峰值 3.68%；使用仓库 `.tools/dotnet` 启动并关闭成功且无残留媒体进程。该样本存在启动时序波动，不等价于 30 分钟性能门禁。
- v54 发布候选：`artifacts/csharp-windows-controller-20260903-v54` 根目录 8 个文件、1,140,967 bytes；独立符号包 `artifacts/csharp-windows-symbols-20260903-v54` 为 7 个文件、371,850 bytes；外置媒体运行时复用 v50 的 13 个硬链接文件、352,365,694 bytes，5/5 大小与 SHA-256 匹配。5 秒空闲基线 4 个样本峰值私有工作集 98,762,752 bytes、工作集 162,676,736 bytes、CPU 峰值 4.07%；使用仓库 `.tools/dotnet` 启动并关闭成功且无残留媒体进程。该样本存在启动时序波动，不等价于 30 分钟性能门禁。

## 53. 当前实施增量：AkVirtualCamera 契约与输出状态边界（2026-09-03）

- `GpAutoLive.Contracts/VirtualCameraContracts.cs` 对齐 Rust/Tauri 虚拟摄像头核心合同：固定 `GpAutoLive Camera`、YUY2、1280×720@30fps、`zero_copy=false`，并拒绝尺寸、传输、WARP、CPU 缩放或 CPU 色彩转换等不符合门禁的事实。
- `GpAutoLive.Core/VirtualCameraOutputManager.cs` 提供纯逻辑唯一所有者：`Unavailable → Installed → Starting → Ready → Streaming`，支持有界恢复、停止、代际失效、下游客户端数量、容量 1 latest-wins、过期帧拒绝、YUY2 黑帧策略和最近 512 个回读样本的平均/P50/P95/P99 指标。
- WPF “GpAutoLive Camera” 卡片现展示当前状态和待验收事实；没有注册设备、启动 GPL sidecar 或执行未验证 WGC/D3D11 捕获，避免把契约接入误报为虚拟摄像头可用。
- 新增 Contracts 5 项、Core 6 项测试；当前全量自动化测试为 **312 项通过**（Contracts 21、Core 64、Media 91、Windows 111、App 25）。
- 真实 WGC/D3D11、一次有界 GPU→CPU 回读、AkVirtualCamera IPC/DirectShow、签名/许可证、下游应用兼容和 GPU 长稳继续按专项方案保持“正式需求·待实施/未验收”。
- 详见 [`C14 AkVirtualCamera 契约与输出状态实现记录`](./C14-AkVirtualCamera契约与输出状态实现记录.md)。
- v55 发布复验：`artifacts/csharp-windows-controller-20260903-v55` 根目录 8 个文件、1,174,247 bytes；独立符号包 `artifacts/csharp-windows-symbols-20260903-v55` 为 7 个文件、394,365 bytes；外置媒体运行时复用 v54 的 13 个硬链接文件、352,365,694 bytes，5/5 大小与 SHA-256 匹配。使用锁定 `.tools/dotnet` 启动/关闭成功，退出码 0 且无 `GpAutoLive/mpv/ffmpeg/ffprobe` 残留；5 秒空闲基线 4 个样本峰值私有工作集 84,037,632 bytes、工作集 145,567,744 bytes、CPU 峰值 1.90%，不等价于 30 分钟 GPU 长稳门禁。

## 54. 当前实施增量：AkVirtualCamera sidecar 固定协议（2026-09-03）

- `GpAutoLive.Windows/WindowsVirtualCameraSidecarProtocol.cs` 对齐 Rust/Tauri sidecar 协议：协议版本 1、`GPAKVC01` magic、52 字节固定帧头、固定 YUY2 `1280×720×2` payload 和 little-endian 字段。
- `TryEncode` 写入调用方提供的固定 `Span<byte>`，不在编码路径创建第二份完整帧；`TryDecode` 在固定长度完整到达后才复制 payload，并返回已消费字节数，避免按不可信长度无界扩容。
- Named Pipe 名称只允许固定前缀加 16 字节非零随机会话令牌的 32 位十六进制后缀；协议层不接受任意 IPC 名称，不启动 sidecar 或创建系统设备。
- 新增 Windows 5 项协议测试；当前全量自动化测试为 **317 项通过**（Contracts 21、Core 64、Media 91、Windows 116、App 25）。
- 真实 Named Pipe ACL、WGC/D3D11、AkVirtualCamera DirectShow、sidecar Job Object、签名/许可证和下游兼容仍保持“正式需求·待实施/未验收”。
- 详见 [`C15 AkVirtualCamera sidecar 协议实现记录`](./C15-AkVirtualCamera-sidecar协议实现记录.md)。
- v56 发布复验：`artifacts/csharp-windows-controller-20260903-v56` 根目录 8 个文件、1,188,583 bytes；独立符号包 `artifacts/csharp-windows-symbols-20260903-v56` 为 7 个文件、396,477 bytes；外置媒体运行时复用 v54 的 13 个硬链接文件、352,365,694 bytes，5/5 大小与 SHA-256 匹配。使用锁定 `.tools/dotnet` 启动/关闭成功，退出码 0 且无 `GpAutoLive/mpv/ffmpeg/ffprobe` 残留；5 秒空闲基线 4 个样本峰值私有工作集 84,017,152 bytes、工作集 145,698,816 bytes、CPU 峰值 1.76%，不等价于 30 分钟 GPU 长稳门禁。

## 55. 当前实施增量：抖音 M1 本地合同与有界队列（2026-09-03）

- `GpAutoLive.Contracts/DouyinLiveContracts.cs` 建立本地配置、状态、最小 `WebcastChatMessage` 映射、回复任务、队列统计和脱敏状态快照；房间号、回复池、Unicode/UTF-8 长度、控制字符和队列容量均在边界内规范化，重复回复按顺序静默去重。
- `GpAutoLive.Core/DouyinLiveManager.cs` 是会话、代际、去重集合、随机选句和有界串行任务队列的唯一所有者：只在 `Listening` 接受消息，过滤本账号回显/重放/重复 ID，去重集合最多 4096 条；满队列丢最旧未发送项，任务等待严格超过 60 秒才过期。
- 发送终态固定为 `accepted/not_sent/rejected/outcome_unknown`；未知结果不自动重试。停止/失败清空内存中的任务和去重集合并提升代际，暂停后才允许调整 10～5000 的队列容量。
- WPF 只显示脱敏 M1 状态，并明确“扫码/sidecar 待验收”；没有 QR 登录、平台网络、模型调用、Go 转发或抖音凭据持久化。
- 新增 Contracts 4 项、Core 7 项测试；当前全量自动化测试为 **328 项通过**（Contracts 25、Core 71、Media 91、Windows 116、App 25）。Release 构建 0 警告/0 错误，格式检查和 `desktop/` 作用域检查通过。
- v57 发布复验：`artifacts/csharp-windows-controller-20260903-v57` 根目录 8 个文件、1,220,839 bytes；独立符号包 `artifacts/csharp-windows-symbols-20260903-v57` 为 7 个文件、417,732 bytes；外置媒体运行时复用 v56 的 13 个硬链接文件、352,365,694 bytes，5/5 大小与 SHA-256 匹配。使用锁定 `.tools/dotnet` 启动/关闭成功，退出码 0 且无 `GpAutoLive/mpv/ffmpeg/ffprobe` 残留；5 秒空闲基线 4 个样本峰值私有工作集 84,275,200 bytes、工作集 145,805,312 bytes、CPU 峰值 1.52%，不等价于真实抖音或 30 分钟媒体长稳门禁。
- 真实 QR/协议兼容、受管 Python sidecar、单条发送限频、自回显确认、网络恢复、账号安全、许可和长稳仍保持“正式需求·待实施/未验收”。详见 [`C16 抖音 M1 本地合同与有界队列实现记录`](./C16-抖音M1本地合同与有界队列实现记录.md)。

## 56. 当前实施增量：WPF 抖音 M1 本地配置与生命周期入口（2026-09-03）

- “声音与互动”卡片新增本地自动回应开关、直播间号、逐行回复池和队列容量；编辑态不写入凭据或服务端，回复正文只存在当前进程内存。
- 新增 `Features/Douyin/DouyinConfigDraft`，复用合同层校验完成文本拆分、空行处理、重复回复规范化和队列整数解析，页面不重复维护业务范围。
- 启动只进入本地 `WaitingQr`，暂停后可修改队列容量，恢复时提交容量；停止、登录失效和窗口关闭均清理 M1 队列与去重状态。按钮不会创建网络连接、二维码、sidecar 或发送任务。
- 新增 App 4 项测试；当前全量自动化测试为 **332 项通过**（Contracts 25、Core 71、Media 91、Windows 116、App 29）。Release 构建 0 警告/0 错误，格式检查和 `desktop/` 作用域检查通过。
- v58 发布复验：`artifacts/csharp-windows-controller-20260903-v58` 根目录 8 个文件、1,225,959 bytes；独立符号包 `artifacts/csharp-windows-symbols-20260903-v58` 为 7 个文件、420,368 bytes；外置媒体运行时复用 v57 的 13 个硬链接文件、352,365,694 bytes，5/5 大小与 SHA-256 匹配。使用锁定 `.tools/dotnet` 直接启动/关闭成功，退出码 0 且无 `GpAutoLive/mpv/ffmpeg/ffprobe` 残留；5 秒空闲基线 4 个样本峰值私有工作集 85,204,992 bytes、工作集 146,878,464 bytes、CPU 峰值 1.65%，不等价于真实抖音或 30 分钟媒体长稳门禁。真实 QR/协议 sidecar、平台发送/自回显、AkVirtualCamera 实机链路、目标 GPU/声卡矩阵和长稳仍保持“正式需求·待实施/未验收”。
- 详见 [`C17 WPF 抖音 M1 本地配置与生命周期入口实现记录`](./C17-WPF抖音M1本地配置与生命周期入口实现记录.md)。

## 57. 当前实施增量：抖音 M1 非敏感配置 JSON 存储（2026-09-03）

- 新增 `GpAutoLive.Core/Configuration/DouyinLiveConfigStore.cs`，复用现有 `VersionedJsonStore<T>` 的版本封套、64 KiB 上限、敏感字段保护和原子写入，不新增第二套配置基础设施。
- 固定写入 `%LocalAppData%\\GpAutoLive\\profiles\\douyin\\default.json`，只保存启用标志、规范化房间号、去重后的本地回复池和队列容量；Cookie、Token、二维码、登录态和平台正文不落盘。
- WPF 启动时异步读取配置，M1 启动成功后原子保存；读取/保存失败只显示脱敏提示，不阻塞窗口或取消当前内存会话。
- 新增 Core 2 项配置存储测试；当前全量自动化测试为 **334 项通过**（Contracts 25、Core 73、Media 91、Windows 116、App 29）。Release 构建 0 警告/0 错误，格式检查和 `desktop/` 作用域检查通过。
- v59 发布复验：`artifacts/csharp-windows-controller-20260903-v59` 根目录 8 个文件、1,230,055 bytes；独立符号包 `artifacts/csharp-windows-symbols-20260903-v59` 为 7 个文件、421,711 bytes；外置媒体运行时复用 v58 的 13 个硬链接文件、352,365,694 bytes，资源清单 5/5 大小与 SHA-256 匹配。锁定 `.tools/dotnet` 启动/关闭成功，退出码 0 且无残留；5 秒空闲基线 4 个样本峰值私有工作集 85,856,256 bytes、工作集 146,931,712 bytes、CPU 峰值 1.92%。
- 真实 QR/协议 sidecar、平台发送/回显确认、账号安全、AkVirtualCamera 实机链路、目标 GPU/声卡矩阵和 30 分钟长稳仍保持“正式需求·待实施/未验收”。详见 [`C18 抖音 M1 非敏感配置 JSON 存储实现记录`](./C18-抖音M1非敏感配置JSON存储实现记录.md)。

## 58. 当前实施增量：Windows 抖音探针 Conda 启动计划（2026-09-03）

- 新增 `GpAutoLive.Windows/WindowsDouyinProbeLaunchPlan.cs`，构造 `conda run --no-capture-output -n <env> python <script>` 的参数数组，复用 `ExternalProcessPlan` 和既有 `HiddenNoShellProcessTree` 策略。
- 启动计划校验上游 `Douyin_Spider` 必需文件、Conda/脚本绝对普通文件、环境名白名单、M1 房间号/回复池、30～900 秒整数超时及已有目录下的 PNG 二维码路径。
- stdout/stderr 分别限制 256 KiB/64 KiB；本轮不启动进程、不连接抖音、不读取凭据，仅交付可测试的 Windows 安全启动边界。
- 新增 Windows 4 项测试；当前全量自动化测试为 **338 项通过**（Contracts 25、Core 73、Media 91、Windows 120、App 29）。Release 构建 0 警告/0 错误，格式检查和 `desktop/` 作用域检查通过。
- v60 发布复验：`artifacts/csharp-windows-controller-20260903-v60` 根目录 8 个文件、1,236,199 bytes；独立符号包 `artifacts/csharp-windows-symbols-20260903-v60` 为 7 个文件、423,211 bytes；外置媒体运行时复用 v59 的 13 个硬链接文件、352,365,694 bytes，资源清单 5/5 大小与 SHA-256 匹配。锁定 `.tools/dotnet` 启动/关闭成功，退出码 0 且无残留；5 秒空闲基线 4 个样本峰值私有工作集 85,921,792 bytes、工作集 147,005,440 bytes、CPU 峰值 1.52%。
- 真实 Conda 环境、Python sidecar 运行、QR/协议兼容、平台发送/回显确认、账号安全、AkVirtualCamera 实机链路、目标 GPU/声卡矩阵和长稳仍保持“正式需求·待实施/未验收”。详见 [`C19 Windows 抖音探针 Conda 启动计划实现记录`](./C19-Windows抖音探针Conda启动计划实现记录.md)。

## 59. 当前实施增量：Windows 抖音 sidecar 受管宿主与事件桥接（2026-09-03）

- 新增 `GpAutoLive.Windows/WindowsDouyinProbeEventParser.cs`：固定事件白名单、16 KiB 单行上限和 fail-closed JSON 解析；不返回弹幕正文、Cookie、Token 或异常正文。
- 新增 `GpAutoLive.Windows/WindowsDouyinProbeHost.cs`：消费 C19 `ExternalProcessPlan`，以隐藏窗口、双流重定向、有限输出、取消/超时和 Job Object 优先的进程树清理运行 Conda sidecar。
- sidecar 事件桥接到 `DouyinLiveManager` 的扫码、登录、房间解析、公屏连接、外部弹幕观察、回复尝试、自回显过滤、通过/证据不足/失败终态；`DouyinLiveState.Inconclusive` 与 WPF 文案/编辑门禁同步。
- WPF “启动/停止 M1” 在显式配置 `AUTOLIVE_DOUYIN_ROOT`、`AUTOLIVE_DOUYIN_PROBE`、`CONDA_EXE` 时进入受管 sidecar；变量未配置时继续使用无网络的本地合同模式。
- 无效计划不会改变核心状态；自然退出但没有 `probe_passed` 标记证据不足，超时、输出越界、读取失败和失败事件均不报告成功；停止/释放清理内存队列与去重集合。
- 新增 Windows 事件解析/宿主/事件桥接边界测试、Core sidecar 终态测试和 App 环境变量工厂测试；全量自动化测试为 **350 项通过**（Contracts 25、Core 75、Media 91、Windows 126、App 33）。
- `dotnet build` 0 警告/0 错误；`dotnet format --verify-no-changes` 与 `tools/verify-scope.ps1` 均通过。
- v63 发布复验：正式候选 `artifacts/csharp-windows-controller-20260903-v63` 根目录 8 个运行文件、1,270,503 bytes，`GpAutoLive.exe` 162,816 bytes；独立符号包 7 个文件、432,779 bytes；外置媒体运行时 13 个文件、352,365,694 bytes（5 个二进制资源使用硬链接复用）；资源清单 5/5 大小与 SHA-256 匹配。
- v63 启动关闭冒烟使用锁定 `.tools/dotnet` 成功（`CloseMainWindow=True`、退出码 0、无 `GpAutoLive/mpv/ffmpeg/ffprobe` 残留）；5 秒空闲基线 6 个样本，私有工作集峰值 85,643,264 bytes、工作集峰值 148,094,976 bytes、CPU 峰值 1.54%。
- 真实 Conda 环境、上游 `Douyin_Spider`、QR 登录、WebSocket、平台发送、自回显、账号安全、许可和目标机长稳仍保持“正式需求·待实施/未验收”；`desktop/` 未修改。
- 详见 [`C20 Windows 抖音 sidecar 受管宿主与事件桥接实现记录`](./C20-Windows抖音sidecar受管宿主与事件桥接实现记录.md)。

## 60. 当前实施增量：sidecar stdout 有界读取修正与 v64 发布复验（2026-09-03）

- `WindowsDouyinProbeHost.ReadStdoutAsync` 改为固定字符缓冲和逐行有界组装；在收到换行前也不会允许超过 16 KiB 的行继续增长，完整行仍同时受 256 KiB 总 stdout 上限约束。
- 该修正不改变 sidecar 事件白名单、脱敏字段和进程树清理策略；新增代码复用既有取消、输出越界分类和 Job Object 回收路径。
- 全量自动化测试保持 **350 项通过**（Contracts 25、Core 75、Media 91、Windows 126、App 33）；Release 构建 0 警告/0 错误，格式检查和 `desktop/` 作用域检查通过。
- v64 发布复验：正式候选 `artifacts/csharp-windows-controller-20260903-v64` 根目录 8 个运行文件、1,270,503 bytes，`GpAutoLive.exe` 162,816 bytes；独立符号包 7 个文件、432,951 bytes；外置媒体运行时 13 个文件、352,365,694 bytes，5 个二进制资源使用硬链接复用，资源清单 5/5 大小与 SHA-256 匹配。
- v64 启动关闭冒烟使用锁定 `.tools/dotnet` 成功（`CloseMainWindow=True`、退出码 0、无 `GpAutoLive/mpv/ffmpeg/ffprobe` 残留）；5 秒空闲基线 6 个样本，私有工作集峰值 85,688,320 bytes、工作集峰值 148,262,912 bytes、CPU 峰值 2.32%。该数据仅是同机空闲对照，不等价于真实 sidecar、网络或 30 分钟长稳门禁。

## 61. 当前实施增量：Windows GDI/User 性能指标与 v65 发布复验（2026-09-03）

- `WindowsCurrentProcessPerformanceSource` 通过 Windows `GetGuiResources` 读取 GDI/User 对象计数；权限、进程句柄或 API 不可用时返回 `null`，不把缺失指标伪装成 0。
- `WindowsProcessPerformanceSample` 与 `WindowsProcessPerformanceSnapshot` 保持可选字段，采样上限仍为 1,000,000；WPF 性能短文本在计数可用时追加 GDI/User，基线脚本同步输出每样本及 min/max。
- 新增 GUI 资源计数有效性测试；全量自动化测试为 **351 项通过**（Contracts 25、Core 75、Media 91、Windows 127、App 33）。Release 构建 0 警告/0 错误，格式检查和 `desktop/` 作用域检查通过。
- v65 发布复验：正式候选 `artifacts/csharp-windows-controller-20260903-v65` 根目录 8 个运行文件、1,272,551 bytes，`GpAutoLive.exe` 162,816 bytes；独立符号包 7 个文件、433,327 bytes；外置媒体运行时 13 个文件、352,365,694 bytes，5 个二进制资源使用硬链接复用，资源清单 5/5 大小与 SHA-256 匹配。
- v65 启动关闭冒烟使用锁定 `.tools/dotnet` 成功（`CloseMainWindow=True`、退出码 0、无 `GpAutoLive/mpv/ffmpeg/ffprobe` 残留）；5 秒空闲基线 6 个样本，私有工作集峰值 86,007,808 bytes、工作集峰值 148,484,096 bytes、CPU 峰值 1.75%，GDI 对象 17、User 对象 40。该数据仅是同机空闲对照，不等价于 30 分钟性能门禁。

## 62. 当前实施增量：AkVirtualCamera Named Pipe 固定帧传输客户端与 v66 发布复验（2026-09-03）

- 新增 `WindowsVirtualCameraSidecarClient`：仅连接由上层/sidecar 创建的受控本机 Named Pipe，串行化 Connect/Write/Stop/Dispose 生命周期，复用单个 `ArrayPool<byte>` 固定帧缓冲；不启动 sidecar、不创建 ACL、不执行 WGC/D3D11 捕获。
- `VirtualCameraFrame` 的 90kHz 时间戳按 `×1000/9` 转为协议 100ns 时间戳；超范围、无效帧、不可信管道名、连接失败、写入超时和取消均返回稳定错误码，快照不暴露令牌/管道名/异常正文。
- 新增本机 Named Pipe 并发读写回环测试，覆盖大帧背压、资源回收和停止幂等；全量自动化测试为 **356 项通过**（Contracts 25、Core 75、Media 91、Windows 132、App 33）。Release 构建 0 警告/0 错误，格式检查和 `desktop/` 作用域检查通过。
- v66 发布复验：正式候选 `artifacts/csharp-windows-controller-20260903-v66` 根目录 8 个运行文件、1,285,863 bytes，`GpAutoLive.exe` 162,816 bytes；独立符号包 7 个文件、436,011 bytes；外置媒体运行时 13 个文件、352,365,694 bytes，5 个二进制资源使用硬链接复用，资源清单 5/5 大小与 SHA-256 匹配。
- v66 使用仓库锁定 `.tools/dotnet` 设置 `DOTNET_ROOT` 启动/关闭成功（`CloseMainWindow=True`、退出码 0、无 `GpAutoLive/mpv/ffmpeg/ffprobe` 残留）；5 秒空闲基线 4 个样本，私有工作集峰值 85,348,352 bytes、工作集峰值 146,055,168 bytes、CPU 峰值 1.65%，GDI 对象 17、User 对象 42。该数据仅是同机空闲对照，不等价于真实 sidecar、GPU 或 30 分钟长稳门禁。
- 真实 Named Pipe ACL、sidecar 进程、WGC/D3D11、AkVirtualCamera DirectShow、签名/许可证、目标 GPU 和 30 分钟长稳仍待验收。

## 63. 当前实施增量：应用内 GC 计数快照与 v67 发布复验（2026-09-03）

- `WindowsProcessPerformanceSample`/`WindowsProcessPerformanceSnapshot` 新增可选 Gen0、Gen1、Gen2 回收计数；Windows 当前进程源使用 `GC.CollectionCount` 读取，不把跨进程性能计数器或推算值写入基线。
- WPF 性能摘要在三个 GC 字段同时可用时追加 `GC gen0/gen1/gen2`；计数为负、运行时不可用或源异常时保持 `null`/不可用，原有内存、CPU、GDI/User 边界不变。
- 新增 GC 无效范围测试并扩展首样本、格式化和 Windows 源测试；全量自动化测试为 **357 项通过**（Contracts 25、Core 75、Media 91、Windows 133、App 33）。Release 构建 0 警告/0 错误，格式检查和 `desktop/` 作用域检查通过。
- v67 发布复验：正式候选 `artifacts/csharp-windows-controller-20260903-v67` 根目录 8 个运行文件、1,288,423 bytes，`GpAutoLive.exe` 162,816 bytes；独立符号包 7 个文件、436,475 bytes；外置媒体运行时 13 个文件、352,365,694 bytes，5 个二进制资源使用硬链接复用，资源清单 5/5 大小与 SHA-256 匹配。
- v67 使用仓库锁定 `.tools/dotnet` 设置 `DOTNET_ROOT` 启动/关闭成功（`CloseMainWindow=True`、退出码 0、无 `GpAutoLive/mpv/ffmpeg/ffprobe` 残留）；5 秒空闲基线 4 个样本，私有工作集峰值 99,835,904 bytes、工作集峰值 163,004,416 bytes、CPU 峰值 5.27%，GDI 5～17、User 21～40。该数据受启动时序影响，仅作同机对照，不等价于 30 分钟长稳或 GPU 门禁。
- GC 字段已进入应用内快照，但外部基线脚本仍只记录 OS 进程计数器；真实 sidecar、WGC/D3D11、DirectShow、签名/许可证、目标 GPU 和 30 分钟长稳继续待验收。

## 64. 当前实施增量：AkVirtualCamera sidecar 启动计划与 v68 发布复验（2026-09-03）

- 新增 `WindowsVirtualCameraSidecarLaunchPlan` 与 `WindowsVirtualCameraSidecarLaunchPlanBuilder`：只接受 Windows x64 约定文件名 `akvirtualcamera-sidecar-x64.exe`、绝对普通文件和非 Reparse 父目录；文件大小上限 64 MiB，默认启动预算 5 秒（100 ms～30 秒）。
- sidecar 计划仅使用 `--session-token-stdin`，令牌由受管宿主经标准输入传递，不进入命令行、环境变量、普通 JSON/INI 或日志；管道名由内存令牌派生并继续由既有协议层校验。该计划只生成受限启动描述，不宣称已启动真实 sidecar。
- 新增启动计划单元测试，覆盖固定文件名、绝对路径、stdin 令牌参数、错误文件名、全零令牌、无效配置和超时边界；全量自动化测试为 **360 项通过**（Contracts 25、Core 75、Media 91、Windows 136、App 33）。Release 构建 0 警告/0 错误，`dotnet format --verify-no-changes --no-restore` 与 `tools/verify-scope.ps1` 均通过。
- v68 发布复验：正式候选 `artifacts/csharp-windows-controller-20260903-v68` 根目录 8 个运行文件、1,293,031 bytes，`GpAutoLive.exe` 162,816 bytes；独立符号包 7 个文件、436,679 bytes；外置媒体运行时 13 个文件、352,365,694 bytes，5 个二进制资源使用硬链接复用，资源清单 5/5 大小与 SHA-256 匹配。
- v68 使用仓库锁定 `.tools/dotnet` 设置 `DOTNET_ROOT` 启动/关闭成功（`CloseMainWindow=True`、退出码 0、无 `GpAutoLive/mpv/ffmpeg/ffprobe` 残留）；5 秒空闲基线 4 个样本，私有工作集峰值 99,823,616 bytes、工作集峰值 162,435,072 bytes、CPU 峰值 4.33%，GDI 5～17、User 21～40。
- 真实 sidecar PE 签名/x64 文件验证、当前用户 Named Pipe ACL、Job Object 进程树、WGC/D3D11 捕获、AkVirtualCamera DirectShow 注册/卸载、GPL 许可与下游兼容仍保持“正式需求·待实施/未验收”；v68 基线仅是同机空闲对照，不等价于真实设备或 30 分钟长稳门禁。

## 65. 当前实施增量：sidecar PE32+ x64 架构校验与 v69 发布复验（2026-09-03）

- `WindowsVirtualCameraSidecarLaunchPlanBuilder` 在固定文件名、绝对路径、普通文件和大小上限之外，读取不超过 1 MiB 的 PE 头部，严格要求 `MZ`、`PE\0\0`、机器类型 `AMD64 (0x8664)` 和 `PE32+ (0x20b)`；读取失败、截断、x86 或非 PE 文件均 fail-closed。
- 新增 x64 合法最小 PE 夹具和 x86 架构拒绝测试；不会加载、执行或修改待验证 sidecar，也不把文件名当作架构证明。
- 全量自动化测试为 **361 项通过**（Contracts 25、Core 75、Media 91、Windows 137、App 33）。Release 构建 0 警告/0 错误，`dotnet format --verify-no-changes --no-restore` 与 `tools/verify-scope.ps1` 均通过。
- v69 发布复验：正式候选 `artifacts/csharp-windows-controller-20260903-v69` 根目录 8 个运行文件、1,293,543 bytes，`GpAutoLive.exe` 162,816 bytes；独立符号包 7 个文件、437,159 bytes；外置媒体运行时 13 个文件、352,365,694 bytes，5 个二进制资源使用硬链接复用，资源清单 5/5 大小与 SHA-256 匹配。
- v69 使用仓库锁定 `.tools/dotnet` 设置 `DOTNET_ROOT` 启动/关闭成功（`CloseMainWindow=True`、退出码 0、无 `GpAutoLive/mpv/ffmpeg/ffprobe` 残留）；5 秒空闲基线 6 个样本，私有工作集峰值 100,352,000 bytes、工作集峰值 163,983,360 bytes、CPU 峰值 6.60%，GDI 17、User 40。
- 真实 sidecar Authenticode 签名、当前用户 Named Pipe ACL、Job Object 进程树、WGC/D3D11 捕获、DirectShow 注册/卸载、GPL 许可与下游兼容仍保持“正式需求·待实施/未验收”；v69 基线仅为同机空闲对照。

## 66. 当前实施增量：AkVirtualCamera 受管 sidecar 宿主与 v70 发布复验（2026-09-03）

- 新增 `WindowsVirtualCameraSidecarHost`：消费已验证的启动计划，隐藏启动独立 sidecar，使用参数数组和 `UseShellExecute=false`，通过 stdin 写入 32 个 ASCII 十六进制令牌字符加换行并立即关闭 stdin。
- 宿主要求进程加入 Windows Job Object；启动失败、令牌写入失败、取消、stdout/stderr 超限或读取失败均进入稳定错误分类，并在 2 秒清理预算内终止/Join 进程树。标准输出与错误各自限制 64 KiB，不把原文、路径、令牌或管道名写入快照。
- `ConnectClientAsync` 只在宿主处于 Running 时把内部会话管道名交给既有 `WindowsVirtualCameraSidecarClient`，UI/Renderer 不获得令牌或管道名；宿主不创建管道、不注册设备、不执行 WGC/D3D11 捕获，Named Pipe ACL 仍由 sidecar 负责。
- 新增伪造启动计划、未启动连接和不可执行 PE 的宿主测试；全量自动化测试为 **364 项通过**（Contracts 25、Core 75、Media 91、Windows 140、App 33）。Release 构建 0 警告/0 错误，`dotnet format --verify-no-changes --no-restore` 与 `tools/verify-scope.ps1` 均通过。
- v70 发布复验：正式候选 `artifacts/csharp-windows-controller-20260903-v70` 根目录 8 个运行文件、1,315,047 bytes，`GpAutoLive.exe` 162,816 bytes；独立符号包 7 个文件、442,215 bytes；外置媒体运行时 13 个文件、352,365,694 bytes，5 个二进制资源使用硬链接复用，资源清单 5/5 大小与 SHA-256 匹配。
- v70 使用仓库锁定 `.tools/dotnet` 设置 `DOTNET_ROOT` 启动/关闭成功（`CloseMainWindow=True`、退出码 0、无 `GpAutoLive/mpv/ffmpeg/ffprobe` 残留）；5 秒空闲基线 6 个样本，私有工作集峰值 99,950,592 bytes、工作集峰值 163,012,608 bytes、CPU 峰值 5.13%，GDI 17、User 40～41。
- 真实 sidecar 运行、当前用户 Named Pipe ACL、WGC/D3D11 捕获、DirectShow 注册/卸载、Authenticode/GPL 发布材料、目标 GPU/下游兼容和 30 分钟长稳仍保持“正式需求·待实施/未验收”。

## 67. 当前实施增量：虚拟摄像头资源包固定路径探测与 v71 发布复验（2026-09-03）

- 新增 `WindowsVirtualCameraSidecarLocator`，固定解析 `virtual-camera/bin/akvirtualcamera-sidecar-x64.exe`，默认锚定应用目录，也支持显式 `AUTOLIVE_AKVIRTUALCAMERA_ROOT` 供安装验证；安装根目录必须为绝对、存在且非 Reparse 目录。
- 探测只读调用既有 x64 PE32+ 校验，不启动进程、不加载 DLL、不注册 DirectShow、不创建 Named Pipe ACL；宿主启动前仍会再次校验，避免 TOCTOU 后把路径探测误当作执行授权。
- `WindowsVirtualCameraSidecarHost` 的输出超限/读取失败取消路径改为对已释放 `CancellationTokenSource` 安全幂等，避免停止与后台 drain 竞态时产生未观察异常。
- 新增 Windows 5 项定位测试；全量自动化测试为 **369 项通过**（Contracts 25、Core 75、Media 91、Windows 145、App 33）。Release 构建 0 警告/0 错误，格式检查与 `desktop/` 作用域检查通过。
- v71 发布复验：正式候选 `artifacts/csharp-windows-controller-20260903-v71` 根目录 8 个运行文件、1,318,631 bytes，`GpAutoLive.exe` 162,816 bytes；独立符号包 `artifacts/csharp-windows-symbols-20260903-v71` 为 7 个文件、442,895 bytes；外置媒体运行时 13 个文件、352,365,694 bytes，资源清单 5/5 大小与 SHA-256 匹配，继续以 v70 已验证资源和硬链接复用。启动关闭冒烟 `CloseMainWindow=True`、退出码 0 且无 `GpAutoLive/mpv/ffmpeg/ffprobe` 残留；5 秒空闲基线 7 个样本，私有工作集 84,815,872～100,261,888 bytes，工作集 141,918,208～163,397,632 bytes，CPU 峰值 5.49%，GDI 17，User 40～42。
- 真实 sidecar 启停、当前用户 Named Pipe ACL、WGC/D3D11、DirectShow 安装/卸载、Authenticode/GPL 材料、目标 GPU/下游兼容和 30 分钟长稳仍保持“正式需求·待实施/未验收”。

## 68. 当前实施增量：D3D11 硬件前置探测与 v72 发布复验（2026-09-03）

- 新增 `WindowsD3D11CapabilityProbe`，仅在 sidecar 文件已经通过固定路径/x64 PE 校验后由 WPF `Window_Loaded` 异步调用系统 `D3D11CreateDevice`；只请求硬件设备和 BGRA 支持，不使用 WARP，不加载应用 DLL，不创建 WGC 或虚拟摄像头会话。
- 探测结果只分为 Windows 不适用、运行库不可用、设备创建失败、Feature Level 不足和前置通过；即使前置通过，UI 仍显示“设备/WGC 待验收”，不会把 D3D11 可用误报成捕获链运行中。
- 增加资源释放和异常分类边界，COM 设备/上下文指针在 finally 中立即释放；WPF 使用可取消后台任务，窗口关闭时不再回写状态。
- 新增 Windows 1 项探测测试；全量自动化测试为 **370 项通过**（Contracts 25、Core 75、Media 91、Windows 146、App 33）。Release 构建 0 警告/0 错误，格式检查与 `desktop/` 作用域检查通过。
- v72 发布复验：正式候选 `artifacts/csharp-windows-controller-20260903-v72` 根目录 8 个运行文件、1,322,215 bytes，`GpAutoLive.exe` 162,816 bytes；独立符号包 `artifacts/csharp-windows-symbols-20260903-v72` 为 7 个文件、443,711 bytes；外置媒体运行时 13 个文件、352,365,694 bytes，资源清单 5/5 大小与 SHA-256 匹配。启动关闭冒烟 `CloseMainWindow=True`、退出码 0 且无媒体进程残留；5 秒空闲基线 7 个样本，私有工作集 85,098,496～100,839,424 bytes，工作集 142,184,448～163,123,200 bytes，CPU 峰值 4.78%，GDI 17，User 40～43。
- D3D11 前置探测不等价于真实 WGC HWND 捕获、GPU 转换、AkVirtualCamera DirectShow 注册/卸载、签名/许可证、目标 GPU/下游兼容或 30 分钟长稳；这些门禁继续保持“正式需求·待实施/未验收”。

## 69. 当前实施增量：最终效果 HWND 绑定代际契约与 v73 发布复验（2026-09-03）

- 新增 `WindowsVirtualCameraSurfaceBinding`，虚拟摄像头只允许绑定最终效果视频表面 HWND；同一 HWND 重复绑定保持代际不变，窗口重建、切换或关闭会递增 generation，使旧 WGC/sidecar 结果失效。
- `MainWindow` 在最终效果窗口显示后刷新绑定，在窗口关闭时解除绑定并在应用退出时释放；绑定结果不向 UI 暴露句柄，也不创建第二窗口或第二播放器。
- 捕获实现可用 `IsCurrent(windowId, generation)` 作为提交前门禁；本轮仍未把绑定对象误报成真实像素捕获或 GPU 输出。
- 新增 Windows 3 项绑定测试；全量自动化测试为 **373 项通过**（Contracts 25、Core 75、Media 91、Windows 149、App 33）。Release 构建 0 警告/0 错误，格式检查与 `desktop/` 作用域检查通过。
- v73 发布复验：正式候选 `artifacts/csharp-windows-controller-20260903-v73` 根目录 8 个运行文件、1,327,847 bytes，`GpAutoLive.exe` 162,816 bytes；独立符号包 `artifacts/csharp-windows-symbols-20260903-v73` 为 7 个文件、444,887 bytes；外置媒体运行时 13 个文件、352,365,694 bytes，资源清单 5/5 大小与 SHA-256 匹配。启动关闭冒烟 `CloseMainWindow=True`、退出码 0 且无媒体进程残留；5 秒空闲基线 7 个样本，私有工作集 84,451,328～100,298,752 bytes，工作集 141,713,408～163,991,552 bytes，CPU 峰值 5.26%，GDI 17，User 40～41。
- WGC HWND 捕获、D3D11 GPU 转换、真实 sidecar/ACL、DirectShow 安装卸载、签名/许可证、多 GPU/Win10/11、下游兼容和 30 分钟长稳仍保持“正式需求·待实施/未验收”。

## 70. 当前实施增量：真实 WGC HWND frame-pool 会话与 v74 发布复验（2026-09-03）

- 新增 `GpAutoLive.Windows/WindowsGraphicsCaptureCapabilityProbe.cs`：按 Windows 版本和 `GraphicsCaptureSession.IsSupported()` 做 fail-closed 前置探测；WPF 只在窗口加载后异步显示“D3D11/WGC 前置通过”，不把它写成摄像头已运行。
- 新增 `WindowsGraphicsCaptureWindowSession`：绑定 `WindowsVirtualCameraSurfaceBinding` 的最终效果 HWND，在专用捕获线程初始化 WinRT、通过 `IGraphicsCaptureItemInterop` 创建 `GraphicsCaptureItem`、建立 `Direct3D11CaptureFramePool` 和 `GraphicsCaptureSession`，周期观察内容尺寸/时间戳并在 generation 变化时停止提交。停止、取消、关闭、超时均有有界 Join；不做 CPU 像素转换、不创建第二个播放器、不启动 sidecar。
- WPF 虚拟摄像头卡片新增 sidecar、D3D11 和 WGC 三段状态文案；WGC 会话暂不自动启动，等待上层输出按钮、GPU→YUY2 路径和真实 sidecar/DirectShow 门禁完成后再接入。
- Windows 目标框架固定为 `net10.0-windows10.0.19041.0`，使用 .NET SDK 隐式 Windows SDK targeting pack `Microsoft.Windows.SDK.NET.Ref 10.0.26100.87`（通过 `WindowsSdkPackageVersion` 锁定），以获得受支持的 WinRT 投影。该包的许可证入口以其 NuGet `licenseUrl`（[Windows SDK License](https://aka.ms/WinSDKLicenseURL)）为准，发布前仍需复核对应材料。`Microsoft.Windows.SDK.NET.dll` 与 `WinRT.Runtime.dll` 不放入安装根目录，而由 `Features/Runtime/ExternalWinRtRuntimeResolver` 从 `runtime/winrt/` 按程序集白名单加载；缺失或加载失败时保持 fail-closed。该目录约 27.85 MB（十进制），属于可替换运行时依赖，不进入主 EXE。
- 新增真实 Windows 集成测试：创建独立 Win32 顶层窗口并启动 WGC frame pool，验证 `Running`、绑定尺寸和有界停止；无桌面合成/无可见帧环境不宣称已取得像素帧，测试不伪造 `FrameCount`。
- 全量自动化测试为 **382 项通过**（Contracts 25、Core 75、Media 91、Windows 156、App 35）。`dotnet build`、`dotnet format GpAutoLive.Windows.slnx --verify-no-changes --no-restore`、`tools/verify-scope.ps1` 均通过；发布包使用外置 WinRT DLL 启动并可通过 `CloseMainWindow` 正常退出。
- v74 历史候选：`artifacts/csharp-windows-controller-20260903-v74` 根目录 8 个运行文件、`1,348,437` bytes，`GpAutoLive.exe` `162,816` bytes；`runtime/winrt/` 2 个 DLL 合计 `27,848,816` bytes；独立 symbols 包 7 个文件、`451,103` bytes；媒体运行时 13 个硬链接文件、`352,365,694` bytes。
- v74 启动关闭冒烟：锁定 `.tools/dotnet` 设置 `DOTNET_ROOT`/`DOTNET_ROOT_X64` 后，窗口标题为 `GpAutoLive`，`CloseMainWindow=True`、退出码 0、无 `GpAutoLive/mpv/ffmpeg/ffprobe` 残留；5 秒空闲基线 8 个样本，私有工作集峰值 `85,962,752` bytes、工作集峰值 `148,541,440` bytes、CPU 峰值 `1.96%`。
- v74 的 WGC GPU→BGRA/YUY2、固定规格三槽回读、sidecar 当前用户 ACL、AkVirtualCamera DirectShow 安装/卸载、Authenticode/GPL 材料、Win10/11 和多 GPU、下游兼容、真实可见帧与 30 分钟长稳均仍待验收。

## 71. 当前实施增量：同设备 GPU→YUY2 输出链与 v75 发布边界（2026-09-03）

- 新增 `WindowsGraphicsCaptureD3D11Context`：WGC frame pool 回调携带与捕获相同的硬件 D3D11 设备/ImmediateContext，避免把捕获纹理跨适配器复制到第二个设备；上下文只在同步回调内使用，停止时有界释放。
- 新增 `WindowsGraphicsCaptureGpuYuy2Converter`：使用 Vortice.Direct3D11/D3DCompiler 在 GPU 上执行固定 1280×720 缩放、BT.601 limited-range RGB→YUV 和两个像素一组的 YUY2 打包；输出 640×720 RGBA 中间纹理，三个 staging 槽通过 `MapFlags.DoNotWait` 轮询回读，队列满时丢弃当前帧，不等待 GPU 阻塞 UI/捕获线程。
- 新增 `WindowsD3D11HardwareContextFactory`：只创建 `DriverType.Hardware`、`FeatureLevel 11_0` 设备，读取实际 adapter LUID/厂商/设备/名称形成 `GpuCaptureFacts`，拒绝 WARP 和猜测 GPU 事实。
- 新增 `WindowsVirtualCameraGpuOutputSession`：把最终效果 HWND 绑定、真实 WGC 会话、GPU 转换器和既有 `VirtualCameraOutputManager` 串成单一生命周期；首次成功回读后才进入 `Ready`，generation 失效和 latest-wins 仍由 Core 所有者处理，sidecar/DirectShow 继续保持独立边界。
- 新增 Windows GPU 合同/回读测试：硬件设备结果码和常量 BGRA 纹理 GPU→YUY2 limited-range 输出通过；真实 WGC 集成测试继续诚实地只验证 frame pool/尺寸/停止，不在无合成帧环境伪造像素证据。
- Vortice 3.8.3 及其必要传递运行库（`Vortice.Direct3D11`、`Vortice.DirectX`、`Vortice.DXGI`、`Vortice.D3DCompiler`、`Vortice.Mathematics`、`SharpGen.Runtime*`）由程序集白名单 resolver 统一从 `runtime/gpu/` 外置加载；主 EXE 不静态合并这些 DLL。依赖版本、许可证入口、SHA-256 和目录大小必须写入 v75 发布 manifest，运行库缺失时保持 fail-closed。Vortice 包以 MIT 许可发布，版本入口固定为 [Vortice.Direct3D11 3.8.3](https://www.nuget.org/packages/Vortice.Direct3D11/3.8.3) 与 [Vortice.D3DCompiler 3.8.3](https://www.nuget.org/packages/Vortice.D3DCompiler/3.8.3)，交付前仍需复核第三方材料归档。
- 真实 AkVirtualCamera sidecar 当前用户 ACL、DirectShow 注册/卸载、签名/许可证、GPU→sidecar 写入、Win10/11 多 GPU、下游兼容、可见窗口持续帧和 30 分钟长稳仍保持“正式需求·待实施/未验收”；本轮代码只完成到 GPU YUY2 帧边界，不把合成器/软件适配器冒充硬件证据。
- 当前 `VirtualCameraFrame` 合同仍以 `byte[]` 交付固定 1,843,200 bytes 的 YUY2 负载；三槽 GPU 资源和队列已有限界，但每次完成回读仍会产生一个交付缓冲。待 sidecar 消费者所有权和回收语义接入后，再以可审查的池化/租约方式降低 GC 压力；当前不宣称零分配。
- v75 正式候选：`artifacts/csharp-windows-controller-20260903-v75` 根目录 8 个运行文件、`1,379,568` bytes，`GpAutoLive.exe` `162,816` bytes；`runtime/winrt/` 2 个 DLL 合计 `27,848,816` bytes，`runtime/gpu/` 7 个 DLL 合计 `1,208,320` bytes，二者均带 `manifest.json`；独立 symbols 包 `artifacts/csharp-windows-symbols-20260903-v75` 7 个文件、`456,171` bytes；媒体运行时继续复用 13 个硬链接文件、`352,365,694` bytes，资源清单大小与 SHA-256 匹配。
- v75 全量自动化测试为 **385 项通过**（Contracts 25、Core 75、Media 91、Windows 158、App 36）；Release 构建 0 警告/0 错误，`dotnet format --verify-no-changes` 与 `tools/verify-scope.ps1` 均通过。
- v75 发布包启动关闭冒烟使用锁定 `.tools/dotnet` 和外置 WinRT/GPU 运行库，`WaitForInputIdle=True`、窗口标题 `GpAutoLive`、`CloseMainWindow=True`、退出码 0，且无 `GpAutoLive/mpv/ffmpeg/ffprobe` 残留。
- v75 4 秒空闲基线为 6 个样本：私有工作集 `84,709,376～85,929,984` bytes，工作集 `141,668,352～146,436,096` bytes，CPU 峰值 `2.44%`，GDI `17`，User `40～41`；该基线不等价于 GPU 长稳或摄像头下游验收。

## 72. 当前实施增量：首帧状态顺序修正与 v76 发布复验（2026-09-03）

- 修正 `WindowsVirtualCameraGpuOutputSession.OnCapturedFrame` 的首帧顺序：首次 GPU 回读后先以同一 D3D11 设备构造并校验 `GpuCaptureFacts`，成功调用 `VirtualCameraOutputManager.MarkReady` 后再 `SubmitFrame`；避免 `Starting` 状态下提交帧被 Core 所有者拒绝。
- 该修正不放宽 generation、latest-wins、固定 YUY2 负载、GPU 缩放/色彩转换、三槽 `DO_NOT_WAIT` 回读或 sidecar/DirectShow 独立边界；捕获失败时仍清理 `_started`，允许用户在修复后重新启动会话。
- v76 正式候选：`artifacts/csharp-windows-controller-20260903-v76` 根目录 8 个运行文件、`1,382,640` bytes，`GpAutoLive.exe` `162,816` bytes；`runtime/winrt/` 2 个 DLL 合计 `27,848,816` bytes，`runtime/gpu/` 7 个 DLL 合计 `1,208,320` bytes，均带 manifest；独立 symbols 包 7 个文件、`457,979` bytes；媒体运行时继续复用 13 个硬链接文件、`352,365,694` bytes。
- v76 全量自动化测试为 **385 项通过**（Contracts 25、Core 75、Media 91、Windows 158、App 36）；Release 构建 0 警告/0 错误，`dotnet format --verify-no-changes --no-restore` 与 `tools/verify-scope.ps1` 均通过；三个运行库 manifest 的文件大小和 SHA-256 校验通过。
- v76 发布包使用锁定 `.tools/dotnet` 启动关闭冒烟 `WaitForInputIdle=True`、窗口标题 `GpAutoLive`、`CloseMainWindow=True`、退出码 0，且无 `GpAutoLive/mpv/ffmpeg/ffprobe` 残留。4 秒空闲基线 6 个样本：私有工作集 `84,545,536～85,749,760` bytes，工作集 `141,676,544～146,472,960` bytes，CPU 峰值 `2.00%`，GDI `17`，User `40～41`。
- 真实 WGC 可见帧、GPU 三槽长期回读、sidecar 当前用户 ACL、AkVirtualCamera DirectShow 注册/卸载、签名/许可证、Win10/11 多 GPU、下游兼容和 30 分钟长稳仍需目标设备验收；当前不把合成纹理夹具或 frame-pool 启停证据写成真实摄像头已可用。

## 73. 当前实施增量：sidecar 固定帧输出组合与 v77 发布复验（2026-09-03）

- 新增 `WindowsVirtualCameraSidecarOutputWriter`：以固定 `30fps` 向已连接的 sidecar 串行写入协议帧，首帧立即发送；输出策略由 `VirtualCameraOutputManager` 决定，播放/帧不可用时发送单份预分配黑色 YUY2 负载，播放有效时读取 latest-wins 帧并复用其 payload 引用，不额外复制像素负载。
- 新增 `WindowsVirtualCameraOutputCoordinator`：按 `sidecar host → Named Pipe client → WGC/D3D11 GPU 会话 → 固定帧输出泵` 顺序启动，按相反顺序停止/释放；使用生命周期信号量防止并发启停，失败路径回收已启动的 sidecar/client/GPU 资源；sidecar stdout 仅解析 `GPAKVC_CLIENTS N` 状态行并限制 64 KiB 总量、4 KiB 单行。
- 输出组合继续不安装 DirectShow、不创建 sidecar ACL、不修改 `desktop/`，也不把“管道写入成功”当作设备已安装或下游正在消费；下游客户端数量只有收到 sidecar 状态行后才更新 Core 状态。
- 新增固定帧输出泵、完整组合生命周期和客户端数量状态解析测试；全量自动化测试为 **392 项通过**（Contracts 25、Core 75、Media 91、Windows 165、App 36）。Release 构建 0 警告/0 错误，`dotnet format --verify-no-changes --no-restore` 与 `tools/verify-scope.ps1` 均通过。
- v77 正式候选：`artifacts/csharp-windows-controller-20260903-v77` 根目录 8 个运行文件、`1,414,384` bytes，`GpAutoLive.exe` `162,816` bytes；`runtime/winrt/` 2 个 DLL 合计 `27,848,816` bytes，`runtime/gpu/` 7 个 DLL 合计 `1,208,320` bytes，均带 manifest；独立 symbols 包 `artifacts/csharp-windows-symbols-20260903-v77` 7 个文件、`471,231` bytes；媒体运行时继续复用 13 个硬链接文件、逻辑大小 `352,365,694` bytes，三个运行库 manifest 的大小与 SHA-256 校验通过。
- v77 发布包启动关闭冒烟使用锁定 `.tools/dotnet` 和外置 WinRT/GPU 运行库，`WaitForInputIdle=True`、窗口标题 `GpAutoLive`、`CloseMainWindow=True`、退出码 0，且无 `GpAutoLive/mpv/ffmpeg/ffprobe` 残留；6 个样本空闲基线私有工作集 `84,504,576～85,823,488` bytes、工作集 `141,459,456～146,329,600` bytes、CPU 峰值 `1.72%`，GDI `17`、User `40～41`。
- 真实 sidecar 进程和当前用户 Named Pipe ACL、GPU→sidecar 持续写入、WGC 可见帧、多 GPU/Win10/11、AkVirtualCamera DirectShow 安装/卸载、签名/许可证、下游兼容和 30 分钟长稳仍需目标设备验收；v77 只把代码边界和本机回环测试标记为已接入，不宣称真实摄像头发布已完成。

## 74. 当前实施增量：Windows 安装门禁探测、WPF 刷新入口与 v78 发布复验（2026-09-03）

- 新增 `WindowsVirtualCameraInstallationProbe`：只读检查 `HKLM` 64/32 位 `Webcamoid\VirtualCamera` 安装所有者、固定 x64/x86 组件和 `AkVCamManager.exe` 文件，以及当前存在的 `GpAutoLive Camera` PnP 设备；不调用安装器、`regsvr32`、Manager 或 sidecar，不返回路径、实例 ID 或原始系统错误。
- 安装门禁要求双注册表视图路径一致、固定组件完整且 PnP 设备存在，任一条件不满足均保持 fail-closed；只有探测成功才允许 Core 从 `Unavailable/Failed` 进入 `Installed`，sidecar 文件存在本身不会提升状态。
- WPF 虚拟摄像头卡片新增“刷新探测”入口，启动加载和手动刷新均在后台执行安装、D3D11、WGC 探测，并在窗口线程合并结果；刷新不会注册设备、启动进程或阻塞本地播放，状态文案明确区分安装门禁、GPU 前置和真实 sidecar/DirectShow 验收。
- 新增 Windows 安装探测边界测试；全量自动化测试为 **394 项通过**（Contracts 25、Core 75、Media 91、Windows 167、App 36）。Release 构建 0 警告/0 错误，`dotnet format --verify-no-changes --no-restore` 与 `tools/verify-scope.ps1` 均通过。
- v78 正式候选：`artifacts/csharp-windows-controller-20260903-v78` 根目录 8 个运行文件、`1,421,040` bytes，`GpAutoLive.exe` `162,816` bytes；`runtime/winrt/` 2 个 DLL 合计 `27,848,816` bytes，`runtime/gpu/` 7 个 DLL 合计 `1,208,320` bytes，均带 manifest；独立 symbols 包 `artifacts/csharp-windows-symbols-20260903-v78` 7 个文件、`471,879` bytes；媒体运行时继续复用 13 个硬链接文件、逻辑大小 `352,365,694` bytes，三个运行库 manifest 的文件大小与 SHA-256 校验通过。
- v78 发布包启动关闭冒烟使用锁定 `.tools/dotnet`，`WaitForInputIdle=True`、窗口标题 `GpAutoLive`、`CloseMainWindow=True`、退出码 0，且无 `GpAutoLive/mpv/ffmpeg/ffprobe` 残留；4 秒空闲基线 6 个样本，私有工作集 `85,704,704～86,974,464` bytes、工作集 `143,773,696～147,521,536` bytes、CPU 峰值 `1.47%`，GDI `17`、User `40～41`。
- 真实 DirectShow 双位数安装、签名/许可证、当前用户 Named Pipe ACL、GPU→sidecar 持续输出、WGC 可见帧、多 GPU/Win10/11、下游兼容和 30 分钟长稳仍需目标设备验收；v78 仅把原生安装探测和 UI 刷新入口标记为已接入，不宣称真实摄像头发布已完成。

## 75. 当前实施增量：虚拟摄像头 WPF 启停接线、媒体变更生命周期与 v79 发布复验（2026-09-03）

- 虚拟摄像头卡片新增“启动输出”“停止输出”“刷新探测”三个明确动作；启动按钮只有在登录、双视图安装门禁、受信任 sidecar、D3D11 硬件前置、WGC 前置、最终效果 HWND 绑定全部通过时才可用。未满足任一条件时保持禁用并给出可行动文案，不自动注册 DirectShow、不自动启动 Manager 或 sidecar。
- 启动动作通过既有 `RunPlaybackCommandAsync` 串行化，按 `sidecar host → Named Pipe client → WGC/D3D11 GPU → 30fps writer` 顺序启动；停止按逆序释放。sidecar 会话令牌每次启动重新生成，超时/取消/失败均回收已启动资源并回写 Core 终态。
- 媒体池导入、拖放、替换、删除、清空提交前统一调用 `StopMediaForMutationAsync`；若虚拟摄像头正在 `Starting/Ready/Streaming/Recovering`，先停止输出，失败则保持旧媒体池。最终效果窗口被用户关闭时也会先停止虚拟摄像头，再解除 HWND generation 绑定，避免旧捕获链继续写入。
- 新增/调整文件：`GpAutoLive.Windows/WindowsVirtualCameraInstallationProbe.cs`、`GpAutoLive.Windows.Tests/WindowsVirtualCameraInstallationProbeTests.cs`、`GpAutoLive.App/MainWindow.xaml`、`GpAutoLive.App/MainWindow.xaml.cs`。安装探测仍为只读、fail-closed；本轮未删除任何用户代码或参考实现。
- 全量自动化测试为 **394 项通过**（Contracts 25、Core 75、Media 91、Windows 167、App 36）；锁定还原、Release 构建（0 警告/0 错误）、`dotnet format --verify-no-changes --no-restore` 与 `tools/verify-scope.ps1` 均通过。
- v79 正式候选：`artifacts/csharp-windows-controller-20260903-v79` 根目录 8 个运行文件、`1,428,208` bytes，`GpAutoLive.exe` `162,816` bytes；`runtime/winrt/` 2 个 DLL 合计 `27,848,816` bytes，`runtime/gpu/` 7 个 DLL 合计 `1,208,320` bytes，均带 manifest；独立 symbols 包 `artifacts/csharp-windows-symbols-20260903-v79` 7 个文件、`473,235` bytes；媒体运行时继续复用 13 个硬链接文件、逻辑大小 `352,365,694` bytes，三个运行库 manifest 的文件大小与 SHA-256 校验通过。
- v79 发布包使用锁定 `.tools/dotnet` 启动关闭冒烟：`WaitForInputIdle=True`、窗口标题 `GpAutoLive`、`CloseMainWindow=True`、退出码 0，且无 `GpAutoLive/mpv/ffmpeg/ffprobe` 残留。4 秒空闲基线 6 个样本，私有工作集 `85,475,328～86,773,760` bytes、工作集 `142,733,312～147,476,480` bytes、CPU 峰值 `2.74%`，GDI `17`、User `40～41`。
- 真实 DirectShow 双位数安装、签名/许可证、当前用户 Named Pipe ACL、GPU→sidecar 持续输出、WGC 可见帧、多 GPU/Win10/11、下游兼容、真实 ZLMediaKit/RTMPS 网络和 30 分钟长稳仍需目标设备验收；v79 只把 WPF 启停编排和生命周期门禁标记为已接入，不宣称真实摄像头发布已完成。

## 76. 当前实施增量：独立 AppUserModelID 与 v80 发布复验（2026-09-03）

- 新增 `GpAutoLive.Windows/WindowsAppIdentity.cs`，通过 Windows Shell `SetCurrentProcessExplicitAppUserModelID` 设置固定 `GpAutoLive.CSharp.Windows` 身份；C# 客户端与 Rust/Tauri 参考实现使用不同任务栏/开始菜单身份，单实例互斥名继续独立。
- API 调用只在 Windows 上执行，ID 长度/控制字符先校验，Shell API 缺失或失败返回稳定分类但不阻断本地启动；新增 Windows 身份合同测试，未引入第三方依赖。
- `App.OnStartup` 在创建实例互斥前配置 shell 身份；不会修改安装器、Rust/Tauri 文件或用户配置路径。未执行未经确认的广泛未使用代码清理。
- 全量自动化测试为 **396 项通过**（Contracts 25、Core 75、Media 91、Windows 169、App 36）；锁定还原、Release 构建（0 警告/0 错误）、`dotnet format --verify-no-changes --no-restore` 与 `tools/verify-scope.ps1` 均通过。
- v80 正式候选：`artifacts/csharp-windows-controller-20260903-v80` 根目录 8 个运行文件、`1,429,232` bytes，`GpAutoLive.exe` `162,816` bytes；`runtime/winrt/` 2 个 DLL 合计 `27,848,816` bytes，`runtime/gpu/` 7 个 DLL 合计 `1,208,320` bytes，均带 manifest；独立 symbols 包 `artifacts/csharp-windows-symbols-20260903-v80` 7 个文件、`473,431` bytes；媒体运行时继续复用 13 个硬链接文件、逻辑大小 `352,365,694` bytes，三个运行库 manifest 的文件大小与 SHA-256 校验通过。
- v80 发布包使用锁定 `.tools/dotnet` 启动关闭冒烟：`WaitForInputIdle=True`、窗口标题 `GpAutoLive`、`CloseMainWindow=True`、退出码 0，且无 `GpAutoLive/mpv/ffmpeg/ffprobe` 残留。4 秒空闲基线 6 个样本，私有工作集 `85,594,112～86,867,968` bytes、工作集 `142,729,216～147,513,344` bytes、CPU 峰值 `2.49%`，GDI `17`、User `40～41`。
- C# AppUserModelID 只解决 shell 身份隔离，不等价于安装签名、升级/卸载或双客户端媒体所有权门禁；这些以及真实 DirectShow、sidecar ACL、GPU→sidecar、WGC 可见帧、多 GPU、下游兼容、真实 ZLMediaKit/RTMPS 网络和 30 分钟长稳仍需目标设备验收。

## 77. 当前实施增量：全局媒体/输出资源所有权门禁与 v81 发布复验（2026-09-03）

- 新增 `GpAutoLive.Windows/WindowsMediaOutputOwnership.cs`：使用固定 `Local\\GpAutoLive.MediaOutput.Owner.v1` 命名互斥实现全局媒体/输出资源租约；立即尝试、不等待、不启动或终止其他进程，重复获取和异常退出后的 abandoned mutex 均有确定结果，释放幂等且不触碰用户配置或媒体文件。
- 获取资源锁前只读探测 Rust/Tauri 正式进程名 `autolive-desktop-core`；检测到参考客户端或无法可靠探测时 fail-closed。C# 自身进程内另有不区分线程的名称集合，避免 .NET Mutex 同线程可重入造成重复租约。
- `GpAutoLive.App/App.xaml.cs` 在独立 AppUserModelID 配置后、C# 单实例互斥前取得资源租约；资源门禁失败显示可行动提示并退出，单实例创建失败或退出时按逆序释放租约。Rust/Tauri 目录保持只读，因此本轮未宣称两个客户端已完成双向锁协议；灰度期间仍要求单端运行。
- 新增 4 项 Windows 所有权边界测试，覆盖首租约、重复获取、释放重试、参考进程探测、非法名称和探测异常；全量自动化测试为 **400 项通过**（Contracts 25、Core 75、Media 91、Windows 173、App 36）。
- v81 正式候选：`artifacts/csharp-windows-controller-20260903-v81` 根目录 8 个运行文件、`1,434,352` bytes，`GpAutoLive.exe` `162,816` bytes；`runtime/winrt/` 2 个 DLL 合计 `27,848,816` bytes，`runtime/gpu/` 7 个 DLL 合计 `1,208,320` bytes，二者均带 manifest；独立 symbols 包 `artifacts/csharp-windows-symbols-20260903-v81` 7 个文件、`474,875` bytes；媒体运行时继续复用 13 个硬链接文件、逻辑大小 `352,365,694` bytes，三个运行库 manifest 的大小与 SHA-256 校验通过。
- v81 发布包使用锁定 `.tools/dotnet` 启动关闭冒烟：`WaitForInputIdle=True`、窗口标题 `GpAutoLive`、`CloseMainWindow=True`、退出码 0，且无 `GpAutoLive/mpv/ffmpeg/ffprobe` 残留；4 秒空闲基线 6 个样本，私有工作集 `85,340,160～86,671,360` bytes、工作集 `142,761,984～147,570,688` bytes、CPU 峰值 `2.26%`，GDI `17`、User `40～42`。
- 全局租约只证明 C# 端的 fail-closed 边界；Rust/Tauri 未修改，故其启动时不会主动读取该互斥，仍需发布前双客户端协议、升级/卸载、签名/许可证、真实 DirectShow、sidecar ACL、GPU→sidecar、WGC 可见帧、多 GPU、下游兼容、真实 ZLMediaKit/RTMPS 网络和 30 分钟长稳验收。

## 78. 当前实施增量：Windows Authenticode 签名探测与 v82 发布复验（2026-09-03）

- 新增 `GpAutoLive.Windows/WindowsAuthenticodeProbe.cs`：在 Windows 上对绝对路径执行普通文件、长度、控制字符、目录和重解析点校验，再通过系统 `WinVerifyTrust` 检查 Authenticode；调用固定为无 UI、无网络吊销刷新（缓存优先）、不修改文件，并将未签名、无效签名、文件缺失、路径非法、API 不可用和探测失败映射为稳定结果码。
- 新增 `GpAutoLive.Windows.Tests/WindowsAuthenticodeProbeTests.cs` 3 项边界测试，覆盖缺失/相对路径、目录路径和当前测试程序集的有界签名分类。未签名开发构建只返回 `Unsigned`，不在启动时强制签名，也不把开发包写成“已签名发布”。
- v82 正式安装候选：`artifacts/csharp-windows-controller-20260903-v82` 根目录 8 个运行文件、`1,437,424` bytes，`GpAutoLive.exe` `162,816` bytes；`runtime/winrt/` 2 个 DLL 合计 `27,848,816` bytes，`runtime/gpu/` 7 个 DLL 合计 `1,208,320` bytes，二者均带 manifest；独立 symbols 包 `artifacts/csharp-windows-symbols-20260903-v82` 7 个文件、`475,775` bytes；媒体运行时 13 个文件、逻辑大小 `352,365,694` bytes，三套运行库 manifest 的大小与 SHA-256 校验通过。
- v82 全量自动化测试为 **403 项通过**（Contracts 25、Core 75、Media 91、Windows 176、App 36）；锁定还原、Release 构建（0 警告/0 错误）、`dotnet format --verify-no-changes --no-restore` 与 `tools/verify-scope.ps1` 均通过。
- v82 发布包启动关闭冒烟使用锁定 `.tools/dotnet`：`WaitForInputIdle=True`、窗口标题 `GpAutoLive`、`CloseMainWindow=True`、退出码 0，且无 `GpAutoLive/mpv/ffmpeg/ffprobe` 残留；4 秒空闲基线 6 个样本，私有工作集 `84,905,984～86,142,976` bytes、工作集 `139,907,072～144,031,744` bytes、CPU 峰值 `1.25%`，GDI `17`、User `38`。
- 签名探测只提供发布前事实校验，不替代代码签名证书、时间戳、安装器签名、升级/卸载回滚、第三方 GPL/MIT/WinSDK 材料归档或目标设备兼容验收；当前 v82 候选仍为未签名开发包，签名门禁保持“正式需求·待实施/未验收”。
- 全局媒体/输出互斥仍是 C# 端 fail-closed 边界；Rust/Tauri 目录保持只读，未宣称已完成双向锁协议。真实 DirectShow/sidecar ACL、GPU→sidecar 连续帧、WGC 可见帧、多 GPU/Win10/11、下游兼容、真实 ZLMediaKit/RTMPS 网络和 30 分钟长稳仍需目标设备验收。

## 79. 当前实施增量：sidecar 签名启动门禁与 v83 发布复验（2026-09-03）

- `WindowsVirtualCameraSidecarProbeResult` 新增 `SignatureCode` 与 `IsTrusted`：sidecar 先通过固定文件名、普通文件、x64 PE32+ 和目录校验，再执行 Authenticode 探测；未签名、无效签名或签名 API 失败时仍保留诊断结果，但不被当作“受信任组件”。
- WPF 虚拟摄像头“启动输出”现在要求安装门禁、sidecar `IsTrusted`、D3D11/WGC 前置和最终效果 HWND 同时通过；刷新探测在未签名 sidecar 时只更新诊断状态，不启动 sidecar、Named Pipe 或 GPU 输出链。界面文案改为明确显示“签名发布门禁未通过”，不把 x64 文件存在误报为可用设备。
- 该门禁不修改 `desktop/`，不自动签名，也不改变现有单实例、资源互斥、generation/latest-wins 和停止回收顺序；开发夹具中的最小 PE 会返回 `Unsigned`/无效分类，保持可测试但不可启动。
- v83 正式安装候选：`artifacts/csharp-windows-controller-20260903-v83` 根目录 8 个运行文件、`1,439,472` bytes，`GpAutoLive.exe` `162,816` bytes；`runtime/winrt/` 2 个 DLL 合计 `27,848,816` bytes，`runtime/gpu/` 7 个 DLL 合计 `1,208,320` bytes，二者均带 manifest；独立 symbols 包 `artifacts/csharp-windows-symbols-20260903-v83` 7 个文件、`476,035` bytes；媒体运行时 13 个文件、逻辑大小 `352,365,694` bytes，三套运行库 manifest 的大小与 SHA-256 校验通过。
- v83 全量自动化测试为 **403 项通过**（Contracts 25、Core 75、Media 91、Windows 176、App 36）；Release 构建 0 警告/0 错误，锁定还原、`dotnet format --verify-no-changes --no-restore` 与 `tools/verify-scope.ps1` 均通过。
- v83 发布包启动关闭冒烟使用锁定 `.tools/dotnet`：`WaitForInputIdle=True`、窗口标题 `GpAutoLive`、`CloseMainWindow=True`、退出码 0，且无 `GpAutoLive/mpv/ffmpeg/ffprobe` 残留；4 秒空闲基线 6 个样本，私有工作集 `85,250,048～86,310,912` bytes、工作集 `140,173,312～144,130,048` bytes、CPU 峰值 `1.72%`，GDI `17`、User `38`。
- v83 候选仍未签名，`Get-AuthenticodeSignature` 返回 `NotSigned`；代码签名证书/时间戳、安装器升级卸载、真实 DirectShow/sidecar ACL、GPU→sidecar 连续帧、WGC 可见帧、多 GPU/Win10/11、下游兼容、真实 ZLMediaKit/RTMPS 网络和 30 分钟长稳继续保持“正式需求·待实施/未验收”。

## 80. 当前实施增量：发布包只读核验工具与 v83 门禁复核（2026-09-03）

- 新增 `tools/verify-release-package.ps1`：只读检查安装根目录白名单（8 个运行文件）、拒绝额外根目录文件/目录和重解析点，按 Manifest 校验 GPU、WinRT、媒体资源的版本、相对路径、大小和 SHA-256，并确认媒体法律材料存在。
- 工具支持可选 `-SymbolsRoot` 校验 7 个独立 PDB/XML 文件；默认输出主程序集和外置 `.dll/.exe` 的 Authenticode 状态（不把 `NotSigned` 伪装成通过），传入 `-RequireSigned` 时任一非 `Valid` 状态立即 fail-closed。报告可通过 `-OutputPath` 原子写入 JSON，不改写候选包。
- 已对 v83 执行核验：Manifest/许可证/符号包通过；WinRT 与 `d3dcompiler_43.dll` 为 `Valid`，开发程序集、mpv、FFmpeg、PortAudio 和 Vortice GPU DLL 为 `NotSigned`，因此 `-RequireSigned` 按预期拒绝该开发候选。
- 该工具补齐发布门禁的可重复证据，但不替代签名证书/时间戳、安装器升级卸载、双客户端锁协议、真实设备和 30 分钟长稳；`desktop/` 仍保持只读。

## 81. 当前实施增量：版本化应用安装事务与原子激活（2026-09-03）

- 新增 `tools/install-csharp-windows-package.ps1`：安装前调用发布包核验脚本，拒绝重解析点、非法版本号、额外根目录内容和无法检查的运行中 `GpAutoLive` 进程；安装目标采用 `versions/<version>/`，不覆盖已存在版本。
- 安装顺序固定为“校验 → 创建临时 staging → 逐文件复制 → 同卷移动到版本目录 → 原子写入 `current.json`”；指针写入失败不会删除旧版本或旧指针，临时目录只在未提交时清理。`-WhatIf` 可输出计划而不写入版本目录，`-RequireSigned` 贯穿发布包签名门禁。
- 已在本机临时安装根目录验证 v83：首次安装生成 `versions/v83-test/` 和 `current.json`，安装目录再次通过 `verify-release-package.ps1`；重复安装同一版本按预期拒绝覆盖。该测试目录仅为本地验证产物，不代表已安装到用户系统。
- 该骨架提供离线包的安全激活边界；Runtime bootstrapper 已在第 83 节接入，最小 WPF 安装/升级维护入口已在第 89 节接入。真实 Runtime 下载/安装、签名证书/时间戳、完整卸载清理和真实设备验收仍待实施，不自动触碰用户配置或 `desktop/`。

## 82. 当前实施增量：升级回滚与版本卸载边界（2026-09-03）

- 新增 `tools/rollback-csharp-windows-package.ps1`：读取并校验有界 `current.json`，默认回滚到 `previous_version`，也支持指定已安装版本；目标版本先复用发布包核验，再以临时指针文件和同卷覆盖方式原子激活，失败时保留原指针。
- 新增 `tools/uninstall-csharp-windows-package.ps1`：只允许卸载经过安装状态核验的非活动、非当前回滚候选版本；拒绝运行中的目标安装、重解析点、非法版本和损坏指针，保留 `current.json`、其他版本及用户配置；`-WhatIf` 只输出计划，不写入或删除任何内容。
- 已在本机 v83 临时安装根目录验证：回滚 `v83-test-3 → v83-test` 成功，非活动 `v83-test-2` 的 WhatIf/实际卸载成功，尝试卸载活动版本按预期拒绝；`desktop/` 未修改。
- 该增量完成离线版本切换的可恢复边界；第 89 节已接入最小 WPF 维护壳。裸机在线 Runtime bootstrapper、签名证书/时间戳、完整安装根目录卸载、真实设备和 30 分钟长稳仍待实施/验收。

## 83. 当前实施增量：Windows .NET Desktop Runtime Bootstrapper（2026-09-03）

- 新增 `tools/bootstrap-csharp-windows-runtime.ps1`：从固定 Program Files/PATH 探测 `dotnet.exe`，用 `--list-runtimes`、5 秒超时和有界输出确认指定主版本 `Microsoft.WindowsDesktop.App`；已安装时只输出 `ready`，缺失时输出 `runtime_missing`，不影响主程序体积。
- 在线下载必须显式传入 `-Download -Install -Sha256 <64位哈希>`，仅接受 HTTPS 官方 .NET 域名，禁止自动重定向，响应和安装器均有 350 MB 上限；下载完成后要求 SHA-256 与 Authenticode `Valid` 才允许继续，避免下载到临时文件后被丢弃。
- 安装必须额外传入 `-Install`，调用官方安装器 `/install /quiet /norestart`，支持管理员提升，接受 0/3010 并重新探测；`-WhatIf` 只输出下载/安装计划，不联网、不写盘、不启动安装器。
- 已验证本机：默认探测返回 `runtime_missing`；官方 AzureEdge URL 的 WhatIf 输出正确；未签名 v83 EXE 被安装器签名门禁拒绝。真实 Runtime 下载、安装、重启、Windows 10/11 权限和卸载仍待目标机验收。

## 84. 当前实施增量：发布包 Authenticode 签名流水线（2026-09-03）

- 新增 `tools/sign-csharp-windows-package.ps1`：签名对象限定为候选包内 `.exe/.dll` 普通文件，证书只从 CurrentUser/LocalMachine `My` 存储区按 SHA-1 thumbprint 查找，必须存在私钥；不接受密码参数，不把敏感信息写入命令行或日志。
- 时间戳 URL 仅允许 HTTPS 的固定 RFC3161 站点；`signtool.exe` 使用 Windows SDK x64 工具，参数通过 `ArgumentList` 传递，输出 256 KiB、单文件 120 秒有界，异常立即停止。
- 签名后重算 GPU、WinRT、媒体 Manifest 中的文件大小/SHA-256，并调用 `verify-release-package.ps1 -RequireSigned` 复核；`-WhatIf` 只统计 PE 文件，不修改候选包。
- 已验证本机 v83：WhatIf 识别 20 个 PE、17 个待签名文件；无效证书 thumbprint 按预期拒绝；未在没有用户证书和 `signtool` 的环境中伪造或执行签名。真实证书、时间戳、第三方许可、签名后完整安装包及目标机验收仍待实施。

## 85. 当前进度审计与可查看性（2026-09-03）

- 可查看状态：C# WPF 开发端已能在本机启动，当前进程为 `GpAutoLive.exe`，窗口标题 `GpAutoLive`，响应状态正常；启动路径为 `src/GpAutoLive.App/bin/x64/Release/net10.0-windows10.0.19041.0/win-x64/GpAutoLive.exe`，使用项目内锁定 .NET Runtime。仓库静态 GUI 预览见 `docs/assets/gpautolive-csharp-windows-gui-preview-20260902.png`；静态预览不等同于当前实时窗口截图。
- 已完成的工程层范围：C0/C1/C2 已完成；C3/C4/C5 的核心边界、WPF 播放/设置/输出入口、外置运行库拆分、性能采样、C7 发布包核验、版本安装/回滚/卸载、Runtime 探测/Bootstrapper、签名流水线已接入并有自动化或本机脚本验证。最新全量自动化测试为 422 项通过，v83 主 EXE 为 162,816 bytes。
- 尚未达到正式发布的门禁：真实 mpv 首帧/EOF/换源、PortAudio 声卡与设备恢复、真实 RTMP/RTMPS、AkVirtualCamera DirectShow/sidecar/ACL/WGC、抖音 QR/协议/发送、自回显、代码签名证书与时间戳、裸机在线 Bootstrapper、完整安装根卸载、Windows 10/11 与多 GPU/声卡矩阵、30 分钟长稳。
- 时间估算（非验收承诺）：在已有测试设备、证书、Runtime 安装权限和抖音/RTMP 测试条件都可用的前提下，剩余代码整合约 **5～8 个工作日**，真实设备/网络/发布门禁约 **10～20 个工作日**，合计约 **3～5 周**；外部账号、驱动、上游 sidecar 或硬件不可用时，时间会顺延，不能用本机单元测试替代。
- 当前不替换现有 `desktop/`，不删除用户代码或配置；默认继续保持 C# 与 Rust/Tauri 双端灰度隔离。

## 86. 当前实施增量：安装状态只读核验（2026-09-03）

- 新增 `tools/get-csharp-windows-install-state.ps1`：只读检查安装根目录形状、有界 `current.json`、`versions/<version>` 目录、活动版本和 `previous_version` 回滚候选；每个已安装版本复用 `verify-release-package.ps1` 校验 GPU/WinRT/媒体 Manifest、文件大小与 SHA-256。
- 工具拒绝非法版本名、缺失或不匹配的指针、活动/回滚版本缺失、额外安装根目录项、重解析点、版本数量或目录项超限；输出只包含版本名、状态、Manifest 数量和签名计数，不输出安装绝对路径、凭据或原始异常文本。
- 默认运行模式为 `read_only`，`-WhatIf` 仍执行相同有界检查并将模式标记为 `what_if`；工具不创建、覆盖、删除文件，也不启动进程。状态报告超过有界大小时只返回 `report_limit`，保持 fail-closed。
- 已使用本机临时安装根目录 `artifacts/install-test-v83` 验证：活动版本 `v83-test`、回滚候选 `v83-test-3` 和两套 Manifest 均返回 `healthy/installation_verified`；`-WhatIf` 返回相同健康结论且不改变目录；非法 `../escape` 指针返回 `invalid/current_pointer_active_invalid`。该工具不会修改 `desktop/`。

## 87. 当前实施增量：发布冒烟与多会话验证接线（2026-09-03）

- 新增 `tools/smoke-csharp-windows-release.ps1` 与 `docs/C7-发布启动关闭冒烟验证.md`：使用锁定 `.tools/dotnet` 启动指定候选，等待 `WaitForInputIdle`/顶层窗口，调用 `CloseMainWindow()`，检查退出码、强制终止和 `GpAutoLive/mpv/ffmpeg/ffprobe` 残留；单个 JSON 输出限制 64 KiB，`-PlanOnly/-WhatIf` 不启动进程、不写报告。
- 发现同名进程时脚本返回 `blocked_preexisting_processes` 并保留现有进程；本机当前开发窗口正在运行时已验证该 fail-closed 行为，未终止 PID 20504。
- 本轮按功能边界拆分为发布冒烟、安装状态核验两个子会话并行执行，主线程完成脚本审查、文档覆盖修正和统一回归；两个子会话均未修改 `desktop/`，未删除用户代码。
- 主线程统一验证：PowerShell 解析、安装状态 read-only/WhatIf、发布冒烟 PlanOnly、`dotnet format --verify-no-changes`、全量 422 项测试、`verify-scope.ps1` 和 v83 发布包核验均通过。真实无进程冒烟需先关闭当前开发窗口后再执行。

## 88. 当前实施增量：C3–C5 本地 Windows 离线验收矩阵（2026-09-03）

- 新增 `tools/verify-c3-c5-local-acceptance.ps1` 与 [`C3–C5 本地 Windows 离线验收矩阵`](./C3-C5-本地离线验收矩阵.md)，按 C3 媒体池/输入探测、C4 资源/mpv/进程/音频设备和 C5 RTMP 合同/生命周期组织既有自动化边界。
- 脚本固定 `--no-restore --no-build`，临时禁用 `AUTOLIVE_TEST_*` 真实夹具，只运行不访问控制面或 RTMP 服务的测试类；同时比较执行前后的 `mpv/ffmpeg/ffprobe` PID，既有进程只记录、不终止，新增残留返回非零。
- 超时/取消进程夹具改用 `127.0.0.1` 回环地址，不再向外部保留地址发包。本矩阵不启动 `GpAutoLive.exe`，不替代 C7 发布启动/关闭冒烟，也不提升真实 GPU、声卡、麦克风、ZLMediaKit/RTMPS 或 30 分钟长稳的未验收状态。
- 代码总监审查把真实媒体、无输入/输出设备和 RTMP 握手/重连作为 `deferred_gates` 单列；同时记录 PortAudio 恢复次数/原生硬超时/麦克风健康观察、FFmpeg stderr 反压和 RTMP 进程启动即标记 Publishing 等未闭合风险，离线矩阵通过不得覆盖这些状态。
- 本机验证：Release 构建 0 警告/0 错误，格式检查、8 行 158 项离线矩阵和 `verify-scope.ps1` 通过，报告未发现新增 `mpv/ffmpeg/ffprobe` 残留；全量回归 422 项通过。

## 89. 当前实施增量：C7 最小 WPF 安装维护壳（2026-09-03）

- 新增独立 `GpAutoLive.Installer` WPF 项目，发行入口为 `GpAutoLive.Setup.exe`。该项目无任何主客户端 `ProjectReference` 或第三方生产依赖，只复制六个既有安装维护脚本到固定 `tools/` 目录，因此没有改变 `GpAutoLive.exe`；两者本机 Release apphost 均为 `162,816` bytes。
- 界面支持 `.NET 10 Windows Desktop Runtime` 只读探测、发布包核验、安装状态刷新、安装/升级、活动版本回滚和删除指定非活动版本。脚本名固定白名单，参数通过 `ProcessStartInfo.ArgumentList` 逐项传递；版本、路径、目录重叠、磁盘根目录、非安装器内容和重解析点在 UI 边界先行拒绝。旧版本删除脚本也先复用状态核验，拒绝活动版本、当前回滚候选、损坏安装或未验证版本。
- `RequireSigned` 默认开启。安装/升级、回滚和删除旧版本在真实执行前自动运行同操作的 `-WhatIf` 并再次确认；脚本结果按有界 JSON 状态解释，不能只看退出码，真实变更无论成功或失败都会重新调用安装状态核验。
- 单次脚本 stdout/stderr 各限制为 256 KiB，默认 10 分钟超时。只读检查和 WhatIf 支持取消并终止进程树；真实写入开始后禁用手动取消和关窗，因为现有脚本没有协作式取消，强杀可能留下 staging、临时指针或半删除目录。错误只显示有界脱敏摘要，不持久化本机路径或凭据。
- Installer 使用 `net10.0-windows` framework-dependent WPF，并在启动时显式拒绝 Windows 10 2004 以前系统；这样不携带约 25 MiB 的未使用 WinRT 投影 DLL。发布目录 11 个文件、`319,324` bytes，其中 `GpAutoLive.Setup.dll` `64,000` bytes、`GpAutoLive.Setup.exe` `162,816` bytes，脚本与托管入口分离。
- 新增 14 项 Installer 测试，覆盖固定脚本/WhatIf/签名参数、回滚默认版本、非法版本、目录重叠、非专用安装目录、安装状态投影、无 `status` 的发布包清单成功投影、Runtime 缺失、路径脱敏、取消提示、进程取消、未知状态 fail-closed、非对象 JSON 拒绝和未知脚本拒绝；该增量当时全量自动化为 **417 项通过**（Contracts 25、Core 75、Media 91、Windows 176、App 36、Installer 14）。Release 构建 0 警告/0 错误，窗口启动/关闭冒烟 `WaitForInputIdle=True`、标题正确、`CloseMainWindow=True`、退出码 0。后续 C4-A/C4-B 增量已将全量更新为 422 项，见第 91、92 节。
- 既有 `artifacts/install-test-v83` 夹具验证当前 `previous_version` 删除 WhatIf 按预期拒绝，`versions/` 前后不变；发布目录中的 Runtime 探测脚本正常返回 `runtime_missing`，没有联网或写盘。
- 当前维护壳本身仍要求目标机先具备 .NET 10 Desktop Runtime，现有脚本又要求 PowerShell 7；Windows 10/11 默认 PowerShell 5.1 不满足。正式裸机在线安装仍需单独的受签名原生/自包含 Bootstrapper 或锁定 PowerShell 7 运行时，连同代码签名、权限提升/重启、安装中断恢复、完整卸载和干净机矩阵一起验收，不能把本增量标记为正式发布完成。
- 详细职责、操作语义和验证记录见 [`C7 WPF 安装维护壳实现记录`](./C7-WPF安装维护壳实现记录.md)。本增量未修改 `desktop/`，未删除用户代码或配置。

## 90. 主线程代码总监审查与覆盖修正（2026-09-03）

- 复核两个左侧并行任务的实际文件、构建产物、脚本参数和文档证据；确认 C7 安装维护壳没有引用主客户端程序集，C3–C5 矩阵不访问网络、不启动主程序，`desktop/` 跟踪文件保持未修改。
- 将 C4 离线矩阵补齐 `WindowsMpvPlaybackControllerTests`、`WindowsFfmpegPcmDecoderTests`、`WindowsAudioPlaybackControllerTests` 和 `WindowsAudioPauseGateTests`，8 行现为 158 项通过；真实媒体/设备/RTMP 仍由 `deferred_gates` 单独约束。
- 将 Installer 结果解释改为按操作白名单 fail-closed：未知状态、非对象 JSON 和不受支持的成功状态均不再显示为健康；进程取消/输出超限路径会观察并回收 stdout/stderr 任务，避免留下未观察的后台读取异常。
- 主线程复验：`dotnet format --verify-no-changes --no-restore`、Windows/Installer 项目级 Release 构建（0 警告/0 错误）和全量 422 项测试均通过；C3–C5 离线矩阵 158 项通过；统一方案构建因当前展示中的 `GpAutoLive` 进程锁定 App 输出 DLL，未强制关闭用户窗口，故未把该次方案级构建误报为通过；仍未把真实设备、代码签名或网络门禁标记为完成。

## 91. 当前实施增量：C4-A FFmpeg stderr 反压与取消回收修正（2026-09-03）

- `WindowsFfmpegPcmDecoder.DrainStderrAsync` 不再在 16 KiB 诊断计数达到上限时提前返回；读取缓冲仍固定为 4 KiB，计数封顶后持续消费并丢弃 stderr，直到流结束或运行取消，避免 FFmpeg 错误管道填满后反压卡死。
- stderr 仍不进入快照、错误正文或持久化文件；取消/超时路径仍按 Job Object/进程树、2 秒进程退出等待、stdout/stderr 读取任务 Join 和 finally 释放执行，固定内存不随错误输出增长。
- `WindowsFfmpegPcmDecoderTests` 新增 2 项 Windows `cmd.exe` 夹具回归：超过 16 KiB stderr 后进程成功退出；持续 stderr 期间取消在 4 秒内返回 `Cancelled` 且解码器不再运行。该夹具不需要 FFmpeg、网络或真实媒体资源。
- 本轮已执行：`dotnet test tests/GpAutoLive.Windows.Tests/GpAutoLive.Windows.Tests.csproj -c Release -p:Platform=x64 --no-restore --filter FullyQualifiedName~WindowsFfmpegPcmDecoderTests`，6/6 通过；主线程随后完成 Windows/Installer 项目级 Release 构建、全量 422 项回归与 158 项离线矩阵复验。

## 92. 当前实施增量：C4-B PortAudio 恢复资格与输入健康观察修正（2026-09-03）

- 修正 `WindowsPortAudioOutputStream.RestartAsync` 的恢复状态机：第一次重开会先关闭原生句柄，若随后打开失败仍保留“允许继续恢复”的内部资格；`WindowsPortAudioOutputRecovery` 的后续尝试不会因句柄已清空而被误判为初始启动，显式 `Stop`/`Dispose` 仍会清除该资格。
- `WindowsPortAudioOutputRecovery` 增加仅用于 Windows 测试的受控尝试入口，验证每次等待均使用取消令牌、失败按 `Retryable` 分类、最多执行策略指定次数（正式默认 3 次），不创建无界队列或无人管理任务。
- `WindowsPortAudioInputStream` 现在投影 `Pa_IsStreamActive`/`Pa_IsStreamStopped` 健康状态、回调计数和最后一次状态旗标；可选导出缺失时返回 `Unknown`，保留兼容旧版 PortAudio 的保守回退，不把缺失探针当作设备故障。
- `WindowsMicrophoneInterludeController` 的 50 ms 异步观察器消费输入健康快照；发现 `Stopped`、`Inactive` 或 `QueryError` 后进入 `Failed`、取消会话、清除麦克风优先级，并在线程池上执行有界输入停止，避免阻塞异步观察线程或留下活动原生流。输入不自动重开，需用户修复设备后重新启用。
- 新增输出恢复三次上限、首次失败后第二次重开分类和输入健康回退测试；显式 PortAudio DLL 夹具验证了真实句柄首次重开失败后第二次仍走 `ResourceMissing`，不再返回 `InvalidConfig`。本轮 Windows 项目测试 181 项通过，格式检查、全量 422 项回归和 C3–C5 离线矩阵 158 项复验通过。
- 真实声卡拔插、驱动重置、睡眠唤醒、PortAudio 原生调用硬超时和 30 分钟长稳仍是目标设备门禁；当前状态保持“代码已接入·待真实设备验收”。

## 93. 当前实施增量：v84 C4-A/C4-B 发布候选复验（2026-09-03）

- 基于 C4-A/C4-B 最新源码生成 `artifacts/csharp-windows-controller-20260903-v84`：根目录 8 个运行文件、`1,442,544` bytes，`GpAutoLive.exe` `162,816` bytes；GPU 运行库 7 个 DLL、`1,208,320` bytes，WinRT 运行库 2 个 DLL、`27,848,816` bytes，媒体运行时 13 个文件、逻辑大小 `352,365,694` bytes；独立 symbols 包 7 个文件、`476,847` bytes。
- v84 发布包使用 `verify-release-package.ps1` 完成根文件、GPU/WinRT/媒体 Manifest、大小和 SHA-256 核验；报告为 `artifacts/csharp-windows-package-verification-v84.json`，开发包仍按未签名状态处理，`-RequireSigned` 不得通过。
- 主线程已完成 `dotnet format --verify-no-changes --no-restore`、Windows 项目级 Release 构建、Windows/全量 422 项回归、C3–C5 离线矩阵 158 项和 `verify-scope.ps1`；v84 `smoke-csharp-windows-release.ps1 -PlanOnly` 通过。
- 在开发窗口释放后补跑 v84 真实启动/关闭冒烟：`WaitForInputIdle=True`、窗口标题 `GpAutoLive`、`CloseMainWindow=True`、退出码 `0`、无残留媒体进程，耗时约 `1.53s`；报告为 `artifacts/csharp-windows-release-smoke-v84.json`。该结果只覆盖本机启动/优雅关闭，不替代真实设备、网络、签名和 30 分钟长稳门禁。

## 94. 当前实施增量：C5 RTMP 有限重连协调器（2026-09-03）

- 新增 `GpAutoLive.Windows/WindowsRtmpReconnectCoordinator` 与 `WindowsRtmpReconnectPolicy`：一次断开事件最多 3 次尝试，退避固定为 250ms、500ms、1s；所有等待、尝试、异常和取消均有界，状态收敛为 `Reconnecting → Publishing/Failed`，取消回到 `Idle`。
- 协调器只接收上层注入的脱敏尝试函数，不访问网络、不保存地址/路径/stream key、不读取 FFmpeg 原文；`null` 或异常结果均映射为固定 `StartFailed` 并继续受预算约束。由于本地 FFmpeg `Process.Exited` 不能证明远端握手/断开，当前不自动接线到宿主，避免画面和 `pipe:0` 音频会话被错误分开重启。
- 新增 8 项 Windows 测试，覆盖成功、不可重试失败、三次耗尽、退避取消、尝试取消、并发拒绝、异常脱敏和空结果 fail-closed；Windows 测试总数为 189，C3–C5 离线矩阵已把该行纳入并增至 166 项。

## 95. 当前实施增量：C6 性能基线严格评估器与 v85（2026-09-03）

- 新增 `tools/evaluate-process-baseline.ps1`：只读取已有采样 JSON，固定字段/重复字段/类型/范围/重解析点均 fail-closed；用样本 `elapsed_ms` 计算真实观测跨度，不能用 `duration_seconds` 冒充 30 分钟证据；Idle CPU 使用至少 3 个有效样本的 nearest-rank P95。
- `NoPlayback30m`/`JointPlayback30m` 只有在真实跨度至少 1,800 秒、至少 30 个样本且内存增长未超预算时才会继续检查；跨进程采样不能伪造 GC 后稳定，始终要求应用内 GC/长稳证据。输出最多 64 KiB 并使用同目录临时文件原子替换；脚本显式要求 PowerShell 7。
- 固定性能夹具和 `test-evaluate-process-baseline.ps1` 已通过；使用已有短时 v83 基线验证为 `not_ready/failed`，没有把短样本误判为达标。全量解决方案 Release 构建 0 警告/0 错误，全量自动化测试为 430 项，格式检查、作用域检查和 C3–C5 离线矩阵 166 项均通过。
- v85 发布包根目录 8 个文件、`1,451,248` bytes，`GpAutoLive.exe` `162,816` bytes；发布包 Manifest/SHA-256 核验通过，真实启动/关闭冒烟 `WaitForInputIdle=True`、`CloseMainWindow=True`、退出码 `0`、无残留进程，约 `1.50s`。候选仍为未签名开发包，真实设备、RTMP/RTMPS、GPU/摄像头、安装签名和 30 分钟长稳仍待验收。

## 96. 当前实施增量：v85 本机真实媒体夹具复验（2026-09-03）

- 使用 v85 外置媒体运行库的真实 `mpv.exe`，临时生成 `320×180/30fps/2 秒` MP4，在 Windows 原生 HWND 上显式运行 `WindowsMpvRealFixtureTests`；`Original`、`Cpu4`、`Gpu83` 三种模式均各 `1/1` 观察到正播放时间、绝对 seek 和 `eof-reached`，测试进程由 finally 有界停止且无残留。
- 使用同一 v85 `ffmpeg.exe` 显式运行 `WindowsFfmpegPcmDecoderTests.Real_ffmpeg_fixture_decodes_into_bounded_ring_when_explicitly_enabled`，FFmpeg→固定容量 PCM 环缓 `1/1` 通过；临时媒体已清理。
- 该证据只覆盖本机单素材/单 HWND/单次运行，不提升为目标 GPU 矩阵、长 GOP/混排、真实声卡、ZLMediaKit/RTMPS 或 30 分钟长稳门禁；离线矩阵仍保持不启动真实夹具的可重复边界。

## 97. 当前实施增量：WPF RTMP 与性能职责拆分（2026-09-03）

- `MainWindow.Rtmp.cs` 现在单独承载 RTMP 配置校验、开始/停止推流、状态投影和媒体变更前停止逻辑；`MainWindow.Performance.cs` 单独承载性能采样、运行时偏好应用和输出模式显示。主窗口 partial 拆分不新增状态副本、不改变 XAML 事件名或行为。
- 新增 [`C5 WPF RTMP 界面职责拆分记录`](./C5-WPF-RTMP界面职责拆分记录.md)；拆分后 App Release 构建、解决方案格式检查、全量 **431 项**测试和 `verify-scope.ps1` 复验通过，`desktop/` 跟踪文件保持未修改。
- 当前开发端可继续查看：`src/GpAutoLive.App/bin/x64/Release/net10.0-windows10.0.19041.0/win-x64/GpAutoLive.exe`；现有窗口不因验证被强制关闭。拆分仅改善维护边界，不改变真实 RTMP、设备、签名和长稳门禁均待验收的状态。

## 98. 当前实施增量：C5 RTMP 宿主重连接线审计（2026-09-03）

- 审计确认 `WindowsRtmpReconnectCoordinator` 的注入尝试回调已经是当前最小安全接线边界；`WindowsRtmpOutputManager.Process.Exited` 只能证明本地 FFmpeg 退出，不能推断远端 RTMP/RTMPS 握手、鉴权或断开。
- `WindowsRtmpAudioSession` 同时拥有 PCM 解码器、固定容量总线、混音源和唯一分流泵；只重启画面宿主会破坏画面/声音组合，因此本轮没有把协调器自动接入宿主，也没有新增网络探测、隐式后台重连或敏感地址保存。
- 新增 [`C5 RTMP 宿主重连接线审计`](./C5-RTMP宿主重连接线审计.md)，规定未来生产接线必须由同一上层所有者完成远端断开确认、`RtmpSourceIdentity` 校验以及画面/声音成组停止和重建；协调器仍保持最多 3 次、250/500/1000ms 退避、可取消、并发拒绝和脱敏终态。
- 新增首次尝试前取消的契约测试；Windows RTMP 协调器测试为 **9/9** 通过，Windows 项目测试为 **190/190** 通过；`dotnet format --verify-no-changes --no-restore` 与 `git diff --check` 通过。真实 ZLMediaKit/RTMPS 断开信号和恢复门禁仍待验收。

## 99. 当前实施增量：C8 WPF 虚拟摄像头职责拆分（2026-09-03）

- 新增 `MainWindow.VirtualCamera.cs`，单独承载虚拟摄像头状态投影、安装/sidecar/D3D11/WGC 探测、启动/停止、刷新及最终效果 HWND 绑定；主窗口保留生命周期和跨功能编排。
- 拆分保持原有字段、XAML 事件、安装/签名/GPU/HWND 联合门禁及取消/有界清理语义，不新增状态副本，不启动未验收 sidecar，不修改 `desktop/`。
- App Release x64 独立输出构建 0 警告/0 错误，App 测试 36/36 通过，格式检查和 `git diff --check` 通过；临时构建目录已清理。真实 WGC/DirectShow/sidecar/多 GPU 门禁仍待验收。

## 100. 当前实施增量：C16 WPF 抖音 M1 职责拆分（2026-09-03）

- 新增 `MainWindow.Douyin.cs`，单独承载 M1 启动/暂停/恢复/停止、sidecar 快照与结果投影、配置加载/保存及配置存储创建；不改变扫码/网络仍待验收的实际状态。
- 拆分保持原有字段、XAML 事件、脱敏文本、队列容量和取消语义，不新增网络调用、不持久化凭据、不修改 `desktop/`。
- App Release x64 独立输出构建 0 警告/0 错误，App 测试 36/36 通过，格式检查和 `git diff --check` 通过；当前统一回归为 **431 项**（Contracts 25、Core 75、Media 91、Windows 190、App 36、Installer 14），C3–C5 离线矩阵为 **167 项**，真实 QR/协议/发送/自回显门禁仍待验收。

## 101. 当前实施增量：v86 候选与当前状态看板（2026-09-03）

- 基于最新 WPF partial 与 RTMP 宿主审计源码，生成 `artifacts/csharp-windows-controller-20260903-v86`；根目录保持 8 个运行文件、`1,451,248` bytes，`GpAutoLive.exe` `162,816` bytes，外置 GPU/WinRT/媒体运行库沿用已核验 Manifest。
- v86 发布包 Manifest/SHA-256 核验通过；由于当前开发窗口 PID 19612 持有默认输出 DLL，本轮未强制关闭窗口，v86 启动关闭仅执行 `PlanOnly`，v85 真实启动/关闭冒烟证据继续有效。
- 新增 [`C# Windows 当前状态看板`](./CSharp-Windows当前状态.md)，集中记录 431 项测试、167 项离线矩阵、当前开发窗口、已接入边界、未验收门禁和时间估算；不替代本计划的历史审计记录。

## 102. 当前实施增量：C9 WPF 音频/插话/麦克风职责拆分（2026-09-03）

- 新增 `src/GpAutoLive.App/MainWindow.AudioInterlude.cs`，承载固定话术、插话文件池和麦克风本地门控三组 WPF 编排职责；`MainWindow.xaml.cs` 继续唯一持有共享字段、生命周期、通用播放和跨功能状态，不复制控制器、配置或缓存。
- 固定话术迁移 Windows SAPI 启动/取消/完成观察与状态投影；插话迁移目录递归扫描、清空、播放/停止、优先级停止、完成观察、配置 JSON 读写和状态投影；麦克风迁移 PortAudio 设备枚举、输入流启停、快照事件、优先级抢占和状态投影。XAML `Click` 事件名保持不变，未修改 `desktop/` Rust/Tauri 参考目录。
- 新增 [`C9 WPF 音频界面职责拆分记录`](./C9-WPF音频界面职责拆分记录.md)。拆分后 App Release x64 独立构建 0 警告/0 错误、App 测试 36/36、解决方案全量回归 **431/431**、C3–C5 离线矩阵 **167/167**、`dotnet format --verify-no-changes`、`git diff --check` 和 `tools/verify-scope.ps1` 均通过；临时构建目录已清理。
- 本轮只调整 partial 文件职责，不提升固定话术真实 SAPI、声卡拔插/睡眠唤醒、可听混音、AEC/降噪/AGC 或长稳门禁的验收状态；这些仍需目标 Windows 设备验证。

## 103. 当前实施增量：v87 音频职责拆分候选（2026-09-03）

- 基于 C9 partial 拆分后的最新源码生成 `artifacts/csharp-windows-controller-20260903-v87`；根目录保持 8 个运行文件、`1,450,736` bytes，`GpAutoLive.exe` `162,816` bytes，外置 GPU/WinRT/媒体运行库沿用 v86 已核验资源。
- v87 发布包 Manifest/SHA-256 核验通过；由于开发窗口 PID 19612 仍占用默认 App 输出，本轮未强制关闭窗口，未执行真实启动/关闭冒烟，v85 真实冒烟证据继续有效。
- v87 仍是未签名开发候选；真实声卡、SAPI、RTMP/RTMPS、虚拟摄像头、抖音 sidecar、安装签名和 30 分钟长稳门禁保持未验收。

## 104. 当前实施增量：C8 WPF 播放职责拆分与媒体导入生命周期修复（2026-09-03）

- 新增 `src/GpAutoLive.App/MainWindow.Playback.cs`，承载播放/暂停/停止、上一项/下一项、键盘快捷键、mpv/音频完成观察、自然结束推进、播放命令串行闸门、进度投影和视频 seek；主窗口继续唯一持有字段、生命周期、媒体池编辑及跨功能停止编排。
- 修正 `RunImportAsync`：文件选择先于媒体运行库加载，取消选择不再触发 FFprobe/运行库初始化；导入或拖放开始探测前统一调用 `StopMediaForMutationAsync`，确保 RTMP、虚拟摄像头、mpv、纯音频、插话和观察者按既有有界顺序停止。
- 新增 [`C8 WPF 播放界面职责拆分记录`](./C8-WPF播放界面职责拆分记录.md)。审计确认 XAML 播放事件各唯一绑定，`RunPlaybackCommandAsync` 两个重载为预期，未修改 `desktop/`。
- App Release x64 独立构建 0 警告/0 错误、App 测试 36/36、解决方案全量回归 **431/431**、C3–C5 离线矩阵 **167/167**、格式检查、作用域检查和 `git diff --check` 均通过；临时构建目录已清理。

## 105. 当前实施增量：v88 播放职责拆分候选（2026-09-03）

- 基于 C8 播放 partial 与导入生命周期修复生成 `artifacts/csharp-windows-controller-20260903-v88`；根目录 8 个运行文件、`1,450,736` bytes，`GpAutoLive.exe` `162,816` bytes，外置 GPU/WinRT/媒体运行库沿用 v87 已核验资源。
- v88 发布包 Manifest/SHA-256 核验通过；开发窗口 PID 19612 仍在运行，本轮未强制关闭窗口，未执行 v88 真实启动/关闭冒烟，v85 真实冒烟证据继续有效。
- v88 仍是未签名开发候选；真实 ZLMediaKit/RTMPS、声卡/SAPI、虚拟摄像头、抖音 sidecar、签名安装和 30 分钟长稳门禁保持未验收。

## 106. 当前实施增量：C3 WPF 媒体池职责拆分（2026-09-03）

- 新增 `src/GpAutoLive.App/MainWindow.MediaPool.cs`，承载媒体池拖放、导入、选择、上移、下移、移除、清空、确认框、媒体变更前统一停止以及 FFprobe 运行时按需校验；`MainWindow.xaml.cs` 继续唯一持有共享字段、生命周期和跨功能状态。
- 保持媒体池原子编辑与导入顺序：文件选择/拖放候选先返回，取消时不初始化 FFprobe；确认请求后再按需校验运行资源，并在逐项探测前调用 `StopMediaForMutationAsync`。停止失败、取消、重复、超限或任一探测失败均保留旧播放池。
- 统一停止路径继续覆盖 RTMP、虚拟摄像头、mpv、PortAudio/FFmpeg、插话和观察者；XAML 事件名与 `MediaImportCoordinator`、`MediaPoolService`、`MediaRuntimeBoundary` 依赖方向保持不变，未修改 `desktop/` 或删除用户源码。
- 新增 [`C3 WPF 媒体池界面职责拆分记录`](./C3-WPF媒体池界面职责拆分记录.md)。App Release x64 构建 0 警告/0 错误，App 测试 **36/36**、解决方案全量回归 **431/431**、C3–C5 离线矩阵 **167/167**、格式检查、作用域检查和 `git diff --check` 均通过。

## 107. 当前实施增量：v89 媒体池职责拆分候选（2026-09-03）

- 基于 C3 媒体池 partial 与 v88 播放/导入生命周期修复后的最新源码生成 `artifacts/csharp-windows-controller-20260903-v89`；根目录 8 个运行文件、`1,450,736` bytes，`GpAutoLive.exe` `162,816` bytes，GPU/WinRT/媒体运行库继续独立于主 EXE 并沿用已核验 Manifest。
- v89 发布包 Manifest/SHA-256 核验通过，报告为 `artifacts/csharp-windows-package-verification-v89.json`；开发窗口 PID 19612 仍在运行，本轮未强制关闭窗口，未执行 v89 真实启动/关闭冒烟，v85 真实冒烟证据继续有效。
- v89 仍是未签名开发候选；真实 ZLMediaKit/RTMPS、PortAudio/SAPI、AkVirtualCamera、抖音 Conda/QR/协议、代码签名/干净机安装、Windows 10/11 和 30 分钟长稳门禁保持未验收。

## 108. 当前实施增量：C6 WPF UI 状态合并边界（2026-09-03）

- 审查确认媒体 `ListBox` 已启用 WPF UI 虚拟化与 Recycling，播放池由 Core 限制为最多 100 项；当前 UI 不解码缩略图、不保留原图，也没有日志/弹幕历史 `ObservableCollection`，因此这两类缓存不存在无界增长路径。后续增加对应功能时，必须先提供显示尺寸、数量/字节上限和回收测试。
- 性能采样仍由 `WindowsProcessPerformanceSampler` 单次读取，WPF 1Hz 低优先级计时器和 `_performanceSampleInFlight` 门禁只保留当前结果，不建立历史队列。
- 新增 `Features/Performance/LatestWinsAsyncUpdateQueue`，将麦克风和抖音 sidecar 的后台状态投影压缩为每来源最多一个待处理 UI 更新；异步更新串行执行，窗口关闭时清空 pending 并拒绝新更新。详见 [`C6 WPF 性能边界审查记录`](./C6-WPF性能边界审查记录.md)。
- 本轮只改变 UI 调度边界，不改变媒体、音频、抖音协议或外部 sidecar 的真实门禁；新增纯逻辑 App 测试覆盖替换、串行、关闭三类路径，真实 30 分钟长稳、60Hz 100 项滚动、设备和网络门禁仍待验收。

## 109. 当前实施增量：C7 发布安装事务并发门禁审查（2026-09-03）

- 发现安装、回滚、卸载脚本在多个维护壳/命令行实例并发运行时可能同时读取旧 `current.json`，后写入者覆盖前一个安装事务的回滚链；新增 `tools/csharp-windows-install-transaction-lock.ps1`，三条写操作共同取得 `Local\\GpAutoLive.CSharp.Windows.InstallTransaction.v1` 命名互斥。
- 事务锁立即尝试、占用时返回 `install_transaction_busy`，不创建 staging、不切换活动指针、不删除版本；异常退出由 Windows 自动释放，abandoned mutex 可安全接管。安装维护壳已将 helper 复制到固定 `tools/`，与命令行脚本保持同一边界。
- 新增 `tools/test-c7-install-transaction-lock.ps1`，在外部持锁时验证安装、回滚、卸载均 fail-closed；锁竞争和安装 WhatIf 未创建测试安装目录。该修正不修改 `desktop/`，不删除源码，也不改变主程序媒体资源锁。
- v89 包核验、`-RequireSigned` 预期拒绝、`-PlanOnly`、现有安装状态 read-only/WhatIf 均复验通过；当前 PID 19612 仍被保护，v89 真实冒烟保持 `blocked_preexisting_processes`，未强制关闭窗口。签名、UAC、裸机 Runtime/PowerShell 7、干净机安装、跨客户端双向锁协议和长稳仍待验收。详细记录见 [`C7 安装事务锁审查记录`](./C7-安装事务锁审查记录.md)。

## 110. 当前实施增量：v90 C6/C7 统一候选（2026-09-03）

- 基于 C6 latest-wins UI 状态合并与 C7 安装事务互斥后的最新源码生成 `artifacts/csharp-windows-controller-20260903-v90`；根目录 8 个运行文件、`1,453,296` bytes，`GpAutoLive.exe` `162,816` bytes，`GpAutoLive.dll` `215,040` bytes，外置 GPU/WinRT/媒体运行库继续独立并沿用已核验 Manifest。
- v90 发布包 Manifest/SHA-256 核验通过，报告为 `artifacts/csharp-windows-package-verification-v90.json`；`-RequireSigned` 对未签名开发包按预期拒绝，`-PlanOnly` 不启动进程、不写报告。
- 统一 Release 方案构建 0 警告/0 错误；全量测试 **434/434**（Contracts 25、Core 75、Media 91、Windows 190、App 39、Installer 14），C3–C5 离线矩阵 **167/167**，C6 基线评估器夹具和 C7 安装事务锁竞争测试均通过。
- 当前开发窗口 PID 19612 仍在运行，本轮未强制关闭，v90 真实启动/关闭冒烟保持未执行；真实设备、网络、签名、安装和 30 分钟长稳门禁不因候选生成而改变。

## 111. 当前实施增量：C7 媒体输出所有权与单实例竞态审查（2026-09-03）

- 审查确认 C# 启动顺序为 Windows 运行时/AppUserModelID 配置 → `Local\\GpAutoLive.MediaOutput.Owner.v1` 媒体/输出租约 → C# 单实例互斥 → 主窗口；`MainWindow.OnClosed` 先回收媒体/输出会话，`App.OnExit` 再释放两个互斥，失败路径均释放已取得的资源。
- 只读检查 `desktop/src-tauri/src/main.rs`：Rust/Tauri 仅注册自身 `tauri_plugin_single_instance`，没有消费 C# 媒体输出互斥名称，因此当前仍只能宣称 C# 端 fail-closed 和 Rust 进程只读探测，不能伪造双端双向锁协议。
- `WindowsMediaOutputOwnershipLease.TryAcquire` 在立即获取媒体互斥后新增一次参考进程二次探测；二次探测发现参考进程或发生可识别探测失败时，租约不转移并显式释放已持有互斥，收窄首次探测与锁获取之间的竞态。新增两项回归测试验证该路径及释放后重试。
- 所有权专项测试 **6/6**、Windows 项目测试 **192/192**、独立输出目录下全量方案测试 **437/437**；`dotnet format --verify-no-changes --no-restore` 与 `tools/verify-scope.ps1` 均通过。详细记录见 [`C7 媒体输出所有权与单实例审查记录`](./C7-媒体输出所有权与单实例审查记录.md)。
- 二次探测不是跨客户端原子握手；参考端若在二次探测之后启动仍无法由只读 C# 代码证明不存在竞态。双端共同协议、真实设备/网络争用、签名安装、Windows 10/11 和长稳门禁继续保持未验收。

## 112. 当前实施增量：C7 安装指针与 WhatIf 边界收口（2026-09-03）

- 安装脚本读取已有 `current.json` 时新增 schema、活动版本格式、`relative_path` 和 `previous_version` 校验；损坏指针在发布包校验后、任何 staging/指针写入前 fail-closed。
- 安装在版本目录已移动但活动指针写入失败或被取消时，清理本次新版本目录并保留旧 `current.json`；有效 `-WhatIf` 不创建安装根目录。
- 新增 `tools/test-c7-install-boundaries.ps1`，覆盖损坏指针预演拒绝、WhatIf 只读和激活失败清理；新增 Installer 结果投影回归，要求 `healthy` 状态同时具备 schema、活动版本、已验证版本清单及回滚候选一致性。
- Installer Release x64 构建 **0 警告/0 错误**、Installer 测试 **15/15**、PowerShell 全工具解析、边界测试、事务锁竞争测试和作用域检查均通过。方案级 `dotnet test --no-build` 为 **441/441**；方案级重新构建因开发窗口 PID 19612 锁定 App 输出 DLL 未完成，未强制关闭窗口。签名、UAC、裸机 Runtime、干净机安装、跨客户端双向锁协议和长稳仍保持 deferred。

## 113. 当前实施增量：C4/C5 主线程生命周期复核（2026-09-03）

- 代码总监复核发现 RTMP 快速退出路径可能在 PCM 写入仍持有串行锁时直接关闭 stdin；已移除该旁路释放，统一由 `Process.Exited` 的有界 PCM 回收路径处理，避免半回收和悬空写入。
- 自然退出清理现在取消并有界等待 stderr 读取任务，再释放进程、Job Object 和输入流；本轮不改变自动重连接线边界，也不把本地进程退出解释为远端 RTMP/RTMPS 断开。
- 当前源码六个测试项目独立 Release 回归 **444/444**（Contracts 25、Core 75、Media 96、Windows 194、App 39、Installer 15），随后方案级 Release 构建 **0 警告/0 错误**；Windows 10/11、真实设备/网络、签名安装和 30 分钟长稳继续保持 deferred。
- 本轮未重新生成 v90 发布包；复核结束时未检测到 `GpAutoLive.exe`（此前 PID 19612 已退出，本线程未强制终止），未修改 `desktop/`，未删除用户代码。

## 114. 当前实施增量：登录错误边界与 GUI 视觉对照审计（2026-09-03）

- `ControlPlaneAuthCoordinator` 修正登录/Refresh 永久失败未执行 `DeleteRefreshToken` 的问题；串行门等待阶段的取消现在返回 `CONTROL_PLANE_CANCELLED`，不会把 `TaskCanceledException` 冒泡到 WPF。Credential Manager 异常与控制面客户端异常分开映射，退出异常路径本地会话 fail-closed 清理。
- `LoginViewModel` 对控制面调用增加取消/异常收口，未配置 `AUTOLIVE_CONTROL_PLANE_BASE_URI` 时显示可执行的配置提示；未配置时仍不发网络请求。正式真实登录仍需要通过地址校验的 HTTPS 控制面和 Windows 实机联调。
- 新增 [`GUI视觉对照审计-20260903`](./GUI视觉对照审计-20260903.md)。对照 [`gpautolive-csharp-windows-gui-preview-20260902.png`](./assets/gpautolive-csharp-windows-gui-preview-20260902.png) 后，当前 WPF 三栏开发壳与目标图的导航带、左侧播放设置、中央参数标签区、右侧卡片层级和运行态内容均不一致，不能标记为 1:1；登录页也没有对应设计稿。
- 本轮新增 3 项控制面回归；Windows 测试为 **197/197**，全量源码测试目标更新为 **447/447**。未修改 `desktop/`，未执行代码清理，也未生成新的 v90 发布包。

## 115. 当前实施增量：固定远程测试控制面接入（2026-09-03）

- `ControlPlaneHttpClientOptions` 保留显式 `AllowDevelopmentTestHttp` 门禁，仅匹配 `http://101.96.208.132:9090/`；WPF 在 `test` 或 `development` 环境显式开启固定远程测试 HTTP 与 loopback HTTP，其他远程 HTTP 地址仍拒绝，正式 HTTPS 规则保持不变。
- WPF 启动配置支持 `AUTOLIVE_CONTROL_PLANE_ENV=test|development` 访问固定 `http://101.96.208.132:9090`；未设置开发/测试环境时不自动连接 HTTP 控制面，避免 Release 启动误连测试服务。
- 新增 3 项 HTTP 配置边界测试：固定远程地址默认拒绝、显式测试环境选项允许、其他远程地址仍拒绝。当前源码全量目标为 **450/450**（Windows 200 项）。
- 2026-09-03 对 `http://101.96.208.132:9090/`、`/api/v1/health`、`/api/v1/readyz` 进行只读检查均返回 `200`；未发送登录凭据，真实登录、设备激活、Refresh、心跳和网络异常矩阵仍待联调验收。
- 本增量未修改 `desktop/`、未新增第三方依赖、未生成新的 v90 发布包；正式部署继续要求 HTTPS。

## 116. 当前实施增量：GUI 第一轮结构对齐（2026-09-03）

- 根窗口尺寸基准调整为目标图实际 `1586×992`；根布局拆为标题栏 `44px`、导航/操作带 `54px`、工作区、播放栏 `72px`、状态栏 `44px`。工作区按约 `24:50:26` 三栏与 `10/12px` 间距布局，操作带与工作台均受登录门禁控制。
- 左栏拆为媒体池和“播放设置 / 本地有序循环”同级卡片；中央预览增加目标图对应的标题工具行、媒体状态行；下方从“快速参数”替换为四个 Tab、五类分类导航和视频处理参数首屏（亮度、对比度、饱和度、色相、锐度、伽马、曝光、增益、降噪、白平衡、去闪烁、色彩空间、缩放、裁剪）。
- 右栏扩大到约 26% 宽度；输出标题改为 `输出`，虚拟摄像头、固定话术、麦克风插话、抖音弹幕形成独立卡片；插话文件池、PortAudio 输出设备和抖音回复池收纳到可展开的高级区。既有 `x:Name`、`Click` 事件和控制器所有权保持不变。
- `App.xaml` 补充暗色 TextBox/ComboBox、参数卡和 Tab 按钮样式；播放底栏增加播放/暂停/停止/上一条/下一条的目标图顺序，并将状态文本移至独立底部状态栏。
- 本轮本地验证：方案 Release 构建 0 警告/0 错误；WPF 启动/关闭冒烟 `WaitForInputIdle=True`、优雅关闭、退出码 0；原生 WPF 截图接口在当前会话不可用，尚未完成 DPI/像素级 1:1 验收。
- 本增量未修改 `desktop/`、未新增第三方依赖、未生成新的 v90 发布包；参数首屏当前是 UI 结构对齐，不能标记为全部正式参数已生效。

## 117. 当前实施增量：GUI 第二轮状态与卡片密度修正（2026-09-03）

- 标题栏增加产品标识，并由认证状态真实投影设备授权状态、已成功登录账号和本地化的授权到期时间；未授权或仅输入未提交账号时显示占位符，不伪造授权事实，退出登录入口保留。
- RTMP、虚拟摄像头、固定话术、麦克风插话和抖音弹幕右上角徽标改为复用各自现有状态机；状态会随启动、运行、暂停、失败和待机更新，不新增第二份业务状态。
- 右侧卡片按目标图顺序调整为 RTMP→虚拟摄像头→麦克风→固定话术→抖音弹幕；采用更接近目标图的紧凑内边距和间距，RTMP补充分辨率/帧率/音视频码率的紧凑字段，摄像头控制合并为单行，麦克风补充禁用态阈值视觉位，固定话术与抖音主卡压缩；RTMP 输出选项、插话文件池和抖音回复池移到首屏之后的高级区，既有 `x:Name`、`Click` 事件和控制器所有权保持不变。
- `App.xaml` 增加 WPF Button 自定义暗色模板，统一鼠标悬停、按下和禁用态，修复系统主题把禁用按钮绘成白块的问题；ComboBox、PasswordBox、Slider 和 ListBox 选中态继续使用暗色模板/资源。
- 媒体池条目增加与 `SourceMediaDto` 快照绑定的类型图标、文件名、时长、视频宽高/帧率或音频采样率/声道；中央预览状态文案跟随无媒体、就绪、播放、暂停和纯音频状态变化。真实首帧缩略图和主工作台内嵌视频表面不在本轮伪造。
- WPF `ListBoxItem` 增加深色选中/悬停模板，避免系统默认蓝色高亮破坏目标图的暗色边框层级；媒体列表继续使用原有 Recycling 虚拟化。
- 本轮未修改 `desktop/`、未新增第三方依赖、未删除用户代码或生成新的 v90 发布包。显式临时 SDK `C:\Users\asd49\AppData\Local\Temp\dotnet-sdk-10.0.400\dotnet.exe` 下：方案 Release 构建 0 警告/0 错误，全量测试 **450/450**，`dotnet format --verify-no-changes` 和 `tools/verify-scope.ps1` 通过；WPF `WaitForInputIdle=True` 后通过 `WM_CLOSE` 优雅退出；临时授权态布局渲染夹具生成 `.tmp/wpf-capture/.../gpautolive-authorized-layout.png`。真实首帧缩略图、主工作台内嵌视频表面、DPI/像素级 1:1 和参数控件与正式 `MediaEffectParams` 的绑定仍待实施/验收。

## 118. 当前实施增量：参数区首批真实草稿绑定（2026-09-03）

- 新增 `VideoEffectEditorState` 作为 WPF 参数草稿所有者，亮度、对比度、饱和度和色相旋转绑定到现有 `MpvVideoEffectSnapshot` 的默认值与范围：`-100～100`、`0～200`、`0～200`、`-180～180`；“恢复已接入默认”按钮只重置这四项。
- 锐度、伽马、曝光、增益、降噪、白平衡、去闪烁、色彩空间、缩放和裁剪暂时禁用并显示“待接入”，因为当前 C# 播放链没有对应的正式消费者；本轮不把静态控件误标成已生效。
- 参数草稿已具备映射到有限 `MpvVideoEffectSnapshot` 的校验入口，但尚未接入 `WindowsMpvPlaybackController` 的显式运行时提交，因此仍需后续补齐“草稿→应用→mpv IPC→已应用回显”链路。
- 本增量新增 App 单元测试覆盖默认值、范围钳制和 CPU4 快照映射；未修改 `desktop/`、未新增第三方依赖、未删除既有代码。

## 119. 当前实施增量：工作区骨架与播放栏尺寸校正（2026-09-03）

- 根行调整为 `45/56/*/74/44`，三栏调整为 `24:49.5:26.5`，栏间距收紧至 `8px`，播放设置卡片高度调整为 `224px`，以匹配量测到的目标图边界。
- 底部播放栏增加 `10px` 外边距和 `48px` 固定按钮高度，补充当前媒体名称区，进度轨道收窄为 `6px`，音量轨道调整为 `140px`；播放池缩略图列宽修正为可容纳目标图的 `96px` 缩略图。
- 以上是布局/视觉修正，不改变“一个活动媒体源、一个最终效果窗口、一个 mpv”的播放所有权；真实窗口 DPI 和像素级结果仍需目标机手工验收。

## 120. 当前实施增量：媒体池首帧缩略图链（2026-09-03）

- 新增固定 FFmpeg 单帧命令计划与 Windows 提取适配器：仅使用已验证的 `ffmpeg.exe`，输出固定 `192×108` JPEG，使用参数数组、`-nostdin`、12 秒超时、标准流上限和取消/进程树清理，不把二进制图片写入标准输出。
- 新增媒体列表 UI 投影和有界缩略图缓存：缓存最多 64 项/16MiB，导入或媒体池变更后异步加载，WPF 使用 `BitmapCacheOption.OnLoad` 后冻结图片；失败、取消、关闭和路径失效均回退到播放/音频图标，不改变 Core 播放池原子快照。
- 新增媒体命令构造单元测试；尚未使用真实媒体运行包执行 FFmpeg 首帧实机测试，因此真实缩略图质量、编解码兼容、滚动 Recycling 和长时间缓存行为仍待验收。
- 本轮最终回归：方案 Release 构建 0 警告/0 错误，全量测试 **454/454**（Contracts 25、Core 75、Media 98、Windows 200、App 41、Installer 15），`dotnet format --verify-no-changes`、作用域检查和 WPF 启动/关闭冒烟均通过。

## 121. 当前实施增量：GUI 第三轮右栏密度与中央确认预览（2026-09-03）

- 右栏继续按目标图的五张卡片做定点对齐：RTMP 保留地址/流密钥/分辨率/帧率/音视频码率，并补充复制地址、显示/隐藏流密钥和重连入口；虚拟摄像头补充设备名称、固定规格和同一行规格提示；麦克风补充输入设备标签；固定话术补充管理与自动循环视觉位。卡片内边距、间距和高度进一步收紧，抖音卡片完整落在播放栏上方，未删除既有控制器事件或隐藏真实可用的启动入口。
- 中央预览新增确认缩略图层：只复用媒体列表当前项已经由异步缓存确认的 `ImageSource`，无缩略图时保留占位状态；不创建第二个播放器、第二个 HWND 或第二个 mpv 会话，最终视频仍由独立最终效果窗口承载。
- 媒体缩略图缓存新增 UI 完成回调，首帧回填后刷新中央预览；当前夹具通过目标图预览区的内存裁剪图验证真实显示层级，仍未把裁剪图当作 FFmpeg 实机首帧证据。
- 新增 `src/GpAutoLive.App/Properties/launchSettings.json`：提供 `GpAutoLive - 远程测试控制面`（`AUTOLIVE_CONTROL_PLANE_ENV=test` + 固定 `http://101.96.208.132:9090`）、`GpAutoLive - 本地开发控制面`（`AUTOLIVE_CONTROL_PLANE_ENV=development` + 同一固定远程 HTTP 地址）和本地离线壳启动档；正式环境的 HTTPS 与固定远程 HTTP 门禁未放宽。
- 本轮远程只读复查：`/`、`/api/v1/health`、`/api/v1/readyz` 均返回 `200`；桌面登录路由可达但未提交真实账号、密码或 Token，真实登录→自动激活→Refresh→退出仍待用户提供测试账号后的联调验收。
- 本轮最终回归：方案 Release 构建 **0 警告/0 错误**，全量测试 **454/454**（Contracts 25、Core 75、Media 98、Windows 200、App 41、Installer 15），`dotnet format --verify-no-changes --no-restore`、`tools/verify-scope.ps1`、普通配置启动/关闭冒烟和远程测试配置启动/关闭冒烟均通过；未修改 `desktop/`、未新增第三方依赖、未生成新的 v90 发布包。

## 122. 当前实施增量：GUI 第四轮预览时间与抖音卡片对齐（2026-09-03）

- 中央预览状态栏补充当前时间/总时长和播放状态投影，复用已有受身份保护的进度快照；底部播放栏与媒体列表的有效时长统一为 `HH:MM:SS`，相应更新纯逻辑格式化测试，未改变 seek、EOF 或媒体所有权。
- 媒体列表的视频缩略图统一保留目标图中的播放叠加位；抖音卡片补充只读弹幕服务器地址、房间 ID 同行的断开连接入口，并保持真实 `StopDouyinButton_Click` 状态机路径，不新增第二份抖音会话状态。
- 本轮回归：方案 Release 构建 **0 警告/0 错误**，全量测试 **454/454**，`dotnet format --verify-no-changes --no-restore`、`tools/verify-scope.ps1`、`git diff --check`、XAML 86 个命名元素无重复；普通启动/关闭和固定远程测试地址启动/关闭冒烟均通过。远程只读检查 `/`、`/api/v1/health`、`/api/v1/readyz` 当前均为 **200**。
- 本轮仍未修改 `desktop/`、未新增第三方依赖、未删除用户代码或把真实凭据写入配置；下一步仍需真实媒体首帧、Windows DPI/像素截图和真实测试账号登录联调。

## 123. 当前实施增量：GUI 第五轮底部处理列与卡片边界校准（2026-09-03）

- 中央参数 Tab 改为自然宽度排列，预览工具栏补齐目标图中的图标/间距；底部播放栏预留固定的处理开关与音量列，并用复用绑定状态的暗色开关模板呈现视频处理、声音处理，未改变两个独立处理开关的所有权。
- 使用目标图与授权态夹具图的同坐标边界采样回调右栏：麦克风、固定话术和抖音卡片分别落在目标图的垂直区间附近；该夹具仍是伪造元数据/设计图内存裁剪，不代表真实媒体首帧或真实设备状态。
- 本轮最终验证：方案 Release 构建 **0 警告/0 错误**，全量测试 **454/454**；`dotnet format --verify-no-changes --no-restore`、`tools/verify-scope.ps1`、`git diff --check`、XAML 86 个命名元素无重复；普通与远程测试配置启动/关闭冒烟均通过。
- 本轮仍未修改 `desktop/`、未新增第三方依赖、未删除用户代码或把真实凭据写入配置；真实媒体首帧、Windows DPI/像素截图和真实测试账号登录联调仍是剩余验收项。

## 124. 当前实施增量：参数区目标默认值视觉收口（2026-09-03）

- 锐度、伽马、曝光、增益、降噪、白平衡、去闪烁、色彩空间、缩放模式和裁剪改为显示设计图默认值；这些控件仍 `IsEnabled=False`，并通过 Tooltip 说明当前没有正式 C# 播放运行时消费者，未改变“仅四项参数已接入草稿绑定”的事实边界。
- 授权态夹具截图确认中央参数首屏的数值、卡片列宽、底部处理开关以及右栏五卡纵向边界均已按目标图收口；截图仍不代表真实媒体、设备或登录状态。
- 本轮仍未修改 `desktop/`、未新增第三方依赖、未删除用户代码；真实参数运行时提交、真实首帧、Windows DPI/像素级截图和真实测试账号登录联调继续保持未验收。

## 125. 当前实施增量：目标图主题色阶校准（2026-09-03）

- 根据目标图与授权态夹具图同坐标采样，将 WPF 工作区缝隙、卡片、控件和强调色收敛到目标暗色层级：窗口 `#080B10`、卡片 `#151A22`、抬升表面 `#1E232B`、强调色 `#068F6D`；主按钮文字改为浅色，保持目标图对比度。
- 右栏“输出”标题左内边距改为与卡片内容一致；本轮仅调整视觉资源和布局，不改变登录、媒体、播放、输出或状态机逻辑。
- 本轮最终回归：方案 Release 构建 **0 警告/0 错误**，全量测试 **454/454**，`dotnet format --verify-no-changes --no-restore`、`tools/verify-scope.ps1`、`git diff --check`、XAML 86 个命名元素无重复；普通与固定远程测试地址启动/关闭冒烟均通过。
- 本轮仍未修改 `desktop/`、未新增第三方依赖、未删除用户代码；真实 WPF DPI/像素截图、真实媒体/设备状态和登录闭环仍待验收。

## 126. 当前实施增量：v2 双页面 UI 设计基准与实施拆解（2026-09-03）

- 已将 [`首屏工作台设计稿`](./assets/gpautolive-csharp-windows-gui-v2-main-20260903.png) 与 [`下滑参数与状态续页设计稿`](./assets/gpautolive-csharp-windows-gui-v2-scroll-20260903.png) 确认为 C# 授权态主工作台的正式 UI 基准；两张图均按 `1586×992` 设计，第二张不是新路由，而是首屏中央参数区域向下滚动后的连续状态。
- v2 设计覆盖顶栏、操作带、左侧媒体池/播放设置、中部实时预览/参数工作区、右侧五张输出卡、底部播放控制栏以及中央下滑区域。旧版 `gpautolive-csharp-windows-gui-preview-20260902.png` 只保留历史对照，不再指导参数编辑区实现。
- 参数语义正式收口为：视频和普通声音效果值由系统按周期自动/随机生成，UI 只读展示“自动生成 / 只读快照 / 已生效”；用户可修改的是周期范围、预设池、设备、输出地址等规则或运行配置，不能直接修改本周期结果。
- 首屏中央参数区必须实现视频周期快照摘要、只读值卡片、当前轮次/GPU83/源素材、`重新生成本周期参数` 受控动作和中央内部滚动条；不得继续把生成结果实现为可拖动 Slider、可编辑数值框或仅靠 `IsEnabled=False` 伪装的占位控件。
- 下滑中央区域必须实现普通声音周期快照、预设池/周期范围规则控件、高级视觉只读快照、实时频谱只读图、播放与设备状态、脱敏有界事件日志和说明卡；左右栏、顶栏和底部播放栏在滚动时保持稳定上下文。

### 126.1 实施顺序

1. **UI-1 工作台壳和滚动边界**：按设计稿固定 `1586×992` 基准、顶栏/操作带/三栏/底栏的网格、卡片层级、间距、颜色和中央独立 `ScrollViewer`；首屏与下滑续页共享同一窗口，不新增播放器、窗口或业务状态。
2. **UI-2 视频只读快照卡**：拆分视频周期快照的视图模型与展示组件，绑定真实生成快照、周期、轮次、GPU 路径、源素材和应用状态；删除结果编辑控件的调用入口，保留规则/重新生成动作边界。
3. **UI-3 声音周期快照与规则区**：增加普通声音只读快照和预设池/周期范围的有限编辑区；把声音生成、预设选择、刷新和失败状态接入同一快照所有者，不复制 Rust 或 C# 的第二份随机状态。
4. **UI-4 高级视觉、设备状态和日志**：补齐 `65Hz–20000Hz`、微扰、GPU83→CPU4、mpv 单窗口、PortAudio 总线、RTMP 直推和有界脱敏事件日志的投影；“已生效”必须来源于真实运行时确认。
5. **UI-5 交互和状态门禁**：验证内部滚动、键盘焦点、缩放、空值、离线、取消、生成失败、输出失败和窗口关闭清理；独立视频处理/声音处理开关继续由各自状态所有者控制。
6. **UI-6 视觉验收**：在 Windows 10/11 x64 目标机以 `1586×992`、目标 DPI 和授权态分别截取首屏/下滑页，逐区比对边界、间距、颜色、文本层级和滚动位置；真实媒体、设备、GPU、RTMP 和登录闭环必须单独验收，不用设计稿夹具冒充。

### 126.2 v2 验收条件

| 验收项 | 必须满足 |
| --- | --- |
| 首屏结构 | 顶栏、操作带、左栏、中栏预览/参数、右栏五卡和底部播放栏与首屏设计稿的区域顺序和层级一致 |
| 下滑结构 | 中央滚动条处于下方位置，声音、高级视觉、设备状态、日志和说明卡完整可见；左右上下文保持一致 |
| 结果只读 | 视频/声音生成结果没有 Slider、InputNumber 或可编辑结果字段；规则控件与结果卡视觉上明确分离 |
| 生成动作 | 重新生成只提交已校验规则，成功后刷新快照，取消/失败保持旧快照并显示稳定错误 |
| 生效状态 | 只有运行时确认真实应用后才显示“已生效”，不可用默认值或静态夹具伪造 |
| 独立开关 | 视频处理和声音处理可独立显示和切换，不互相覆盖状态 |
| 资源边界 | 不新增第二个播放器、mpv、声音 Worker、输出会话或无界日志集合 |
| 设计证据 | 真实 WPF 窗口截图通过尺寸/DPI/逐区对照；设计稿文件和实现记录可从本计划追溯 |

本节只更新设计和实施口径，尚未声称 v2 UI 已经完成，也未修改 Rust/Tauri `desktop/`、未删除既有代码或把服务端作为构建环境。

## 127. 当前实施增量：v2 双页网格与系统生成只读快照（2026-09-03）

- `MainWindow.xaml` 已按两张 v2 设计稿收口主网格：工作区三栏调整为约 `23.2:50.3:26.5`，栏间距为 `10px`，操作带为 `53px`；中栏预览与参数区按约 `0.96:1.04` 分配，参数区使用独立的中央 `ScrollViewer`，不新增窗口、播放器或业务状态。
- 首屏中央参数区已替换为视频周期只读快照：周期、当前轮次、GPU 路径、当前媒体源和两行八项视频结果卡（亮度、对比度、饱和度、色相、锐度、伽马、曝光、降噪）；结果卡无 Slider、Input 或 ComboBox 写入口，`重新生成本周期参数` 仅执行本地有界随机快照更新，不接受手工效果值，也尚未提交给真实生成器或播放运行时消费者。
- 视频八项结果卡已按 v2 视觉模板调整为“图标/标签在上、放大只读数值与迷你指示在下”的结构；新增的展示样式只负责排版，不改变系统生成快照的所有权、范围或生效语义。
- 新增 `GeneratedVideoEffectSnapshot` 与 `GeneratedAudioEffectSnapshot`，由 `ShellState` 在桌面壳启动时使用有界随机范围生成一次只读结果；视频亮度/对比度/饱和度/色相/锐度/伽马/曝光和声音当前预设均不再绑定旧编辑草稿。该快照目前仅用于正确表达系统生成语义，尚未接入周期调度、普通声音 DSP 或 mpv `已应用` 回显，因此界面仍显示“待接入”，不得显示“已生效”。
- 中央下滑续页已加入声音周期快照、预设池摘要、场景与合成能力状态、播放/设备状态和有界状态日志区域；进入下滑态后隐藏分类导航与 Tab 按钮，仅保留居中的“向下滚动 · 参数与状态”提示，并将声音八项结果、场景五项能力和设备六项状态分别排成设计稿要求的单行卡片。
- 下滑续页的声音规则控件目前只读禁用，真实规则编辑、实时频谱数据和说明卡仍需接入真实所有者后再开放；`当前源素材` 卡片绑定现有当前媒体投影，空池时显示占位符，不把夹具数据冒充真实运行时事实。
- 右侧五张输出卡已按 v2 主卡语义收口：主卡保留配置、状态和设计稿要求的主入口；虚拟摄像头、麦克风、固定话术和抖音 M1 的启动/暂停/朗读等已有操作移动到同一右栏的折叠“高级控制”区，未删除事件处理或状态投影。
- 标题栏品牌标识改为复用项目现有 `desktop/ui/public/app-icon.png` 的 WPF 内嵌资源，设备授权由动态颜色状态点和文字组成；账户、授权到期和性能元信息保留动态绑定，不用静态授权值冒充登录成功。
- `App.xaml` 增加快照卡、状态徽标和暗色中央滚动条样式；底部声音处理开关不再与分隔线重叠；右栏输出标题和麦克风/抖音卡片内边距按 v2 首屏密度收紧，既有 `x:Name`、Click 事件和输出控制器边界未改变。
- 本轮本地验证：方案 Release 构建 **0 警告/0 错误**；全量测试 **456/456**（Contracts 25、Core 75、Media 98、Windows 200、App 43、Installer 15）；授权态 WPF 夹具已生成首屏 [`gpautolive-authorized-layout.png`](../.tmp/wpf-capture/bin/Release/net10.0-windows10.0.19041.0/win-x64/gpautolive-authorized-layout.png) 与下滑页 [`gpautolive-authorized-scroll-layout.png`](../.tmp/wpf-capture/bin/Release/net10.0-windows10.0.19041.0/win-x64/gpautolive-authorized-scroll-layout.png)，确认三栏、预览/参数比例、首屏八项随机只读结果卡、下滑卡片列数和底部处理开关。
- 本增量未修改 Rust/Tauri `desktop/`、未新增第三方依赖、未删除既有代码、未把源码放到服务器构建；真实周期刷新、声音运行时、`已生效` 回显、规则编辑、真实频谱和目标 DPI/像素级验收仍未完成。

## 128. 当前实施增量：v2 视觉夹具媒体缩略图收口（2026-09-03）

- 临时 WPF 授权态夹具改为读取 v2 首屏设计稿，并按左侧媒体池六个实际位置裁剪六项缩略图；中央预览另使用设计稿预览区裁剪图，避免把小尺寸媒体卡片放大后误当作真实预览画面。
- 为保证逐项视觉比对稳定，夹具只在截图进程内注入设计稿示例的固定视频/声音快照值（例如亮度 `2%`、对比度 `98%`、声音预设 `p07`）；生产 `ShellState` 仍通过 `Generated*EffectSnapshot.Create` 按周期随机生成，未改变用户不可直接修改结果的边界。
- 该调整只用于验证左侧媒体池缩略图层级、播放叠加位和中央预览比例，不改变生产 `MediaThumbnailCache` 的 FFmpeg 首帧链，也不把设计稿裁剪图当作真实媒体解码证据。
- 本轮重新生成首屏/下滑截图并确认右栏、底部栏、中央滚动续页未回归；真实媒体首帧、目标 DPI/像素级采集和登录闭环仍保持未验收。

## 129. 当前实施增量：顶栏设置入口与退出路径对齐（2026-09-03）

- 顶栏设置入口调整为设计稿的“齿轮 + 设置”`64px` 按钮，使设置、最小化、最大化和关闭控件的右侧起点与 v2 主图一致。
- 授权态顶栏不再展示独立“退出登录”按钮；退出动作迁移到已有汉堡菜单，仍复用 `LogoutButton_Click` 和登录状态机，未删除或绕过 Logout 能力。
- 本轮只调整 WPF 顶栏布局与入口位置；真实登录、设备状态和性能采样仍按独立验收口径处理。

## 130. 当前实施增量：媒体池条目文案与格式对齐（2026-09-04）

- 媒体池条目按 v2 主图收口显示：视频为时长与 `宽×高 画幅比`，音频为时长与 `kHz 声道类型`；移除缩略图上的重复媒体类型文字，类型信息保留在缩略图 Tooltip 和底层媒体快照中。
- 新增媒体池视图模型格式化测试，覆盖 `1920×1080 16:9` 和 `48kHz 立体声`；本轮全量测试为 **458/458**。
- 本轮只改 WPF 视觉投影与纯格式化逻辑，不改变 `SourceMediaDto`、FFmpeg 探测、播放顺序、缩略图缓存或用户不可直接修改系统生成参数的边界。

## 131. 当前实施增量：媒体池筛选工具栏列宽对齐（2026-09-04）

- 左侧媒体池顶部工具栏按 v2 主图重新分配列宽：筛选框 `102px`、间距 `10px`、搜索框 `164px`，保持左栏总宽度和媒体列表垂直边界不变。
- 本轮只调整 XAML 网格尺寸，不改变搜索框当前只读占位、媒体导入、筛选或播放池逻辑。

## 132. 当前实施增量：媒体池条目垂直节奏对齐（2026-09-04）

- 媒体池条目底部间距由 `4px` 调整为 `9px`，使六项缩略图起始位置从约 `68px` 节奏收敛到约 `73px`，与 v2 主图的六项媒体卡片纵向分布一致。
- 本轮只调整列表项视觉间距，不改变 100 项上限、WPF Recycling 虚拟化、媒体顺序或缩略图缓存边界。

## 133. 当前实施增量：v2 基础暗色阶像素校准（2026-09-04）

- 根据 v2 主图与最新 WPF 截图的同坐标采样，基础窗口底色调整为 `#080E15`，主卡面调整为 `#121A22`，抬升面调整为 `#1C232C`；标题栏、操作带、中央预览头部和底部状态带同步向目标图的蓝黑色阶收口。
- 本轮只调整共享画刷和既有区域背景，不改变控件状态、登录/媒体/输出逻辑或系统生成参数语义；右栏真实设备状态仍不通过静态颜色伪造。

## 134. 当前实施增量：中央参数卡色阶像素校准（2026-09-04）

- 同坐标采样确认中央参数区外层与结果卡仍比 v2 目标偏亮；`SnapshotSectionCardStyle` 调整为 `#111921`，`SnapshotValueCardStyle` 调整为 `#181F27`，统一覆盖视频、声音、场景和设备状态卡。
- 本轮只调整已有共享样式，不改变参数快照的只读边界、随机生成所有权、滚动结构或右栏功能状态。

## 135. 当前实施增量：顶栏授权状态图标对齐（2026-09-04）

- 顶栏设备状态由圆点改为矢量“盾牌 + 勾”图标，图标填充仍绑定 `DeviceStatusText.Foreground`，因此授权、未授权和异常颜色继续由真实状态投影控制。
- 本轮只调整顶栏视觉语义与 45px 标题栏注释，不伪造登录/授权状态、不改变汉堡菜单中的退出登录路径；本地 Release 构建与 v2 首屏/下滑截图夹具已通过。

## 136. 当前实施增量：右栏功能卡标题图标对齐（2026-09-04）

- 右栏 RTMP、虚拟摄像头、麦克风插话、固定话术和抖音弹幕卡片补齐与 v2 主图一致的语义标题图标；图标使用 WPF `Path` 矢量绘制，不引入新的图标依赖。
- 图标只属于标题视觉层，五张卡的状态文本、禁用态、事件处理、高级控制折叠区和真实能力未改变；本轮重新验证了首屏和下滑截图，未改变两张稿的滚动结构。

## 137. 当前实施增量：中央滚动条方向校准（2026-09-04）

- 修正共享 WPF `ScrollBar` 模板的垂直 `Track.IsDirectionReversed`，使首屏 `VerticalOffset=0` 时滚动拇指位于顶部、下滑至末尾时位于底部，与 v2 主图的首屏/续页方向一致。
- 该修正影响中央参数区和其他复用此模板的垂直滚动条的视觉位置及分页方向，不改变内容顺序、滚动事件、参数生成或右栏功能状态；修正后已重新生成两张截图。

## 138. 当前实施增量：滚动拇指灰阶校准（2026-09-04）

- 共享滚动条拇指由强调绿色调整为接近 v2 设计稿的中性灰 `#7E8286`，保留圆角和可见反馈；强调绿色继续只用于有效状态、开关和操作按钮。
- 本轮只调整滚动控件视觉层，不改变滚动范围、偏移映射、中央内容或其他卡片状态。

## 139. 当前实施增量：下滑提示方向标识对齐（2026-09-04）

- 下滑态中央提示由单行文字调整为“← 向下滚动 · 参数与状态 →”，补齐设计稿两侧方向引导；提示仍只在中央参数区滚动偏移超过阈值时显示。
- 本轮只调整提示文案的视觉投影，不改变 `ScrollChanged` 状态切换、声音/高级视觉续页或滚动偏移逻辑。

## 140. 当前实施增量：中央预览内部比例校准（2026-09-04）

- 按两张设计稿同坐标采样，将中央预览行比例从 `0.96*:1.04*` 调整为 `0.965*:1.035*`，在 `1586×992` 基准下把预览卡整体增加 `1px`，使参数区从目标 `y=477` 开始。
- 预览卡内部标题/画面/媒体状态行调整为 `43px / * / 38px`，使设计稿 `759×276` 预览画面按原比例显示，媒体状态栏获得对应高度；下滑态仍由现有滚动状态折叠预览。

## 141. 当前实施增量：底部播放栏与处理状态布局校准（2026-09-04）

- 底部播放栏继续保持 `1586×992` 基准下的 `74px` 工作区高度；播放、暂停、停止、上一条、下一条按钮按设计稿收口为 `82/82/82/88/88px`，次级按钮使用深色抬升面，保留原有播放事件、快捷键和状态绑定。
- 中央播放信息拆成两行：上行显示当前媒体，下行显示约 `320px` 进度轨道和 `当前位置 / 总时长`；轨道起点、时间文本和按钮右边界按主图同坐标校准，不改变 seek、EOF 或播放池所有权。
- 视频处理和声音处理开关改为“标题 + 状态副标签 + 开关”纵向结构，状态分别绑定 `VideoProcessingLabel` 与 `AudioProcessingLabel`；音量轨道调整为 `120px` 并显示当前 `80%` 投影，使右侧控件与设计稿的分隔线和密度一致。
- 本轮只调整 WPF 底部布局与现有状态投影，不新增音量业务所有者、不把设计稿静态值当作实际音频输出能力；真实音量控制仍需接入播放/声音总线后单独验收。

## 142. 当前实施增量：操作带、快照格式与右栏卡片几何校准（2026-09-04）

- 顶部导航/操作带按设计稿收口为 `53px` 行高；首页/工作区按钮与中央五个操作按钮统一使用 `36px` 高度，中央五个工具按钮宽度为 `110/102/100/98/100px`，保持既有导入事件与右侧输出入口不变。
- 视频亮度、曝光以及普通声音增益、动态范围、低频等正向结果按设计稿显示显式 `+` 号；这只是数值格式化，不改变快照数值、随机范围或用户不可直接编辑的边界。
- 右栏授权态静态网格的摄像头、麦克风、固定话术和抖音卡片分别收口到约 `107/146/95/136px` 的内容高度，减少卡片累计误差，使抖音卡片在设计稿底部位置结束；状态、按钮启用态和输入内容仍由真实运行时控制，未用夹具伪造推流/摄像头/抖音连接。
- WPF 截图夹具同步使用设计稿中的声音示例 `+1.5 dB`、`+8.2 dB`、`+2%`、`-1%`、当前轮次 `03`，仅用于静态视觉对照；生产 `ShellState` 仍按实际生成逻辑创建随机只读快照。

## 143. 当前实施增量：中央参数区 Tab 与结果卡基线校准（2026-09-04）

- 参数 Tab 按设计稿调整为 `30px` 高度；首个“参数”按钮固定 `62px` 宽度，其余 Tab 增加横向内边距并保持 `3px` 左起始偏移，使“参数 / 播放队列 / 快捷键 / 日志”的文字基线和间距与主图一致。
- 视频分类入口改为 `30px` 高度并只做视觉平移；视频结果卡新增独立的 `SnapshotMetricCardStyle`，分别收紧上方标签和下方结果值的垂直内边距，保持两行 `4×2` 卡片外框位置不变。
- 本轮只调整参数区排版、间距和样式复用，不改变系统生成快照、重新生成动作、规则编辑边界或真实运行时“已生效”语义。

## 144. 当前实施增量：视频结果卡迷你趋势图矢量化（2026-09-04）

- 视频结果卡底部的字符柱状占位改为 WPF 原生 `Path` 折线，保留亮度、对比度、饱和度、色相、锐度、伽马、曝光和降噪八项的独立趋势指示位置，使其与 v2 主图的细线型迷你图形一致。
- 折线只是快照卡的视觉指示，不读取或伪造真实音视频采样数据；参数值、随机生成边界、重新生成入口和用户不可直接改写结果的约束均未改变。
- 本轮未新增第三方依赖、未修改 Rust/Tauri `desktop/`、未删除既有代码；WPF 首屏/下滑截图、方案 Release 构建（0 警告/0 错误）、全量测试（458/458）、`dotnet format --verify-no-changes` 和 `tools/verify-scope.ps1` 均已通过。

## 145. 当前实施增量：视频结果卡列宽与内边距收口（2026-09-04）

- 两行视频结果卡按 v2 主图的列节奏由 `UniformGrid` 改为四列 `Grid`，基准窗口下右边界从约 `1119px` 收口到约 `1106px`；第一列、后三列和右侧留白分别由专用列定义控制。
- `SnapshotMetricCardStyle` 只增加结果卡左内边距，使图标和大号结果值与目标图的内基线对齐；迷你趋势图仍固定靠右，不改变结果值或卡片外框高度。
- 本轮只调整 WPF 参数卡布局样式，未改变快照生成、播放、输出卡状态或任何业务入口。

## 146. 当前实施增量：中央滚动容器边界校准（2026-09-04）

- 中央参数 `ScrollViewer` 在基准窗口下向右扩展 `8px`，内容 `StackPanel` 保留等量右侧内缩，避免改变音频、高级视觉、设备状态和日志卡片的有效内容宽度。
- 中央滚动条专用样式收窄为 `12px`，使实际 WPF 滚动条从约 `x=1130,width=17` 收口到设计稿约 `x=1139,width=12` 的视觉位置；滚动方向、偏移阈值和内容顺序保持不变。
- 该调整只作用于中央滚动容器，不改变右栏卡片、播放栏、参数值或真实运行态；临时布局诊断代码已移除。

## 147. 当前实施增量：抖音弹幕服务器字段铺满对齐（2026-09-04）

- 抖音弹幕卡的“弹幕服务器”只读地址行移除未使用的 `82px` 右侧空列，使输入框横向铺满到卡片右内边距，与 v2 主图的整行地址字段一致。
- “房间 ID”行仍保留独立 `82px` 断开连接按钮列；本轮不改变抖音 M1 的凭据、sidecar、扫码、队列或状态机边界。

## 148. 当前实施增量：视频结果卡列间留白像素收口（2026-09-04）

- 在保持四列起点和外框高度不变的前提下，按目标图对第一、第二、第三列分别增加 `1/3/3px` 右侧留白，第四列保持原右边界；基准截图中两行卡片边界已与目标图的 `395–559 / 567–740 / 750–924 / 934–1106px` 视觉区间一致。
- 本轮只微调视频快照卡的局部 Margin，未改变声音/高级视觉续页、滚动容器或右栏功能卡的布局和行为。
### 149. 右栏状态徽标固定占位宽度

- 右栏 RTMP、虚拟摄像头、麦克风、固定话术和抖音五张功能卡的状态徽标统一使用 `OutputStatePillStyle`，保留目标图约 `60px` 的标题行占位并将短状态文字居中。
- 该调整只固定徽标几何，不改变 `SetStatusPill` 的真实状态投影；待机、不可用、待连接等运行态仍按实际状态显示，不能用设计图中的已连接示例替代。
- 使用 WPF 现有 `Border` 样式，不增加第三方依赖、不新增业务状态、不删除未确认的历史代码。

## 150. 顶部操作带图标矢量化（2026-09-04）

- 顶部添加媒体、整理媒体、导入列表、保存配置、加载配置、开始推流和打开效果窗口改用 WPF 原生 `Path` 线性图标与文本组合，去除依赖字体字形的 `＋/▣/ↄ/□` 占位符。
- 图标笔画绑定按钮 `Foreground`，保留原按钮宽高、点击事件、自动化名称和操作顺序；不新增第三方图标依赖，也不改变任何业务行为。

## 151. 声音规则行比例与高度收口（2026-09-04）

- 下滑续页声音参数的“预设池/周期范围”规则行固定为 `70/164/91/164` 的列节奏，两个下拉框宽度收口到约 `156px`，与目标图的标签间距和输入长度一致。
- 两个规则下拉框高度调整为 `34px`，重新生成按钮调整为 `212×36px`，使声音快照卡的底部控制行与目标图保持同一垂直节奏，并继续保留禁用态和真实规则接入边界。

## 152. 视频指标图标几何化（2026-09-04）

- 首屏 8 个视频指标由字体符号改为 WPF 原生 `Geometry`/`Path` 图标；亮度/曝光使用太阳线稿，对比度/饱和度/锐度使用圆形线稿，降噪使用电平线稿，色相使用彩色环。
- 图标仅表达指标类别，不读取采样数据、不改变快照值、趋势线、绑定关系和只读边界；颜色环使用静态视觉资源，不代表实际色彩处理已生效。

## 153. 预览与播放控制图标统一（2026-09-04）

- 预览头部的适配窗口、画幅、静音和全屏，以及底部播放/暂停/停止/上一项/下一项改用 WPF 原生线性 `Path` 图标与文本组合。
- 保留原按钮尺寸、点击事件、自动化名称、提示和播放绑定；图标笔画跟随按钮前景色，不新增第三方依赖或改变播放行为。

## 154. 中央快照状态徽标节奏校准（2026-09-04）

- 视频首屏状态徽标由内容自适应改为约 `78px` 固定占位，声音/高级视觉续页收口为约 `70px`，并将文字居中；视频首屏额外保留目标图右侧内缩，使各页徽标的起点、间距和结果卡右边界分别对齐。
- 本轮只调整中央参数区标题行的视觉几何，不把“待接入”改成“已生效”，不改变快照生成、状态投影、滚动结构或真实媒体能力；仍需通过真实运行态和目标 DPI 截图完成最终 1:1 验收。

## 155. 顶部导航按钮高度统一（2026-09-04）

- 首页与工作区导航按钮由 `38px` 调整为 `36px`，与同一操作带的添加媒体、导入列表、配置和输出按钮共用垂直高度；基准截图中导航按钮垂直边界与目标图一致。
- 本轮只调整导航按钮几何，不改变页面切换、快捷键、授权态投影或操作带行高。

## 156. 底部状态带分栏补齐（2026-09-04）

- 底部状态带按目标图补齐左侧应用状态、中部丢帧/性能/上行信息分栏和右侧网络状态占位；左侧生产态继续复用 `StatusMessage`，中部性能文字复用现有 `PerformanceText` 的同一采样投影，避免复制性能事实源。
- 丢帧、上行和网络仍显示真实可用性边界（当前为占位或待检查），不静态伪造设计稿中的运行数字；本轮不改变播放、性能采样或网络请求逻辑。

## 157. 下滑声音卡外框与参数基线校准（2026-09-04）

- 声音续页外框顶部内距保持 `7px`，底部内距收口为 `8px`，外框与高级视觉区之间保留 `5px` 分段间距；第一排八项声音结果卡上间距为 `15px`，使声音卡底部和下一段起点贴合设计稿的续页节奏。
- 本轮只调整下滑页 WPF 排版，不改变声音快照值、规则禁用边界、滚动所有权或真实音频数据接入状态。

## 158. 麦克风设置入口图标化（2026-09-04）

- 右侧“麦克风插话”卡保留左侧测试按钮，将设置入口收口为右对齐的 34×30px WPF 线性齿轮图标，位置与设计稿一致。
- 保留原自动化名称、禁用态、提示和待接入边界；本轮只调整入口呈现，不改变麦克风门控逻辑。

## 159. 设置入口统一为矢量齿轮（2026-09-04）

- 顶部设置按钮与麦克风设置按钮共用 `SettingsGearIconGeometry`，统一使用 WPF `Path` 线性图标，避免字体符号在不同 Windows 字体/DPI 下产生形变。
- 保留按钮尺寸、点击事件、自动化名称和提示，不引入图标依赖或改变设置入口行为。

## 160. RTMP 状态行左右分栏（2026-09-04）

- 右侧 RTMP 卡底部改为左侧“停止推流/重新连接”操作、右侧 `188px` 状态文案的双区布局，状态文案右对齐并使用省略显示，贴合设计稿的操作与连接反馈节奏。
- `RtmpStatusText` 仍由原有校验、启动、停止和源切换逻辑写入；本轮只调整布局和提示，不伪造网络质量、推流成功或重连结果。

## 161. 开发与测试环境 HTTP 门禁（2026-09-04）

- `test` 与 `development` 环境均允许固定远程测试控制面 `http://101.96.208.132:9090` 和 loopback HTTP，未放行其他远程 HTTP 地址。
- 新增 `GpAutoLive - 本地开发控制面` 启动档，默认使用固定远程测试地址 `http://101.96.208.132:9090`；未设置开发/测试环境时，控制面仍要求 HTTPS，避免 Release 启动误连明文服务。
- 仅调整 WPF 启动编排的 HTTP 门禁和开发配置，不改变登录、激活、Refresh、Logout、心跳合同；远程真实登录仍待账号、设备授权和服务端错误矩阵联调。

## 162. C# 桌面端复用 Rust 产品图标（2026-09-04）

- 将 Rust/Tauri `desktop/src-tauri/icons/icon.png` 与 `icon.ico` 复制到 C# `src/GpAutoLive.App/Assets/`，C# 端不再通过项目外部路径链接 Rust UI 资源。
- PNG 继续用于自绘主标题栏品牌标识； ICO 同时作为 `GpAutoLive.exe` 的 `ApplicationIcon` 以及主窗口、设置窗口和最终效果窗口的 WPF `Icon`。
- 本轮只替换产品资源来源和窗口图标，不改变窗口布局、业务状态、登录门禁或 Rust `desktop/` 文件。

## 163. 最终 PCM 总线共享 RTMP 声音出口（2026-09-04）

- `WindowsAudioPlaybackController` 暴露当前会话的 `ActiveFinalPcmBus` 只读入口；`MainWindow` 启动 RTMP 声音/音画会话时优先把同一总线交给 `WindowsRtmpAudioSession`，RTMP 只消费总线的 RTMP 分支，不重复读取当前源或重复解码。
- `WindowsRtmpAudioSession` 增加非拥有总线模式：共享总线时只启动 RTMP 最终 PCM 分流泵，停止时等待泵和宿主回收但不关闭调用方所有的总线；无共享总线时保留原有独立 FFmpeg→FinalPcmBus 路径。
- 播放停止和音频自然完成前先停止 RTMP，避免本地音频控制器关闭总线后留下 RTMP 泵；本轮没有新增音频后端、无界队列或第三方依赖。
- 本轮自动化验证为锁定 SDK 下串行全量 `459/459`、`dotnet format --verify-no-changes` 和 Release 构建通过；N/N+1 真实预载、可听 PTS、设备故障恢复、ZLMediaKit/RTMPS 远端握手与长稳仍未验收。

## 164. WPF 视频处理开关进入 mpv 更新入口（2026-09-04）

- 底部“视频处理”开关新增 WPF 点击处理：只有当前播放池仍为 `Playing`、mpv 活动身份匹配且运行时处于播放/暂停时，才把 `VideoEffectEditorState` 的四项已校验值映射为 `Original` 或 `Gpu83` 快照并提交 `UpdateEffectsAsync`。
- 没有活动视频运行时或当前媒体为纯音频时，开关只保留待播放配置，不启动额外会话；提交失败显示错误，不把 UI 开关当成下一帧有效回显。
- 本轮没有新增依赖或命令通道；完整 GPU83 shader、下一帧有效回显、自动降级和真实媒体渲染门禁仍待验收。

## 165. 版本化 JSON 媒体列表导入（2026-09-05）

- 顶部“导入列表”接入现有 `RunImportAsync`，默认以 `ReplaceAll` 读取并替换当前媒体池；取消、列表格式错误、路径校验失败、运行包缺失或任一 FFprobe 失败时保留旧池，不自动播放。
- 列表使用既有 `VersionedJsonStore<T>`，当前格式为 `schema_version: 1` 与 `data.items[].path`。只允许本地绝对媒体路径和现有媒体扩展名白名单，拒绝未知字段、空项、相对/UNC/URL 路径、规范化重复路径以及超过 100 项的列表。
- 最小文件示例：`{"schema_version":1,"data":{"items":[{"path":"C:\\Media\\intro.mp4"}]}}`；文件只表达播放顺序，不能表达追加/替换操作，当前“导入列表”固定使用 `ReplaceAll`。
- 列表只保存源路径，不保存 `SourceMediaDto`、FFprobe 元数据、效果参数、播放位置、凭据或 RTMP 地址；导入阶段继续由 `MediaImportCoordinator` 重新探测并在全部成功后原子提交。
- 本轮新增 `MediaPlaylistContracts.cs`、`MediaPlaylistReader.cs` 及 Core/App 回归测试；不引入第三方依赖、不修改 Rust，不启用保存配置/加载完整配置，也不发明 PDF/DOCX 导入格式。

## 166. 媒体命令串行与输出生命周期收口（2026-09-05）

- `MainWindow.MediaPool.RunImportAsync` 在请求工厂完成后取得共享 `_playbackCommandSerial`，覆盖运行包校验、FFprobe、`PrepareMediaPoolCommitAsync` 和 `MediaImportCoordinator` 的原子提交；导入探测期间播放、停止、换源不会并发交错，取消/运行包失败/探测失败仍保持旧池并释放实际取得的闸门。
- `FinalEffectWindow_Closed` 不再只异步发送 mpv 关闭命令；用户主动关闭最终效果窗口时，复用 `StopMediaForMutationAsync` 统一停止 RTMP、虚拟摄像头、观察者、插话、PortAudio、mpv 和媒体池，并用实际 `_mediaPool.Snapshot` 更新主窗口。停止失败不显示“已释放”假成功。主窗口关闭时仍由既有 `OnClosed` 负责最终资源回收。
- `WindowsRtmpAudioSession` 以共享 `FinalPcmBus.Channels` 构造 `FfmpegPcmDecodePlan`、`AudioPcmMixingOutputSource` 和 `WindowsRtmpFinalPcmPump`；独立会话没有共享总线时继续使用 `FinalPcmBus.DefaultChannels`。这样单声道 PortAudio/RTMP 分流不会在启动前被固定双声道检查拒绝。
- 新增 `Import_waits_for_the_shared_playback_command_gate`、`Closing_final_effect_window_stops_media_pool_playback` 和 `Shared_mono_final_bus_is_used_by_the_rtmp_audio_chain` 回归测试。锁定 `.tools/dotnet` 下 Windows 全量 `224/224` 通过；App 新增关闭测试 `1/1`，`Main_window_imports_fixture_and_starts_video` 与 `Main_window_advances_media_pool_at_video_eof` 隔离均为 `1/1`，但 App 全量串行两次均为 `80/81`，真实 WPF/mpv 夹具在顺序运行下触发 90 秒宿主超时，故不宣称 App 全量通过。
- 不新增程序集、播放器、队列或第三方依赖，不修改 `desktop/` Rust/Tauri，不删除生产未使用代码；真实 ZLMediaKit/RTMPS、声卡稳定性、GPU 矩阵、AkVirtualCamera 下游、人工页面和 30 分钟长稳继续按总计划单独验收。

## 167. C# 单实例进程门禁修复（2026-09-05）

- 冷启动复现发现旧命名 Mutex 门禁仍会让第二个 `GpAutoLive.exe` 进入可见窗口；新增 `GpAutoLive.Windows/WindowsSingleInstanceLease.cs`，通过用户本地 `%LocalAppData%\GpAutoLive\locks\csharp-instance.lock` 的独占文件句柄建立进程边界。
- `GpAutoLive.App/App.xaml.cs` 在媒体/输出资源租约前取得单实例租约；已有实例存在时第二次启动直接退出，不创建第二个主控窗口。进程异常结束时 Windows 自动释放文件句柄，锁文件保留为空文件，避免删除竞态。
- 新增 `WindowsSingleInstanceLeaseTests`，验证同一路径二次获取返回 `AlreadyOwned`，释放后可再次获取；锁定 SDK 测试 `1/1`。新版 Release x64 构建 `0` 警告/`0` 错误，真实冷启动 A/B 验证为 A 保持运行、B 已退出、进程数保持 `1`。
- 不新增第三方依赖、不修改 Rust/Tauri、不删除生产代码。`FinalEffectWindow` 仍是同一 C# 进程中的独立 mpv/WGC 视频承载窗口，不是第二个桌面端。

## 168. 媒体池忙碌态后的按钮投影修复（2026-09-05）

- 复现了 WPF `ItemsSource` 尚未完成绑定时导入忙碌态结束的 UI 缺口：媒体池快照已有媒体，但 `ListBox.SelectedIndex` 暂为 `-1`，导致“下移”等操作按钮仍保持禁用。
- `SetMediaMutationButtonsEnabled` 在非空媒体池且控件尚未收到条目时，回退使用 `MediaPoolService` 的当前源索引；列表绑定完成后仍以实际控件选中项为准，不改变媒体池或播放状态。
- 强化 `Import_busy_disables_every_media_pool_mutation_button` 断言，锁定 `selected=-1/items=0/stateItems=2` 的失败场景；不新增依赖、不修改 Rust、声音、WGC 或单实例边界。

## 169. RTMP 声音首帧门禁与媒体池边界复验（2026-09-05）

- `WindowsRtmpAudioSession.StartAsync` 启动分流泵后，在 3 秒有界预算内确认至少一帧最终 PCM 已转发至 FFmpeg；无 PCM、泵失败或取消均返回失败并走有界停止，禁止把“任务已启动”投影为“声音已生效”。
- `SetMediaMutationButtonsEnabled` 对媒体池当前源索引增加池长度边界检查；WPF 列表绑定暂未完成时仅用于按钮投影回退，绑定后仍由实际 `ListBox.SelectedIndex` 驱动。
- 受影响测试 `28/28`、Windows 全量 `226/226`、`dotnet format --verify-no-changes --no-restore` 和 Release x64 构建 `0` 警告/`0` 错误通过；不新增程序集、播放器、队列或第三方依赖，不修改 Rust/Tauri。真实声卡、远端 ZLMediaKit/RTMPS、人工页面、AkVirtualCamera 下游和长稳继续单独验收。

## 170. C# 音频滤镜顺序与字段消费边界复核（2026-09-05）

- `FfmpegAudioFilterBuilder` 现在按 Rust 音频链的顺序先执行音高微移（`asetrate → aresample → atempo(1/ratio)`），再执行播放速度 `atempo`；这保证两项同时启用时输出时长和处理顺序一致。
- 高频扰动的开关、间隔、强度和目标电平仍由 `AudioEffectParams` 进入同一个受管 `-af`，并沿用 Rust 的 6kHz 以上门控、周期表达式和 dB 电平换算；本轮只修正组合顺序，没有新增 DSP、播放器、线程或队列。
- `GeneratedAudioEffectSnapshot` 的 `DynamicRangeDb`、`Compression`、`Tone` 仍没有正式 C# 参数/算法对应，继续只读展示并明确未接入；不得用 `acompressor`、EQ 或其他近似滤镜冒充这些字段已生效。
- 失败先行测试 `Pitch_shift_precedes_playback_speed_like_the_rust_realtime_chain` 先在旧顺序下失败，修复后通过；Media 音频测试与格式检查按本轮实际结果记录，Rust 端保持只读。

## 171. C# 播放状态契约与播放意图对齐（2026-09-05）

- `MediaPoolOwner.ResumePlayback` 与 Rust `PlaybackCore::resume` 对齐，只允许 `Ready/Paused → Playing`；`Stopped` 保留为停止终态，重新播放必须调用 `StartPlayback`，并且失败的恢复操作保持原快照不变。
- `MainWindow` 只有在 Core 状态为 `Playing/Paused`、当前媒体身份一致且 mpv 运行态有效时才复用 `TogglePauseAsync`；`Ready/Stopped` 不再根据进程存活猜测恢复，而是沿用登录、运行包、FFprobe、首帧和单一 mpv 会话门禁进入启动路径。
- 本轮不新增播放器、抽象或依赖，不修改 Rust；Core 状态测试 `16/16` 通过。App/WPF 人工导入、暂停/恢复/停止和 EOF 换源、真实声卡及长稳仍需独立验收。
