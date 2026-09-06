# C13 WPF 插话声音配置与预设选择实现记录

日期：2026-09-06

## 本轮落地

- `GpAutoLive.Contracts/InterludeAudioContracts.cs` 复刻 Rust/Tauri `InterludeConfig` 的稳定 JSON 字段：固定/随机选择、p01～p22 预设白名单、多轨选择 1～4、每次/周期变化、插话间隔、音量、duck 深度与 attack/release 边界。
- 默认值保持参考端口径：固定预设 `p01`，随机候选 `p01`～`p20`，插话间隔 8～13 秒，预设周期 8～15 秒，duck 深度 -60 dB，attack/release 50/250 ms。
- `GpAutoLive.Core/InterludeAudioSelector.cs` 在控制线程完成有界随机选择、最多 4 条不重复预设、多轨数量限制和周期窗口复用；使用可注入 `Random`，测试可复现，实时回调不触碰该选择器。
- `GpAutoLive.Core/Configuration/InterludeAudioConfigStore.cs` 通过现有版本化 JSON + 原子写入保存到 `%LocalAppData%\\GpAutoLive\\profiles\\interlude\\default.json`。目录快照仍只由 `InterludeFilePoolService` 持有，音频正文不落盘。
- 重启读取到已保存且 `Enabled=true` 的目录时，通过既有 `InterludeFilePoolService` 安全扫描边界恢复原子候选快照；恢复只重建候选池，不启动解码、音频流或自动播放。目录不存在、不可访问或扫描失败时保留已保存配置并显示明确错误，用户可修复目录后重启或重新选择。
- WPF 目录选择/清空会同步更新插话配置；启动插话时执行配置校验与预设选择，并在状态栏显示选择结果。p01～p22 仍是选择合同，具体声音 DSP 的真实参数映射继续按实机门禁管理，避免把“选中了预设”误报为“声音效果已生效”。
- 2026-09-06 收口真实消费缺口：`InterludeAudioSelection.TryCreateBoundedAudioEffectParams` 对空选择、非法 ID 和多轨选择 fail-closed；固定模式与随机单轨将唯一预设 ID 作为 `VoiceLibraryId` 传入 `FfmpegPcmDecodePlanBuilder`，由现有单输入 `FfmpegAudioFilterBuilder` 生成受管的本地确定性音色 EQ。该路径是 ID-only 的最小消费边界，不声称复刻 Rust `audio-value-presets.ts` 的 35 字段或臆造 22 套滤镜。
- 同步修复已有 `InterludeSchedulePlanner` 的触发后计时语义：当前插话结束后再建立下一次有界到期时间，换源仍立即触发，进度显示按整数百分比投影；不改变预设选择或音频资源所有权。
- 2026-09-07 第一层收口将 duck 释放从 WPF UI continuation 移到插话解码 completion 的后台 `finally`，覆盖成功、失败和取消；但真实复测仍出现主音频终止，因此该接线不能单独视为问题已修复。继续追踪确认真正根因位于 RTMP 已接入时的普通声音定时 N→N+1 切换：本机/RTMP 主轨已晋级后，RTMP overlay 分支仍等待旧总线关闭，而旧总线又等待所有分支晋级确认，最终触发 `candidate_activation_timeout` 并终止整个主音频会话。当前 `FinalPcmBusTrackSwitch` 让本机和 RTMP overlay 都跟随已确认主轨强制晋级；有真实消费者的 RTMP 主轨仍自然晋级，不放宽其确认边界。
- 2026-09-07 修复下一轮插话进度被音频宿主瞬时状态清零的问题：自动调度的“配置/文件池/媒体仍有效”与“当前允许启动插话”分开判断。插话结束后，下一轮等待进度继续按真实调度窗口从 0% 推进；本机或 RTMP 音频宿主短暂不可用时不启动新插话，进度到 100% 后等待，宿主恢复才触发。真正停止播放、关闭/清空插话配置或候选池失效时仍重置并清零，避免展示过期进度。
- `AudioPcmMixEnvelopeOptions` 已接入输出源：在固定 PortAudio/RTMP 回调缓冲内按配置采样率进行有界线性 attack/release 增益过渡；未传入过渡参数的测试/兼容调用保持立即切换。
- `CreateBaseAudioMixPolicy` 已使用配置中的音量和 duck 深度；固定话术/麦克风优先级的静音与插话混音边界保持不变。
- WPF 自动插话已接入同一配置的间隔边界；调度器只选择文件索引，预设选择仍由 `InterludeAudioSelector` 在启动时完成，避免把文件调度与 DSP 参数生成混成一个状态机。

## 验证

- `dotnet build GpAutoLive.Windows.slnx -c Release --no-restore`：0 警告、0 错误。
- `dotnet test GpAutoLive.Windows.slnx -c Release --no-build --no-restore --logger "console;verbosity=minimal"`：Contracts 16、Core 58、Media 91、Windows 111、App 25，合计 **301 项通过**。
- `dotnet format GpAutoLive.Windows.slnx --verify-no-changes --no-restore`：通过。
- `tools/verify-scope.ps1`：通过，C# 任务未修改 `desktop/`。

## v51 发布复验

- 正式安装候选：`artifacts/csharp-windows-controller-20260903-v51`，根目录 8 个文件、1,137,383 bytes；`GpAutoLive.exe` 162,816 bytes。
- 独立符号包：`artifacts/csharp-windows-symbols-20260903-v51`，7 个文件、370,974 bytes；外置媒体运行时复用 v50 的 13 个硬链接文件，352,365,694 bytes。
- 资源清单逐项验证：mpv、FFmpeg、FFprobe、PortAudio、D3DCompiler 共 5/5 大小与 SHA-256 匹配。
- 5 秒空闲基线：4 个样本，私有工作集峰值 99,041,280 bytes、工作集峰值 163,127,296 bytes、CPU 峰值 5.24%，原始文件为 `artifacts/csharp-windows-baseline-20260903-v51.json`。
- v54 在周期候选配置变化失效边界修正后重新发布；正式安装根目录 8 个文件、1,140,967 bytes，独立符号包 7 个文件、371,850 bytes；5 秒空闲基线 4 个样本，私有工作集峰值 98,762,752 bytes、工作集峰值 162,676,736 bytes、CPU 峰值 4.07%，原始文件为 `artifacts/csharp-windows-baseline-20260903-v54.json`。

## 待验收/后续

- 单轨 ID-only 消费链、22 个预设的完整 Rust 字段映射、预设声音变化是否可听、周期调度与 attack/release 逐帧曲线仍待 Windows 10/11 x64 实机验证；代码已完成固定缓冲过渡，但尚未通过真实声卡听感与长稳门禁。多轨与完整 35 字段映射保持未接入。当前不启动第二输出流、不把整段文件载入内存。

## 2026-09-06 验证

- Core `InterludeAudioSelectorTests`：10/10；覆盖固定单轨、随机单轨、空/非法 ID/多轨消费边界。
- Media `FfmpegPcmDecodePlanTests` 与 `FfmpegAudioFilterBuilderTests`：17/17；确认单轨预设进入受管 `-af` 计划。
- Contracts `InterludeAudioContractTests`：3/3。
- 真实声卡拔插/睡眠唤醒、视频 mpv 内部声音、AEC/降噪/AGC/完整 VAD、RTMP 网络和 30 分钟长稳仍未通过门禁。

## 2026-09-07 插话收尾与进度验证

- `InterludePriorityReleaseTests`：2/2；覆盖解码成功和异常终态均在后台 completion `finally` 释放插话优先级，不等待 UI 投影。
- App `InterludePriorityReleaseTests|EffectCycleProgressProjectionTests` 聚焦筛选：4/4；覆盖 duck 释放与三条进度绑定的有界投影。
- Core `Schedule_` 聚焦筛选：4/4；覆盖插话完成后下一轮进度 `0% → 50% → 100%`、音频宿主不可用时不误触发，以及宿主恢复后触发。
- 集成聚焦筛选 App `24/24`、Core `21/21` 通过；x64 Release 本地构建为 `0` 警告、`0` 错误并已启动新进程。未执行全量测试；真实 PortAudio 主音频听感恢复、RTMP 插话收尾、人工页面连续观察、设备异常和 30 分钟长稳仍待验收。自动化结果不等同于真实声卡或网络业务结果。

## 2026-09-07 插话音量与主音频切换互等修复

- 插话高级区新增 `0%–100%` 音量滑块，按既有产品合同映射为 `-60–0 dB`；拖动时实时更新 `InterludeAudioConfig.VolumeDb` 并进入现有 overlay 混音策略，鼠标释放或键盘调整后复用既有原子 JSON 存储。主媒体音量和 `DuckingDepthDb` 保持独立。
- RTMP 消费者接入时，overlay 分支不再阻塞已完成的普通声音 N→N+1 切换；回归测试写入真实 overlay PCM，并覆盖 RTMP attached、定时提交和四分支晋级确认。
- 主线程集成定向测试：App `13/13`、Core `15/15`、Media `19/19`、Windows 音频/RTMP `23/23`，格式检查通过，x64 Release 本地构建 `0` 警告、`0` 错误并已重新启动开发端；完整 Media 测试由子线程验证为 `141/141`。真实 RTMP + PortAudio + 插话 + 声音周期联合播放、页面点击、重启持久化和长稳仍待验收。
