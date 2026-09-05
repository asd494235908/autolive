# C4 FFmpeg PCM 解码边界实现记录

日期：2026-09-02  
状态：代码已接入·待普通声音候选编排与长稳验收

## 已实现

- `FfmpegPcmDecodePlanBuilder` 对 FFmpeg 路径、媒体路径、音频轨道、采样率、声道和 Windows 命令长度做固定校验。
- 解码参数固定选择首条音频轨道（`0:a:0`），输出 `f32le` 到 `pipe:1`，不读取视频、不使用 Shell、不访问网络。
- 计划在输入前加入 FFmpeg `-re`，按源媒体时钟有界读取，避免解码器以磁盘速度填满环缓造成音频加速或大量覆盖；暂停门控仍位于 stdout 读取边界。
- `WindowsFfmpegPcmDecoder` 以隐藏进程、Job Object、取消/超时和 2 秒清理预算运行；标准输出按块读取，二选一写入固定容量 `AudioPcmRingBuffer` 或 `FinalPcmBus`，不把完整音轨载入内存。
- 字节/样本/声道边界、非有限样本和环缓满载均有界处理；快照不暴露源路径、命令或音频正文。
- stderr 使用固定 4 KiB 读取缓冲；达到 16 KiB 诊断计数上限后不再累计，仍持续读取并丢弃后续字节，直到 FFmpeg 退出或取消，避免错误管道反压阻塞进程；不保留或传播 stderr 原文。

## 明确未实现

- 当前是单次有限解码入口，尚未接入 N/N+1 候选预载、A/B 切换、循环重启、30ms 交叉淡化或插话文件调度。
- 已由 `WindowsAudioPlaybackController` 把解码任务接到 `FinalPcmBus`/PortAudio 输出流，且由 `MainWindow` 纯音频播放/暂停/恢复/停止入口调用；N/N+1 目前只有 `AudioCyclePrewarmCoordinator` 纯逻辑调度边界，尚未接入真实候选解码和可听 PTS 主时钟；PortAudio 设备健康观察与有界重开已接入，真实设备拔插/睡眠唤醒和长稳仍保持“代码已接入·待验收”。

## 验证

计划/生命周期边界测试已加入；新增 Windows `cmd.exe` 夹具持续写入超过 16 KiB 的 stderr 后正常退出，以及取消持续 stderr 时进程树终止、读取任务 Join 和解码器返回 `Cancelled` 的回归测试。显式设置 `AUTOLIVE_TEST_FFMPEG` 与 `AUTOLIVE_TEST_PORTAUDIO_DLL` 后，本机 FFmpeg 正弦 WAV fixture 实测成功解码、写入最终 PCM 总线并完成 PortAudio 输出排空。完整候选链路、真实声卡和长稳仍需目标 Windows 设备门禁。
