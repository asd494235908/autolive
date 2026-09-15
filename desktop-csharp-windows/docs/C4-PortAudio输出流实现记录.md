# C4 PortAudio Windows 输出流实现记录

## 状态

当前状态：代码已接入·设备健康探测与有界恢复已接入，待普通声音候选、真实丢设备和长稳验收。

## 2026-09-07 原生调用预算与资源所有权修复

- 输入、输出流复用专用 `WindowsPortAudioOperationOwner`，一个流只有一条在途原生调用链；初始化、打开、启动、暂停、恢复、关闭、终止和健康查询均在后台执行。普通操作预算为 2 秒，等待已开始的健康查询也计入同一预算；不创建无界命令队列。
- 预算到期只结束调用方等待并返回失败。原 Task、stream、DLL 和回调对象仍保留；同一 owner 等旧调用结束后串行收尾，后续 `StopAsync`/`Dispose` 可继续等待或重试，禁止覆盖尚未回收的流。`Dispose` 未确认释放会明确抛出失败，不再返回假成功。
- `Snapshot` 只读取受管缓存和回调计数；到期健康刷新由同一 owner 在后台执行，间隔至少 250 ms，迟到健康结果不能覆盖停止状态。原生查询阻塞不会持有快照锁，快照不再同步进入 PortAudio。
- 只在原生启动确认后才标记 `IsRunning`。停止先撤销回调消费资格，确认 `CloseStream` 后才释放 stream/回调缓冲；确认 `Terminate` 后才卸载 DLL。关闭失败保留资源供重试，设备恢复不能通过忽略停止错误来吞掉 Close/Terminate 失败。一个 `GCHandle` 在原生回调有效期间保留流 owner，防止受管 delegate 先被回收。
- `IWindowsPortAudioStreamNative` 仅用于现有 ABI 与可控原生故障测试之间的隔离，不替换 PortAudio，不新增音频库、sidecar、缓存队列或后台重试服务。
- 故障注入覆盖 Init/Open/Start/Query、Stop/Close/Terminate 阻塞，取消，重复停止，关闭失败和 Dispose 重试。实际执行结果记录在本轮修复报告；这组自动化不等同于真实驱动拔插、睡眠唤醒或长稳验收。
- 进程内无法安全强杀永久卡死的驱动调用。上述边界保证受管等待有预算和原生资源不被误释放；永久卡死时仍保留原 owner，并报告未回收。需要强制回收该故障时必须另行设计进程隔离，当前未实现。

## 本轮实现

- `WindowsPortAudioNative` 只解析设备枚举和输出流所需的 PortAudio v19 导出；DLL 必须是资源清单验证后的 `portaudio_x64.dll`，不搜索系统 PATH。
- `WindowsPortAudioOutputStream` 是单一输出流所有者，负责初始化、打开、启动、停止、关闭和终止；`Dispose` 会先标记关闭，再有界停止原生流。
- 流配置限制为设备索引、1～8 声道、8 kHz～384 kHz、16～4096 帧，格式固定为交错 float32；路径、资源、配置、取消和关闭错误使用稳定分类，不回显原生异常或完整路径。
- PortAudio 回调只读取 `AudioPcmRingBuffer`，使用预分配回调数组和 `Marshal.Copy`，没有文件/网络 I/O。环缓欠载时输出静音并累计欠载帧，无法满足回调边界时返回 `Abort`。
- 设备枚举和输出流共用动态 ABI；枚举器的探测调用通过串行门禁保护，避免并行初始化/终止破坏 PortAudio 全局状态。
- `FinalPcmBus` 将最终 PCM 有界复制到本机和 RTMP 两个消费者环缓；`WindowsRtmpFinalPcmPump` 只把 RTMP 环缓串行写入已启动宿主的 `pipe:0`，避免从源媒体重复读取声音。
- `WindowsPortAudioInputStream` 提供显式启用的本地输入流，使用预分配回调缓冲写入固定容量环缓；它不执行识别、模型调用或网络上传。
- `MicrophoneInterludeGate` 只提供本地 RMS 电平、迟滞和 hangover 状态，作为后续 VAD 接入的窄边界；不会把能量门控宣传为 AEC/降噪/AGC。
- `WindowsPortAudioNative` 对 `Pa_IsStreamActive`/`Pa_IsStreamStopped` 使用可选导出；缺失导出返回 `Unknown`，不把旧版 DLL 误判为故障。输出快照额外记录硬件状态、回调次数和最后一次原生状态旗标。
- `WindowsPortAudioOutputRecovery` 只在会话未暂停且状态为 `Stopped`、`Inactive` 或 `QueryError` 时工作，按最多 3 次、250/500/1000 ms 退避重新打开同一设备配置；第一次重开失败关闭原生句柄后，后续尝试仍保留恢复资格，不会被误判成“尚未启动”的初始调用。恢复委托发生可预期的原生/对象生命周期异常时统一投影为脱敏 `RestartFailed`，仍走相同有界重试；恢复失败立即取消会话并 fail-closed，不清空 PCM 环缓、不无限重试。
- `WindowsPortAudioInputStream` 同样投影 `Pa_IsStreamActive`/`Pa_IsStreamStopped` 的健康状态、回调次数和最后一次状态旗标；缺少可选导出时返回 `Unknown`，不把兼容性差异误报为设备故障。麦克风门控观察器发现 `Stopped`、`Inactive` 或 `QueryError` 后停止输入并进入 `Failed`，不自动无限重开，用户可在修复设备后重新启用。

## 明确未接入

- 普通声音候选的 N/N+1 预载、A/B 切换、duck 和插话优先级；
- AEC、降噪、AGC、VAD、麦克风优先级抢占和监听开关；
- 固定话术 SAPI 与主音频总线的静音/恢复协调；
- 普通声音会话的可听 PTS 主时钟；设备恢复已有代码边界，但真实拔插/睡眠唤醒仍未验收；
- 真实 30 分钟声卡、独占/共享模式、采样率切换、睡眠唤醒和 Windows 10/11 矩阵。

## 验证

- `WindowsPortAudioOutputStreamTests` 覆盖无效路径、缺失资源、无效配置、取消、停止幂等、关闭后拒绝启动、未创建健康状态、有界退避策略、三次恢复尝试上限、恢复委托原生异常的脱敏分类，以及首次重开失败后下一次仍按恢复路径执行；启用显式 PortAudio 夹具时还验证真实句柄关闭后的第二次重开分类。
- `WindowsPortAudioInputStreamTests` 覆盖未创建输入流的 `NotCreated` 健康回退、回调计数/状态旗标初值、资源/配置/取消和关闭边界；`WindowsMicrophoneInterludeController` 的异步健康观察保持 50 ms 轮询、有界停止和取消 Join。
- `FinalPcmBusTests` 覆盖双路顺序、满载丢旧、形状/容量和关闭排空；`WindowsRtmpFinalPcmPumpTests` 覆盖缺宿主、宿主未运行和关闭来源排空。
- 本机真实 DLL 冒烟使用 48 kHz、双声道、256 帧输出流启动约 500 ms 后停止成功；环缓为空时仅输出静音，未写入媒体或网络。
- 全量测试与发布验证以主实施计划为准；恢复策略已通过纯逻辑边界测试，本机真实输出夹具已走过健康观察器；未通过真实拔插、睡眠唤醒和长稳门禁前，UI 只允许刷新设备，不标记普通声音可用。
