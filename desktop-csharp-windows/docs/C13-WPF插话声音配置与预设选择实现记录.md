# C13 WPF 插话声音配置与预设选择实现记录

日期：2026-09-03

## 本轮落地

- `GpAutoLive.Contracts/InterludeAudioContracts.cs` 复刻 Rust/Tauri `InterludeConfig` 的稳定 JSON 字段：固定/随机选择、p01～p22 预设白名单、多轨选择 1～4、每次/周期变化、插话间隔、音量、duck 深度与 attack/release 边界。
- 默认值保持参考端口径：固定预设 `p01`，随机候选 `p01`～`p20`，插话间隔 8～13 秒，预设周期 8～15 秒，duck 深度 -60 dB，attack/release 50/250 ms。
- `GpAutoLive.Core/InterludeAudioSelector.cs` 在控制线程完成有界随机选择、最多 4 条不重复预设、多轨数量限制和周期窗口复用；使用可注入 `Random`，测试可复现，实时回调不触碰该选择器。
- `GpAutoLive.Core/Configuration/InterludeAudioConfigStore.cs` 通过现有版本化 JSON + 原子写入保存到 `%LocalAppData%\\GpAutoLive\\profiles\\interlude\\default.json`。目录快照仍只由 `InterludeFilePoolService` 持有，音频正文不落盘。
- 重启后只恢复脱敏目录配置并显示“已保存目录”，不会在未再次确认前自动递归枚举用户目录；重新选择目录才建立新的原子候选快照。
- WPF 目录选择/清空会同步更新插话配置；启动插话时执行配置校验与预设选择，并在状态栏显示选择结果。p01～p22 仍是选择合同，具体声音 DSP 的真实参数映射继续按实机门禁管理，避免把“选中了预设”误报为“声音效果已生效”。
- `AudioPcmMixEnvelopeOptions` 已接入输出源：在固定 PortAudio/RTMP 回调缓冲内按配置采样率进行有界线性 attack/release 增益过渡；未传入过渡参数的测试/兼容调用保持立即切换。
- `CreateBaseAudioMixPolicy` 已使用配置中的音量和 duck 深度；固定话术/麦克风优先级的静音与插话混音边界保持不变。

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

- 22 个预设的真实滤波参数映射、预设声音变化是否可听、周期调度与 attack/release 逐帧曲线仍待 Windows 10/11 x64 实机验证；代码已完成固定缓冲过渡，但尚未通过真实声卡听感与长稳门禁。当前不启动第二输出流、不把整段文件载入内存。
- 真实声卡拔插/睡眠唤醒、视频 mpv 内部声音、AEC/降噪/AGC/完整 VAD、RTMP 网络和 30 分钟长稳仍未通过门禁。
