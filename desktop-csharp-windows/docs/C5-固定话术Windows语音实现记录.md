# C5 固定话术 Windows 语音实现记录

日期：2026-09-06
范围：仅修改 `desktop-csharp-windows`；`desktop/` Rust/Tauri/React 仍为只读参考。

## 1. 本次实现结论

本次完成“Windows 本地语音适配边界”，没有把它标记为真实语音已验收。

- 适配器通过注入的 `IWindowsSpeechBridge` 与本机语音实现解耦，测试不需要安装语音包或创建音频设备。
- 生产桥接器 `WindowsSapiSpeechBridge` 使用 Windows SAPI Automation COM `SAPI.SpVoice`，不新增 NuGet，不把语音 DLL 嵌入主 EXE。
- 每个 SAPI 操作只创建一个后台 STA 线程；COM 对象在同一 STA 线程释放。
- 适配器只有一个活动操作。新操作抢占旧操作，麦克风优先时拒绝新固定话术。
- 启动确认默认预算为 1500ms；超时、取消、voice 不可用和 SAPI 不可用均按稳定错误分类失败关闭。
- 快照只保留 `operation_id`、脱敏 `voice_key` 和错误摘要，不保存话术正文或 SAPI token ID。
- P0 接线已补齐：生产桥接将 SAPI 输出绑定到 `SpMemoryStream`，按 48kHz、16-bit、单声道/双声道 PCM 分片送入既有最终 PCM 总线；没有总线时 fail-closed，不回退到 SAPI 默认音频设备。
- PCM sink、固定容量总线、单活动操作、取消/抢占/关闭和终态清理共同构成边界；真实 SAPI voice、声卡和 RTMP-only 输出仍未验收。

## 2. 新增边界

| 文件 | 职责 |
| --- | --- |
| `src/GpAutoLive.Windows/WindowsSpeechContracts.cs` | 适配器状态、错误分类、脱敏 voice 描述、桥接器与 PCM sink 接口 |
| `src/GpAutoLive.Windows/WindowsSystemSpeechAdapter.cs` | 固定话术单操作所有者、启动超时、抢占、取消、终态和释放 |
| `src/GpAutoLive.Windows/WindowsSapiSpeechBridge.cs` | SAPI COM Automation、PCM 捕获、voice token 内存解析、STA 线程和 COM 资源释放 |
| `src/GpAutoLive.Media/FinalPcmBus.cs` | 保持既有本机/RTMP overlay 消费者边界，并提供终态尾部清理 |
| `tests/GpAutoLive.Windows.Tests/WindowsSystemSpeechAdapterTests.cs` | 注入替身覆盖启动、完成、取消、抢占、超时、失败、关闭、脱敏和 PCM 接线 |

适配器已接入 `MainWindow` 的固定话术卡片，`MainWindow` 通过 `WindowsAudioPlaybackController.ActiveFinalPcmBus` 提供当前总线。固定话术进入既有 overlay 混音分支，优先级协调器静音原媒体并允许固定话术 overlay；插话、普通声音候选和 PortAudio 仍由同一音频所有者负责。没有活动本地最终 PCM 会话时，固定话术拒绝启动。

## 3. 操作生命周期

```text
Speak(command)
  -> FixedSpeechStateMachine: Starting
  -> bridge.StartAsync(text, redactedVoiceKey, pcmChannels, pcmSink)
  -> SAPI.SpMemoryStream: 48kHz/16-bit PCM
  -> FinalPcmBus overlay -> PortAudio / attached RTMP consumer
  -> operation.Started (<= 1500ms)
  -> Playing
  -> operation.Completion
  -> Completed / Cancelled / Failed
```

- `Started` 未在启动预算内完成：请求取消并进入 `Failed(StartupTimeout)`。
- `CancelAsync(operation_id)` 只接受当前活动 ID；过期 ID 不改变新操作。
- 新 `Speak` 先取消并有界释放旧操作，再等待新操作启动。
- 旧操作的迟到 `Completed`/`Failed` 只由操作身份门禁丢弃，不得覆盖新操作。
- WPF 登出路径先取消当前固定话术，再提交认证状态变化，避免授权门禁变化后遗留本地语音操作。
- `DisposeAsync` 取消活动操作，等待操作监视器和桥接器在预算内收敛。
- 总线关闭、固定话术取消、被新话术抢占或进入任意终态时，丢弃未消费 overlay 尾部，避免优先级结束后旧 SAPI PCM 泄漏到原媒体。

## 4. voice 脱敏和安全边界

- SAPI 注册表中的完整 token ID 只在 `WindowsSapiSpeechBridge` 内存中使用。
- voice key 为 token ID 的 SHA-256 十六进制摘要（64 个字符）；适配器拒绝原始 token、短 key 和包含控制字符的 key。
- voice 目录只返回摘要 key、受限语言标签和固定显示名“本地系统语音”，不返回注册表路径或原始 voice 名称。
- SAPI/COM 异常正文不进入公开错误；错误统一收敛为稳定错误分类和短错误摘要。
- 不写入 JSON、INI、日志或控制面；不保存语音 token、话术正文或中间音频文件。

## 5. 资源和线程约束

- SAPI COM 对象、voice token、voice token 集合和状态 RCW 均在创建它们的 STA 操作线程释放。
- `CancellationTokenSource` 在线程 `finally` 完成后才释放；即使释放等待超时，也不会在线程仍运行时提前释放 CTS，避免后台 STA 访问已释放对象。
- 操作释放等待为有界预算；正常路径等待 `threadExited`，退出路径不传播 COM 异常。
- 适配器不创建无界队列、不启动长期后台任务、不生成音频文件；SAPI 内存捕获受 `24 MiB` 上限约束，PCM 分片按约 85ms 节奏送入固定容量总线。

## 6. 2026-09-06 P0 增量状态

- 新增失败先行测试：没有活动最终 PCM 总线时，适配器不得调用桥接器或回退默认设备。
- 新增闭环测试：注入的 PCM16 数据进入 `FinalPcmBus.OutputOverlayBuffer`，固定优先级静音原媒体；取消后本机/RTMP overlay 待消费帧均清空。
- 生产 `WindowsSapiSpeechBridge` 使用 Windows 自带 `SAPI.SpMemoryStream` 作为唯一捕获路径，不新增 NuGet 或第二音频后端；SAPI/COM 对象继续在操作 STA 线程创建和释放。
- 本轮使用仓库锁定的 `desktop-csharp-windows/.tools/dotnet/dotnet.exe` 完成 `WindowsSystemSpeechAdapterTests` `11/11`；与 RTMP 受影响测试组合复验 `30/30`，项目编译成功。该结果只覆盖注入替身和边界逻辑，不替代真实 SAPI voice、声卡或 RTMP-only 业务验收。

## 7. 历史验证结果

已执行：

```powershell
dotnet build tests/GpAutoLive.Windows.Tests/GpAutoLive.Windows.Tests.csproj -c Release --no-restore -p:Platform=x64 -p:NoWarn=1591
dotnet test tests/GpAutoLive.Windows.Tests/GpAutoLive.Windows.Tests.csproj -c Release --no-build -p:Platform=x64
dotnet format GpAutoLive.Windows.slnx analyzers --diagnostics IDE0005 --verify-no-changes --no-restore
```

结果：Windows 测试 `60/60` 通过，`IDE0005` 格式检查通过，统一解决方案构建 `0` 警告、`0` 错误。

## 8. 尚未验收项目

以下项目必须在真实 Windows 10/11 x64 环境逐项验收后，才能把状态从“代码已接入边界·待验收”改为“已实现并生效”：

1. `SAPI.SpVoice` COM 激活和中文语音包枚举。
2. 中文 voice 实际发音、voice 选择和输出设备切换。
3. 启动确认延迟、取消延迟、CPU/内存和长时间播放稳定性。
4. SAPI 朗读期间主效果音频静音，完成/取消/失败后的恢复时序。
5. 切源、窗口关闭、设备拒绝和系统睡眠恢复场景。

在上述门禁完成前，C5 固定话术 Windows 语音状态应显示为：

> 正式需求·代码已接入最终 PCM 边界，真实 SAPI 语音/声卡/RTMP-only 消费未验收；未宣称真实设备已发声或发布成功。
