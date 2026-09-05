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

结果：App Release x64 独立构建 **0 警告/0 错误**；App 测试 **36/36 通过**；`dotnet format --verify-no-changes` 和 `git diff --check` 通过。当前 `GpAutoLive` PID 19612 未停止。
