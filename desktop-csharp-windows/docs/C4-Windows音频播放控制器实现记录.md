# C4 Windows 音频播放控制器实现记录

日期：2026-09-02  
状态：代码已接入·WPF 纯音频播放/暂停/停止和设备健康恢复已接线，待真实丢设备和音画时钟验收

## 已实现

- `WindowsAudioPlaybackController` 统一拥有一次本机音频会话：FFmpeg PCM 解码 → 固定容量环缓 → PortAudio 输出流。
- 启动顺序固定为输出流先启动、再启动解码；解码结束后关闭环缓并停止输出，取消/关闭使用 Job Object、进程终止和 2 秒 Join 预算。
- 循环模式使用受环缓水位限制的重新解码，不创建无界候选队列；快照只返回状态、计数和脱敏错误。
- 暂停使用独立 `WindowsAudioPauseGate`：先暂停 PortAudio 原生流，再阻止 FFmpeg 继续读取，保留环缓与解码位置；恢复按相反顺序打开输出并释放门控。暂停/恢复失败会取消并有界回收整条会话，避免“状态已暂停但进程仍泄漏”。有限音轨结束后增加最多 5 秒环缓排空等待，避免尾部 PCM 被立即关闭截断。
- 该控制器不依赖 WPF、登录或网络，可由上层在已完成媒体资源和设备校验后调用。
- `MainWindow` 纯音频路径已接线：播放前必须通过已验证媒体清单、FFmpeg/PortAudio 固定资源和用户选择的输出设备；播放、暂停、恢复和停止均先完成真实设备操作，再提交播放池状态。有限单项会话结束后由唯一完成观察者校验四维身份并按播放池顺序启动下一项，切换媒体项时不复用旧解码会话，失败保持 fail-closed。
- `WindowsAudioPlaybackController` 可选接管 `FinalPcmBus`：FFmpeg 解码只向最终总线发布一次，PortAudio 消费 `OutputBuffer`，RTMP 侧保留 `RtmpBuffer`，避免为每个输出重复读取源媒体。WPF 纯音频已传入该总线；RTMP 声音/音画由独立 `WindowsRtmpAudioSession` 启动自己的总线和分流泵。
- 会话同时运行一个 250 ms 健康观察器：暂停门控期间不触发恢复；非暂停状态检测到 `Stopped`、`Inactive` 或 `QueryError` 时调用 `WindowsPortAudioOutputRecovery`，持续 xrun 达到 1024 callback 且至少 75% 异常时也复用同一入口重建输出流。xrun 恢复每轮成功后重新建立计数基线，连续 3 次仍异常则取消解码、停止输出并报告 `audio_output_overrun`；设备恢复仍最多按 3 次有界退避重开同一设备配置，失败报告 `audio_device_lost`，不会让 UI 保持“播放中”假状态。
- 麦克风输入由 `WindowsMicrophoneInterludeController` 以 50 ms 异步观察 `Pa_IsStreamActive`/`Pa_IsStreamStopped` 投影；缺少可选原生导出时保守返回 `Unknown` 并继续工作，发现 `Stopped`、`Inactive` 或 `QueryError` 则清除优先级、取消会话并在线程池执行有界停止。输入不自动重开，避免设备拔出期间重复持有采集句柄。

## 明确未实现

- 控制器本身仍是单源有限解码/循环入口，不拥有媒体池 EOF、N/N+1 候选、A/B 交叉淡化或播放池索引；`MainWindow` 通过完成观察者在控制器会话之间完成有限单项的顺序推进。
- 仍未实现 A/B 交叉淡化、PTS/FPS 纠偏和 SAPI/插话对 PCM 总线的实际静音恢复；N/N+1 真实候选已由 WPF 编排接入。xrun/设备恢复代码已接入，但真实声卡拔插、睡眠唤醒、重新枚举和长稳继续待目标 Windows 设备验收。
- RTMP 声音输出已具备独立最终 PCM 会话编排（含视频源音频和有界分流）；编码器探测、断线重试、ZLMediaKit/RTMPS 网络门禁仍未完成，不能将本机总线验证误报为网络推流可用。

## 验证

控制器的空计划、取消、关闭、停止幂等、暂停门控、输出流暂停/恢复和健康恢复边界测试已加入；显式设置 `AUTOLIVE_TEST_FFMPEG` 与 `AUTOLIVE_TEST_PORTAUDIO_DLL` 后，本机 FFmpeg 正弦 WAV fixture 已实际走完解码、PortAudio 输出、健康观察和会话清理。真实声卡拔插/睡眠唤醒、设备重新枚举与长稳仍需目标 Windows 设备完成。
