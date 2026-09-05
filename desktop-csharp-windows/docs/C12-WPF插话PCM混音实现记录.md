# C12 WPF 插话 PCM 混音实现记录

日期：2026-09-03

## 本轮落地

- `GpAutoLive.Media/AudioPcmOutputSource.cs` 增加固定容量输出源边界：基础轨与插话轨在同一预分配回调缓冲内读取和混合，PortAudio 不创建第二个输出流、不复制整段音频、不引入无界队列。
- `AudioPcmMixingOutputSource` 同时支持普通线程和实时回调读取；插话没有帧时按静音处理，基础轨与插话轨声道不一致、目标容量不足或混音参数非法时 fail-closed。
- `FinalPcmBus` 为本机和 RTMP 消费者分别增加插话环缓。插话写入不会污染基础轨统计，关闭时四个固定环缓一起进入可排空状态。
- `WindowsFfmpegPcmDecoder` 增加 `finalPcmOverlay` 有界模式：插话 FFmpeg 只写插话消费者环缓，不重新读取主媒体、不修改基础轨；普通声音路径保持原有解码合同。
- `WindowsAudioPlaybackController` 增加 `StartInterludeAsync/StopInterludeAsync`。主会话只允许一个插话解码任务，插话与主音频共享 pause gate、取消、Join 和 Windows 进程清理；主会话结束时先回收插话再关闭总线。
- 混音模式把基础轨策略统一放到输出源回调，避免解码线程和回调重复 duck；固定话术/麦克风层级会把基础轨和插话轨按既有优先级同时静音。
- WPF“声音与互动”卡片增加“插话试播/停插话”。当前取目录快照排序后的首项，纯音频项播放或暂停时可叠加；插话完成后只释放插话层，主音频继续，不自动重播被打断内容。切换媒体、停止播放、清空插话池、固定话术或麦克风开始说话都会请求插话停止。

## 与 Rust/Tauri 复刻边界

- 复刻范围是“一个主音频输出 + 一个有界插话叠加轨 + 优先级门控”，没有修改 `E:\aotlve\desktop` 下任何 Rust/Tauri/React 文件。
- WPF 纯音频 PortAudio 路径和 RTMP PCM 分流泵均已接入相同的基础轨/插话轨输出源；视频 mpv 内部声音、22 套预设/随机周期、attack/release 曲线、麦克风可听 PCM 和 AEC/降噪/AGC 仍保持“正式需求·待实施/未验收”。
- 插话候选仍只保存在进程内存目录快照；不写普通 JSON/INI，不把音频正文写入日志或 IPC 快照。FFmpeg、PortAudio 和 Job Object 继续由已验证资源清单与 Windows 受管生命周期统一管理。

## 验证

- `dotnet build GpAutoLive.Windows.slnx --no-restore`：0 警告、0 错误。
- `dotnet test GpAutoLive.Windows.slnx --no-build --no-restore --logger "console;verbosity=minimal"`：Contracts 13、Core 52、Media 90、Windows 111、App 25，合计 **291 项通过**。
- `dotnet format GpAutoLive.Windows.slnx --verify-no-changes --no-restore`：通过。
- `tools/verify-scope.ps1`：通过，C# 任务未修改 `desktop/`。

## v50 发布复验

- 正式安装候选：`artifacts/csharp-windows-controller-20260903-v50`，根目录 8 个运行文件、1,114,855 bytes；`GpAutoLive.exe` 162,816 bytes。
- 独立符号包：`artifacts/csharp-windows-symbols-20260903-v50`，7 个文件、353,084 bytes。
- 外置媒体运行时复用 v49 的 13 个硬链接文件、352,365,694 bytes；5/5 资源大小与 SHA-256 清单匹配，不重复占用媒体库空间。
- 使用仓库 `.tools/dotnet` 启动发布 DLL 的关闭冒烟：`CloseMainWindow=True`、退出码 0、无 `GpAutoLive/mpv/ffmpeg/ffprobe` 残留。该发布包保持 `--self-contained false`，目标机器需要已安装 .NET 10 Desktop Runtime；直接在仅有 .NET 8 的系统启动 EXE 会按预期报告框架缺失。
- 5 秒空闲基线：3 个样本，私有工作集峰值 97,406,976 bytes、工作集峰值 159,248,384 bytes、CPU 峰值 2.65%，原始文件为 `artifacts/csharp-windows-baseline-20260903-v50-final6.json`。该数据用于同机回归；与前次同机采样存在启动时序波动，不等价于 30 分钟门禁。

## 待验收/后续

- 需要在 Windows 10/11 x64 实机使用真实 FFmpeg、PortAudio 和声卡验证：插话可听结果、基础轨 -6 dB duck、固定话术/麦克风抢占时静音、暂停/恢复、设备重开、自然 EOF 和 30 分钟长稳。
- 需要在 Windows 10/11 x64 实机验证 RTMP 分流的插话可听结果、断线/重连和 30 分钟稳定性；代码接线完成不等价于网络推流门禁通过。
