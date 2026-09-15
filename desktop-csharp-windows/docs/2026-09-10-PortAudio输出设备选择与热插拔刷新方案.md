# PortAudio 输出设备选择与热插拔刷新方案

状态：代码已接入，定向自动化和本地 Release 构建通过；真实热插拔与指定设备出声待实机验收。2026-09-10。

## 1. 目标与边界

让 C# 主界面能够看见、刷新并选择当前连接的 PortAudio 输出设备，真实声音从所选设备播放。以手动刷新、停止后选择、下次播放生效完成本次功能；播放中无缝切换不是本次验收目标。

用户在方案完成后授权实施，并明确输出控件放在整个“插话与互动”区域正上方。沿用现有 PortAudio DLL、播放控制器、输入流和原生资源所有者，不新增依赖，不修改 Rust、Go、RTMP 或虚拟摄像头链路。

## 2. 已确认的现状

| 位置 | 当前行为 | 问题 |
| --- | --- | --- |
| `src/GpAutoLive.App/MainWindow.xaml`，1334、1500 附近 | 输出设备和刷新按钮位于默认折叠的“插话文件池（高级）” | 输出是全局能力，却藏在插话区域 |
| `MainWindow.AudioInterlude.cs`，`RefreshAudioDevicesAsync` | 筛选输出声道大于零的设备；刷新后总是选默认设备或第一项 | 用户原选择被覆盖；状态文案固定说尚未启动音频流 |
| `MainWindow.Playback.cs`，`TryPrepareAudioPlaybackAsync` | 准备播放时读取 ComboBox 的设备索引 | 改下拉不会改变已打开的输出流，界面没有说清生效时机 |
| `WindowsPortAudioDeviceEnumerator.cs` | 每次探测调用 Initialize / 枚举 / Terminate | 其他输入或输出流仍持有初始化引用时，不能据此保证热插拔列表已更新 |
| `WindowsPortAudioOutputRecovery.cs` | 有界重开同一设备配置 | 这是已有播放故障恢复，不是设备刷新或重新选择 |

Rust 对照：`desktop/ui/src/desktop/portaudio-device-panel.tsx` 打开下拉时刷新，支持系统默认、Host API 筛选和“应用设置”；`App.tsx` 调用 `list_audio_output_devices`，成功替换列表，失败保留旧列表。`desktop/crates/autolive-portaudio-output/src/lib.rs` 的 `pa_acquire/pa_release` 采用引用计数，播放中不会因为枚举而重新初始化。输出 ID 只是设备索引字符串，不能视为跨刷新稳定身份。`desktop/src-tauri/src/commands.rs` 的应用设置会停止旧输出并按媒体时钟重建，复杂度高于本次需要。

PortAudio 的初始化和终止必须配对；最后一个初始化引用释放后才完成终止，见[官方 API](https://portaudio.com/docs/v19-doxydocs/portaudio_8h.html)。官方 [HotPlug 说明](https://github.com/PortAudio/portaudio/wiki/HotPlug)讨论的刷新 API 属于提议分支，不能假定当前 DLL 提供它。因此本方案采用所有本进程流释放后重新枚举，实机插拔作为最终证据。

## 3. 最小交互

在整个“插话与互动”区域正上方增加独立常显的一行：`输出设备 [设备名称 · Host API ▼] [刷新设备]`，下方显示当前状态。移动现有控件，不复制第二套入口；麦克风选择仍在麦克风区域。

- 空闲时允许刷新和选择；选择直接作为下一次播放配置，不增加“应用”按钮。
- 播放、暂停仍持有音频资源或麦克风监听期间，禁用刷新与设备选择，明确显示“停止播放和麦克风后可刷新；所选设备在下次播放生效”。暂停不算资源释放。
- 用户停止相关功能后，点击刷新，读取本次实际枚举结果。无需重启应用。
- 初次枚举选本次系统默认设备；默认设备缺失则要求手选，不任意选第一项冒充系统默认。
- 刷新前记住所选设备的名称和 Host API；刷新后只有唯一匹配才保留选择，并使用新索引。有重名歧义或原设备消失时清空选择，提示手选，不猜测、不静默换设备。这只是会话内匹配，不声称稳定硬件 ID，不存磁盘。
- 列表为空显示“未发现输出设备”，禁止启动依赖输出设备的声音播放。刷新失败显示真实错误，旧列表可以保留作展示，但不能把它标成最新或继续用旧索引启动。

本轮“系统默认”只在初次枚举解析为具体设备，不新增永久跟随系统默认的模式。Host API 随名称显示以区分重复端点，不增加筛选面板或测试音功能。

## 4. 调用链与资源约束

`刷新按钮 → 现有播放/麦克风生命周期检查与串行入口 → 确认旧流及其原生引用已释放 → ProbeAsync → 更新设备列表与选择 → 点击播放 → TryPrepareAudioPlaybackAsync → 现有输出流打开`

刷新、开始播放和开始麦克风必须通过现有生命周期入口互斥，不能只依赖按钮禁用。实现前核对现有锁的获取顺序，复用既有同步方式，不建立新的设备管理器、队列或全局锁。枚举器的实例锁本身不能替代跨播放/输入的互斥。

先核实停止后的实际释放路径：播放控制器、麦克风输入和枚举器的每次成功初始化都要有对应终止；不能额外多调用 Terminate 强行清引用。若停止只暂停流，则在原所有者的停止路径完成关闭与释放，不扩大到重写音频引擎。

输入和输出来自同一 PortAudio 索引表。重新初始化后同步更新空闲麦克风列表并重新定位其选择，防止下一次监听沿用旧索引。保留已有“麦克风运行时禁止刷新”边界。

点击播放前要求选择属于最近一次成功枚举且具备输出声道；设备可能随后被拔出，原生打开失败直接显示现有错误，由用户刷新重选。已有播放中故障恢复按原规则保留，不扩展为自动切默认设备。

取消和窗口关闭沿用窗口 CancellationToken、原生操作所有者和 Join 路径，等待正在执行的探测完成资源清理，不创建超时后遗留后台任务。不为本次增加重试预算或超时配置。

## 5. 文件职责与实施顺序

1. `MainWindow.xaml`：移动现有输出控件到常显声音区域，补充生效时机与状态提示。
2. `MainWindow.AudioDevices.cs`（已新增）：收拢现有设备刷新、选择匹配和设备状态投影；由主窗口事件与播放准备调用。只移动直接相关逻辑，不让播放或 Windows 库反向依赖 UI。
3. `MainWindow.AudioInterlude.cs`：移出相应输出/枚举代码，保留麦克风交互并调用统一设备状态投影，删除移动后重复的处理函数。
4. `MainWindow.Playback.cs`：接入有效选择与操作互斥，明确停止后可刷新；不加实时重建链。
5. `WindowsPortAudioDeviceEnumerator.cs`、输入/输出控制器：仅当释放核查发现实际缺口时修改，保持现有公共边界。
6. 对应 Windows/App 测试、GUI 验证记录及本方案：同步真实状态与结果。

UI 调整与设备逻辑由两个独立子线程完成，主线程完成必要集成和验证。用户已选择“只完成实施和必要验证”，本轮未开展额外代码总监专项审核或扩展未使用代码清理。移动后的旧函数不重复保留，未删除无关功能。测试先落盘，但首次执行因系统 dotnet 无 SDK 被阻断；收到项目 SDK 路径后实现已完成，未取得有效的实现前失败基线，不宣称完成了红绿测试流程。

## 6. 验收与最小验证

- 首屏可见输出设备，不展开插话文件池也能操作；窄窗口仍能看清名称并点击刷新。
- 应用保持运行：停止音频和麦克风，插入 USB/蓝牙输出，刷新后出现；拔出再刷新后消失。蓝牙以 Windows 已完成连接并暴露音频端点为前提。
- 选择外接设备，播放源音、普通声音效果和插话，人工确认声音确实从该设备发出；暂停、停止与再次播放行为一致。
- 保留设备遇索引变化仍选回唯一匹配项；同名歧义、拔出与无设备都不误选其他设备。
- 播放或麦克风运行时不能刷新；停止后成功刷新；刷新期间开始播放或监听不会交叉初始化。
- 枚举失败、设备打开失败、刷新时关窗都有可见终态和资源释放，不能显示假成功。
- 刷新之后启动麦克风，使用重新定位后的输入设备；验证输入索引不会串到其他设备。

自动化仅运行新增选择/互斥测试及直接受影响的 PortAudio 枚举、所有权、输出与输入测试；如修改底层共享生命周期，再按影响面扩大。执行受影响 C# 项目的本地构建，不跑 Rust/Go 全量测试。具体命令和过滤器在实施记录中按实际新增测试填写。真实插拔和出声必须单独记录，不能用源码字符串断言替代。

方案阶段只读核对了 C#/Rust 调用链及官方初始化契约；实施阶段的实际检查见第 8 节。

## 7. 消融结论

方案已排除独立设备管理框架、后台轮询、设备通知服务、持久化索引、Host API 筛选面板、测试音、无缝切换、额外应用按钮和新的自动恢复链。复用已有枚举、流所有者与停止/播放入口。

保留设备能力校验、唯一匹配、串行生命周期、初始化配对与取消清理；这些直接对应误选、设备插拔、原生资源竞争和退出验收。未删除现有安全或故障恢复边界。

最终方案覆盖“看见输出设备 → 停止后刷新热插拔列表 → 选择 → 播放到指定设备”。限制是刷新和切换需先停止相关音频活动；播放中无中断切换与自动跟随系统默认不在本轮范围。真实设备兼容性留待实施后的本机验收。

## 8. 实施与验证结果

实际产品文件为 `MainWindow.xaml`、`MainWindow.AudioDevices.cs`、`MainWindow.AudioInterlude.cs`、`MainWindow.Playback.cs`；新增测试 `tests/GpAutoLive.App.Tests/AudioDeviceSelectionTests.cs`。Windows 底层、Rust、Go 与依赖清单未改。本方案和 `GUI多分辨率调整与验证.md` 同步产品位置与操作规则。

刷新按钮调用播放串行入口后进入枚举核心，首次播放准备直接调用核心，避免重复获取同一闸门。活动或暂停状态拒绝刷新；空闲或失败态先由既有输入/输出所有者确认 StopAsync 成功，再重新初始化和枚举。第一次成功枚举才记录已枚举状态，失败重试仍可选系统默认；成功枚举后原选择消失则要求手选。失败保留旧列表供展示，但失效标记禁止用其启动输出和麦克风。

| 检查类别 | 实际结果 |
| --- | --- |
| 代码与格式 | 两个新增 C# 文件的 `dotnet format whitespace --verify-no-changes --no-restore --include …` 通过；相关已跟踪文件 `git diff --check` 无错误，仅 LF/CRLF 提示 |
| App 自动化 | `AudioDeviceSelectionTests` 4 项、`MicrophoneUiTests` 2 项，共 6/6 通过；含独立 WPF 实际串行等待与取消检查，不触发原生探测 |
| Windows 自动化 | PortAudio 枚举、输入、输出、所有权及麦克风控制器共 42/42 通过，0 跳过；结果在 `artifacts/portaudio-native-validation/results/portaudio-native.trx` |
| 本地构建 | App Release x64 成功，0 警告、0 错误；输出到隔离目录，未覆盖运行中的开发端 |
| 局部布局 | 真实 XAML 与项目样式在 300/340/480 DIP 下 Measure/Arrange 均通过；30 DIP 下拉与按钮无重叠、控件唯一且不在 Expander 中，位置紧接“插话与互动”标题之前；已目视核对 PNG |
| 页面点击与真实业务 | 未重启或操作正在运行的客户端；没有执行真实 USB/蓝牙插拔、完整播放出声或监听验收，不能据自动化结果宣称已完成硬件验收 |
| 容器健康、外部模型 | 不涉及；未运行 |
| 全量测试与完整发布包 | 未运行；本次只改 C# 设备 UI 编排，不涉及底层共享实现、依赖或发布，不扩大到 Rust/Go、全部 E2E 或安装包构建 |

实际命令在 `desktop-csharp-windows` 目录执行（SDK 固定为 `.tools/dotnet/dotnet.exe`）：

```powershell
# 两个测试项目分别锁定还原到隔离目录。
.tools/dotnet/dotnet.exe restore tests/GpAutoLive.App.Tests/GpAutoLive.App.Tests.csproj --locked-mode -p:Platform=x64 --artifacts-path artifacts/portaudio-device-validation --verbosity quiet
.tools/dotnet/dotnet.exe restore tests/GpAutoLive.Windows.Tests/GpAutoLive.Windows.Tests.csproj --locked-mode -p:Platform=x64 --artifacts-path artifacts/portaudio-native-validation --verbosity quiet
.tools/dotnet/dotnet.exe test tests/GpAutoLive.App.Tests/GpAutoLive.App.Tests.csproj --no-restore --artifacts-path artifacts/portaudio-device-validation --filter 'FullyQualifiedName~AudioDeviceSelectionTests|FullyQualifiedName~MicrophoneUiTests' -p:Platform=x64 -v:q
.tools/dotnet/dotnet.exe test tests/GpAutoLive.Windows.Tests/GpAutoLive.Windows.Tests.csproj --no-restore -c Release -p:Platform=x64 --artifacts-path artifacts/portaudio-native-validation --filter 'FullyQualifiedName~WindowsPortAudioDeviceEnumeratorTests|FullyQualifiedName~WindowsPortAudioInputStreamTests|FullyQualifiedName~WindowsPortAudioOutputStreamTests|FullyQualifiedName~WindowsPortAudioOwnershipTests|FullyQualifiedName~WindowsMicrophoneInterludeControllerTests' --logger 'trx;LogFileName=portaudio-native.trx' --results-directory artifacts/portaudio-native-validation/results --verbosity quiet
.tools/dotnet/dotnet.exe build src/GpAutoLive.App/GpAutoLive.App.csproj -c Release -p:Platform=x64 --no-restore --artifacts-path artifacts/portaudio-device-validation --verbosity quiet
.tools/dotnet/dotnet.exe format whitespace src/GpAutoLive.App/GpAutoLive.App.csproj --verify-no-changes --no-restore --include src/GpAutoLive.App/MainWindow.AudioDevices.cs --verbosity quiet
.tools/dotnet/dotnet.exe format whitespace tests/GpAutoLive.App.Tests/GpAutoLive.App.Tests.csproj --verify-no-changes --no-restore --include tests/GpAutoLive.App.Tests/AudioDeviceSelectionTests.cs --verbosity quiet
powershell.exe -NoProfile -STA -File artifacts/vcam-validation/check-audio-devices-layout.ps1
```

布局脚本首次因 Windows PowerShell 的 UTF-8 无 BOM 解析、缺少测试宿主 CardStyle 资源失败，修正测试脚本后重新执行成功；这两项为验证宿主问题，未修改产品来规避验证。当前运行窗口仍是修改前版本，下次通过开发启动入口重建启动后加载此 UI。必要后续为本机停止 → 插拔 → 刷新 → 选择 → 播放及麦克风输入重定位验收。
