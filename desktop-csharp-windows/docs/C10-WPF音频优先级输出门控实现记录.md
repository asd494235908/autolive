# C10 WPF 音频优先级输出门控实现记录

## 范围

本增量把共享 `AudioPriorityCoordinator` 的基础媒体策略真正应用到 FFmpeg PCM 解码输出：在写入 PortAudio/RTMP 固定容量总线前，按当前优先级原地完成静音或 duck。它不创建第二份音频源、不改变 Rust/Tauri/React 参考目录，也不把麦克风 PCM 伪装成已完成的可听插话混音。

- `FixedSpeech` 或 `Microphone` 优先级：基础媒体静音。
- `InterludeFile` 优先级：基础媒体固定降低 6 dB；当前插话文件源仍待接入，因此不会凭空生成 overlay。
- 默认优先级：基础媒体保持 0 dB。
- 每个解码分片使用既有池化 `float[]` 原地处理，随后写入本机输出和 RTMP 两个有界消费者；PortAudio 原生回调继续不分配、不锁业务状态。

## 文件职责

- `src/GpAutoLive.Media/AudioPcmMixer.cs`：新增 `TryApplyBasePolicy` 原地静音/duck/增益边界。
- `src/GpAutoLive.Windows/WindowsFfmpegPcmDecoder.cs`：接受可选策略提供器，在分片写入前应用基础媒体策略。
- `src/GpAutoLive.Windows/WindowsAudioPlaybackController.cs`：保存单一会话策略提供器并传入解码器。
- `src/GpAutoLive.Windows/WindowsRtmpAudioSession.cs`：RTMP 声音会话复用同一策略提供器。
- `src/GpAutoLive.App/MainWindow.xaml.cs`：从共享优先级快照生成 0/-6 dB 或静音策略。
- `tests/GpAutoLive.Media.Tests/AudioPcmMixerTests.cs`：覆盖原地 duck、静音、未对齐输入和非法增益。

## 明确未接入

视频 mpv 内部声音、插话文件实际候选池、麦克风可听 PCM 混音、AEC/降噪/AGC/完整 VAD、交叉淡化和可听 PTS 主时钟仍待真实链路实施/验收。当前策略是在解码线程分片边界生效，已进入输出环缓的旧帧不会被回溯修改。

## 验证

- .NET 10 x64 构建 0 警告、0 错误；全量自动化测试 **278 项通过**（Contracts 13、Core 47、Media 86、Windows 107、App 25）。
- 运行资源、PortAudio、FFmpeg 和 mpv 仍保持外置；没有新增第三方依赖或主 EXE 内嵌 DLL。
- v48 发布候选正式安装根目录 8 个运行文件、1,075,431 bytes，`GpAutoLive.exe` 162,816 bytes；符号包 7 个文件、336,290 bytes；媒体运行时 13 个文件、352,365,694 bytes，清单 5/5 哈希匹配。启动关闭冒烟退出码 0、无残留（私有工作集 96,948,224 bytes、工作集 158,990,336 bytes、22 线程、1,102 句柄）；4 秒空闲基线 3 个样本峰值私有工作集 82,997,248 bytes、工作集峰值 142,548,992 bytes、CPU 峰值 1.42%。真实设备与网络门禁不得由本地混音单元测试替代。
