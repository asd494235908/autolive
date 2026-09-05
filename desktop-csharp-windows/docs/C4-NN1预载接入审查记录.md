# C4 N/N+1 音频预载接入审查记录

日期：2026-09-03  
状态：真实候选预载未接入；受限调度与单输出源切换边界已核验

## 审查结论

当前 WPF 普通声音链路为：

```text
WindowsAudioPlaybackController
  └─ 一个 WindowsFfmpegPcmDecoder
       └─ 一个固定容量 PCM 目标（FinalPcmBus 或 AudioPcmRingBuffer）
            └─ 一个 WindowsPortAudioOutputStream
```

`MainWindow` 在音频项结束后由唯一完成观察者验证四维播放身份，再停止当前会话并启动下一项。当前实现没有以下可用于无缝提交的能力：

- 输出源在两个候选环缓之间的原子切换；
- `FinalPcmBus` 的本机/RTMP 双消费者候选切换；
- 带起始 PTS/偏移的 FFmpeg 继续解码；
- 由 PortAudio 可听时钟驱动的候选提交时刻。

因此，直接在当前播放期间启动第二个 FFmpeg 解码器会产生两个问题：

1. 第二个候选从媒体开头实时解码时，尚未消费的固定环缓会覆盖旧帧，无法保证提交点从正确 PTS 开始。
2. 将第二个解码器或另一个输出流接入 WPF 会绕过当前单一音频所有者和 `FinalPcmBus` 生命周期，可能产生重复声音、断帧、双重关闭或 RTMP/本机不同步。

本轮不接入伪造的 N/N+1 可听播放，也不把“预先创建 `FfmpegPcmDecodePlan`”标记为 PCM 预载。真实接入前必须先定义候选输出源、可听 PTS 和本机/RTMP 双消费者的统一提交协议，再通过 Windows 设备门禁验证。

## 已加强的可验证边界

- `AudioCyclePrewarmCoordinator` 仍只保存不透明候选引用并产生 `Prepare/Commit/Expire`，不创建解码任务、不拥有 PCM 队列、不创建 N+2。
- 新增 `Planned_evaluation_is_side_effect_free_and_keeps_opaque_sample`，证明重复评估不会改变候选状态或物化 PCM 工作。
- 新增 `AudioPcmTrackSwitchOutputSource`：在一个 `IAudioPcmOutputSource` 内最多保留当前 N 和一个已准备的 N+1；只有显式提交且 N 已关闭、排空到帧边界后才切换，同一读取批次可连续交付 N 尾帧与 N+1 首帧。
- `AudioPcmTrackSwitchOutputSource` 不把临时欠载当作 EOF，拒绝第二个待准备候选，并通过 `AudioPcmRingBuffer` 夹具验证提交前不切换、关闭后切换和候选上限。
- `WindowsAudioPlaybackController` 的显式 FFmpeg + PortAudio fixture 新增第二次启动尝试，必须返回 `already_running`；它验证当前音频所有权不会在单会话期间隐式扩张为第二个输出会话。
- 所有真实音频 fixture 仍要求显式设置 `AUTOLIVE_TEST_FFMPEG` 与 `AUTOLIVE_TEST_PORTAUDIO_DLL`，没有真实资源时不伪造通过。

## 下一步接入前置条件

1. 将当前 Media 层固定容量候选输出源分别接入本机 PortAudio 与 RTMP 的既有单输出/单泵边界；不得增加第二个独立输出流。
2. 为候选定义有限的起始 PTS/解码偏移和取消 Join 预算；没有偏移语义时只允许保留当前纯逻辑调度。
3. 在真实声卡输出、暂停/恢复、设备拔插和 RTMP 音频时钟验证后，才将 C4 状态提升为“代码已接入·待验收”。

## 验证命令

```powershell
& .tools/dotnet/dotnet.exe test tests/GpAutoLive.Media.Tests/GpAutoLive.Media.Tests.csproj -c Release --no-restore --logger "console;verbosity=minimal"
& .tools/dotnet/dotnet.exe test tests/GpAutoLive.Windows.Tests/GpAutoLive.Windows.Tests.csproj -c Release --no-restore --logger "console;verbosity=minimal"
& .tools/dotnet/dotnet.exe format tests/GpAutoLive.Media.Tests/GpAutoLive.Media.Tests.csproj --no-restore --verify-no-changes
```

本轮实际结果：Media 测试 `96/96` 通过，使用独立 `-p:OutputPath=artifacts/c4-media-verify-20260903`；Windows 测试 `194/194` 通过；Media 项目 `dotnet format --verify-no-changes` 与 `tools/verify-scope.ps1` 通过。真实 PortAudio fixture 仅在显式环境变量和可用设备存在时执行；否则按项目现有约定跳过硬件路径，本轮未将其作为已验收证据。

## 本轮未验收

- `AudioPcmTrackSwitchOutputSource` 尚未接入 `WindowsAudioPlaybackController`、`FinalPcmBus` 或 RTMP PCM pump，因此未宣称真实候选预载、PortAudio 可听切换、RTMP 同步或音画纠偏已完成。
- FFmpeg 继续解码的起始 PTS/偏移、候选取消与 Join、真实声卡、设备拔插/睡眠唤醒和网络长稳仍需后续专项门禁。
