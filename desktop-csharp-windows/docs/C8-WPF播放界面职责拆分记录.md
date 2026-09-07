# C8 WPF 播放界面职责拆分记录

本次仅调整 `MainWindow` partial 文件职责，不改变播放行为、共享字段所有权、XAML 事件名、状态语义或 Windows 资源生命周期。Rust/Tauri/React 参考目录保持只读。

## 拆分结果

- 新增 `src/GpAutoLive.App/MainWindow.Playback.cs`，继续使用同一个 `MainWindow` partial 类型和原有私有字段。
- 移入播放池上一项/下一项导航、播放/暂停/停止按钮、键盘快捷键，以及视频进度条鼠标/键盘输入。
- 同时移入通用播放命令串行闸门、mpv 状态观察、PortAudio 完成观察、视频/纯音频自然结束后的下一项推进、音频会话启动和视频进度投影/绝对 seek。
- `MainWindow.xaml.cs` 保留窗口构造/关闭生命周期、共享字段、媒体池编辑、登录/设置/最终效果窗口和跨功能状态编排；不复制播放状态、取消源或控制器。

## 保持不变的边界

- XAML 仍绑定原有 `PreviousButton_Click`、`NextButton_Click`、`PlayPauseButton_Click`、`StopButton_Click`、`Window_PreviewKeyDown`、`PlaybackSlider_PreviewMouseLeftButtonUp` 和 `PlaybackSlider_KeyUp`。
- 播放命令继续通过同一个 `_playbackCommandSerial` 串行执行；mpv、音频控制器和媒体池仍由原有四维 `MediaPlaybackIdentity` 门禁保护。
- 视频 `time-pos` 仍只投影到当前匹配身份；seek 仍调用 `WindowsMpvPlaybackController.SeekAsync`，不在 UI 拼接 IPC 或命令行。
- 纯音频的 FFmpeg→PortAudio 会话、插话混音、RTMP/虚拟摄像头/抖音和窗口生命周期仍保持各自 partial/主窗口边界，不引入第二份控制器或配置状态。

## 最终效果弹窗呈现契约（2026-09-05）

本弹窗对齐 Rust/Tauri `final-effect` 页面：它是同一 C# 进程内的唯一最终效果窗口，不是第二个桌面端实例，也不创建第二个视频渲染表面。客户区只保留黑色画布和媒体表面：

- 视频使用 `ReservedVideoSurface`，纯音频使用黑色表面，空池保持纯黑；不显示等待文字、表面状态浮层或任何底部信息栏。
- 删除 Footer 的播放/暂停、停止、进度、会话/源/循环信息和关闭按钮；播放控制仍由主窗口负责，窗口关闭使用原生标题栏关闭按钮。
- 标题为 `GpAutoLive 最终效果`，默认客户区为 `1280×720`，最小尺寸为 `320×180`，可调整大小并居中，和 Rust 运行时创建窗口一致。
- 全屏只切换同一窗口的 `WindowStyle`、`ResizeMode` 和 `WindowState`；`Loaded`、F11 往返和顶层 HWND 查询仍刷新视频预留表面布局。
- `ReservedVideoSurface` 继续承载 mpv 子 HWND；Windows Graphics Capture 继续绑定最终效果窗口的顶层 HWND。两者不互换、不创建覆盖层窗口。

### 生命周期审计结论

本轮移除了只服务于 Footer 的按钮事件、播放命令事件和进度/身份投影字段，`FinalEffectWindowController` 仅保留窗口开关与表面类型投影。WPF airspace 下的原生视频宿主仍由独立 `ReservedVideoSurface` 承载，C# 不能因删除可见控件而删除 HWND 宿主或顶层窗口绑定。

自动化覆盖：WPF 回归测试验证 Rust 对齐的标题/尺寸、视频纯画布无 Footer/覆盖文字、F11 往返和子 HWND/顶层 HWND 合同；既有 WGC 测试继续负责真实顶层 HWND 绑定门禁。

## 方法清单

`MainWindow.Playback.cs` 当前承载：

- `PreviousButton_Click`、`NextButton_Click`、`NavigateMediaAsync`、`NavigateMediaCoreAsync`
- `PlayPauseButton_Click`、`StopButton_Click`、`Window_PreviewKeyDown`
- `TogglePlaybackAsync`、`TogglePlaybackCoreAsync`、`StopPlaybackAsync`、`StopPlaybackCoreAsync`
- `StartAudioPlaybackAsync`、音频完成观察、视频状态观察和自然结束推进
- `RunPlaybackCommandAsync`
- `UpdateMediaProjection`、`CreateFinalEffectSnapshot`、`ProjectVideoPlaybackPosition`
- `PlaybackSlider_PreviewMouseLeftButtonUp`、`PlaybackSlider_KeyUp`、`SeekVideoFromSliderAsync`

## 验证

使用独立输出目录执行：

```powershell
& .tools/dotnet/dotnet.exe build src/GpAutoLive.App/GpAutoLive.App.csproj `
  -c Release --no-restore -p:Platform=x64 `
  -p:OutputPath=artifacts/app-playback-build

& .tools/dotnet/dotnet.exe test tests/GpAutoLive.App.Tests/GpAutoLive.App.Tests.csproj `
  -c Release --no-restore -p:Platform=x64 `
  -p:OutputPath=artifacts/app-tests-playback-build `
  --logger "console;verbosity=minimal"

& .tools/dotnet/dotnet.exe format GpAutoLive.Windows.slnx `
  --no-restore --verify-no-changes --verbosity minimal

git diff --check
```

本轮样式收口后，FinalEffectWindowController 目标测试 **3/3 通过**；视频纯画布/标题尺寸与 F11 往返测试分别 **1/1、1/1 通过**；关窗后句柄拒绝测试 **1/1**。独立 WGC 启停测试在当前 WPF 测试宿主中未取得最终效果窗顶层 HWND（`0x0`），因而未计为通过；该测试仍使用无 Owner 的非生产构造路径，生产路径的 `Owner=MainWindow` 和真实页面需单独验收。App Release x64 构建 **0 警告/0 错误**，`dotnet format --verify-no-changes` 和 `git diff --check` 通过。自动化环境未进行人工页面点击，不能以这些结果替代真实视频播放、声卡、WGC 下游和多屏/DPI 验收。
