# C5 固定话术 Windows 语音实现记录

日期：2026-09-02  
范围：仅修改 `desktop-csharp-windows`；`desktop/` Rust/Tauri/React 仍为只读参考。

## 1. 本次实现结论

本次完成“Windows 本地语音适配边界”，没有把它标记为真实语音已验收。

- 适配器通过注入的 `IWindowsSpeechBridge` 与本机语音实现解耦，测试不需要安装语音包或创建音频设备。
- 生产桥接器 `WindowsSapiSpeechBridge` 使用 Windows SAPI Automation COM `SAPI.SpVoice`，不新增 NuGet，不把语音 DLL 嵌入主 EXE。
- 每个 SAPI 操作只创建一个后台 STA 线程；COM 对象在同一 STA 线程释放。
- 适配器只有一个活动操作。新操作抢占旧操作，麦克风优先时拒绝新固定话术。
- 启动确认默认预算为 1500ms；超时、取消、voice 不可用和 SAPI 不可用均按稳定错误分类失败关闭。
- 快照只保留 `operation_id`、脱敏 `voice_key` 和错误摘要，不保存话术正文或 SAPI token ID。

## 2. 新增边界

| 文件 | 职责 |
| --- | --- |
| `src/GpAutoLive.Windows/WindowsSpeechContracts.cs` | 适配器状态、错误分类、脱敏 voice 描述、桥接器与操作接口 |
| `src/GpAutoLive.Windows/WindowsSystemSpeechAdapter.cs` | 固定话术单操作所有者、启动超时、抢占、取消、终态和释放 |
| `src/GpAutoLive.Windows/WindowsSapiSpeechBridge.cs` | SAPI COM Automation、voice token 内存解析、STA 线程和 COM 资源释放 |
| `tests/GpAutoLive.Windows.Tests/WindowsSystemSpeechAdapterTests.cs` | 注入替身覆盖启动、完成、取消、抢占、超时、失败、关闭和脱敏 |

适配器已接入 `MainWindow` 的固定话术卡片，支持本地文本提交、单操作取消和脱敏状态投影，但尚未接入音频总线。固定话术的静音/恢复、插话优先级、普通声音候选和 PortAudio 仍由后续唯一音频所有者负责。

## 3. 操作生命周期

```text
Speak(command)
  -> FixedSpeechStateMachine: Starting
  -> bridge.StartAsync(text, redactedVoiceKey)
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
- 适配器不创建无界队列、不启动长期后台任务、不生成音频文件。

## 6. 验证结果

已执行：

```powershell
dotnet build tests/GpAutoLive.Windows.Tests/GpAutoLive.Windows.Tests.csproj -c Release --no-restore -p:Platform=x64 -p:NoWarn=1591
dotnet test tests/GpAutoLive.Windows.Tests/GpAutoLive.Windows.Tests.csproj -c Release --no-build -p:Platform=x64
dotnet format GpAutoLive.Windows.slnx analyzers --diagnostics IDE0005 --verify-no-changes --no-restore
```

结果：Windows 测试 `60/60` 通过，`IDE0005` 格式检查通过，统一解决方案构建 `0` 警告、`0` 错误。

## 7. 尚未验收项目

以下项目必须在真实 Windows 10/11 x64 环境逐项验收后，才能把状态从“代码已接入边界·待验收”改为“已实现并生效”：

1. `SAPI.SpVoice` COM 激活和中文语音包枚举。
2. 中文 voice 实际发音、voice 选择和输出设备切换。
3. 启动确认延迟、取消延迟、CPU/内存和长时间播放稳定性。
4. SAPI 朗读期间主效果音频静音，完成/取消/失败后的恢复时序。
5. 切源、窗口关闭、设备拒绝和系统睡眠恢复场景。

在上述门禁完成前，C5 固定话术 Windows 语音状态应显示为：

> 正式需求·代码已接入边界，真实 SAPI 语音未验收；未宣称已发声或已接入音频总线。
