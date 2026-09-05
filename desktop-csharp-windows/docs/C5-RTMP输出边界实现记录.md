# C5 RTMP/RTMPS 输出边界实现记录

日期：2026-09-02  
范围：仅修改 `desktop-csharp-windows`；`desktop/` Rust/Tauri/React 仍为只读参考。

## 2026-09-05 声音参数进入独立 RTMP PCM 会话

- 修复 `WindowsRtmpAudioSession` 独立拥有最终 PCM 时未消费 `AudioEffectParams` 的缺口：WPF 当前“声音处理”开关开启时，将当前 `CreateCurrentAudioEffectParameters()` 传入既有 `FfmpegPcmDecodePlanBuilder`，关闭时传入 `null`，因此同一 `-af` 滤镜链进入 RTMP 的最终 PCM；共享本地最终 PCM 总线的路径保持原有单一生产者边界。
- 失败先行测试以无效音频参数和无效 RTMP 目标组合证明接线前会错误进入宿主校验并返回 `StartFailed`；接线后在启动宿主前稳定返回 `InvalidArguments`。未新增依赖、后端、队列或音频抽象。
- 本轮仍未宣称真实 ZLMediaKit/RTMPS 握手、远端音频可听性或设备稳定性通过；这些继续保持待验收。

## 2026-09-03 本轮边界收口

- `Process.Exited` 进入失败终态后，在取得 PCM 写入串行锁的前提下关闭 stdin、取消有界 stderr 读取并清除 PID/进程资源；启动阶段先完成宿主字段初始化再启用退出事件，避免快速退出把悬空资源写回 `Publishing`。
- 主线程复核移除快速退出后的旁路强制释放：若 PCM 写入仍在执行，退出事件只按同一有界串行回收路径处理；stderr 读取任务在进程资源释放前执行有界等待，避免关闭后遗留读取任务。
- 停止顺序调整为先取消读取、回收进程树，再等待 PCM 写入完成；若写入者在预算内未结束，不关闭仍被使用的 stdin，也保留资源供下一次 `StopAsync` 重试，避免画面进程与最终 PCM 泵半回收。
- 新增本地 `cmd.exe` 进程夹具回归，验证自然退出后的 `Failed`、脱敏 `rtmp_process_exited`、空 PID 和关闭 PCM 输入；该夹具不启动 FFmpeg、不连接 ZLMediaKit。
- 本轮未把 `Process.Exited` 接到自动重连；远端断开证明、活动源身份和音画成组重建仍由上层提供。

## 本轮验证

- `E:\aotlve\desktop-csharp-windows\.tools\dotnet\dotnet.exe build src/GpAutoLive.Windows/GpAutoLive.Windows.csproj -c Release --no-restore -p:Platform=x64 -p:OutputPath=E:\aotlve\desktop-csharp-windows\.tmp\c5-rtmp-build\`：通过，0 警告/0 错误。
- `... dotnet.exe test tests/GpAutoLive.Windows.Tests/GpAutoLive.Windows.Tests.csproj -c Release --no-restore -p:Platform=x64 -p:OutputPath=... --filter "FullyQualifiedName~WindowsRtmpOutputManagerTests|FullyQualifiedName~WindowsRtmpReconnectCoordinatorTests"`：15/15 通过；同项目不带筛选的全量回归：194/194 通过。
- `dotnet format GpAutoLive.Windows.slnx --verify-no-changes --no-restore`：通过；`tools/verify-scope.ps1`：通过；`git diff --check`：无错误（仅报告既有换行提示）。
- 未执行真实 ZLMediaKit/RTMPS 握手、鉴权、远端断线恢复、GPU 编码器矩阵或长稳；这些仍保持待验收，不把本地进程成功解释为远端 Publishing。

## 本次实现

- `GpAutoLive.Contracts/RtmpContracts.cs` 固定 RTMP 配置、状态、播放源身份、错误分类和脱敏地址合同。
- 地址只接受 `rtmp://` / `rtmps://`，拒绝凭据、片段、非法主机/端口、空发布路径和控制字符；至少选择画面或声音一项。
- 输出视频尺寸、帧率、视频/音频码率均有上限；视频帧率固定为 25/30/50/60 FPS。
- `RtmpOutputRules.RedactTargetUrl` 只保留协议与主机/端口，路径、query 和 stream key 全部替换为 `<redacted>`。
- `GpAutoLive.Media/RtmpFfmpegCommandBuilder.cs` 使用 `ArgumentList` 对应的不可变参数计划：画面直接读取已探测源媒体，声音预留最终 48kHz 双声道 PCM `pipe:0`，默认输出 H.264 + AAC FLV。
- `GpAutoLive.Media/RtmpEncoderProbe.cs` 使用每候选一帧本地 `lavfi` 黑帧和 5 秒超时，按 `h264_nvenc → h264_amf → h264_qsv → h264_mf → libopenh264` 顺序选择本机 FFmpeg 首个可用编码器；不访问网络、不回显 FFmpeg 原文。
- `MainWindow` 右侧输出卡片接入地址、画面/声音轨道、本地“校验输出配置”以及“开始推流/停止推流”按钮；画面轨道使用 RTMP 宿主，声音/音画轨道使用 `WindowsRtmpAudioSession` 接入最终 PCM 分流。
- 编码器按 `h264_nvenc → h264_amf → h264_qsv → h264_mf → libopenh264` 单向降级；WPF 启动前已接入本机逐候选探测，命令计划本身仍不启动进程、不访问网络。
- `GpAutoLive.Windows/WindowsRtmpOutputManager.cs` 已接入 Windows 长生命周期宿主：隐藏 `Process` + `ArgumentList`、Job Object 优先回收、stderr 固定 64 KiB 读取上限、停止 Join 和最终 PCM `pipe:0` 有界写入；不自动重试，不把地址、路径、命令行或 stderr 原文投影到 UI。
- `RequiresFinalPcmInput` 与 `WriteFinalPcmAsync` 明确最终声音所有者边界；在 PortAudio/最终声音总线接入前，不会把“声音推流”伪装成可用。含声音配置不会传 `-nostdin`，避免关闭 PCM 输入。

## 自动化验证

`RtmpContractTests` 覆盖协议、凭据拒绝、轨道/尺寸边界、snake_case 序列化、脱敏和编码器顺序；`RtmpFfmpegCommandBuilderTests` 覆盖画面+声音参数、纯音频参数、源类型和未知编码器拒绝。

当前全量测试为 Contracts 13、Core 39、Media 53、Windows 60、App 18，共 183 项通过。

本次后续增量另加入编码器探测纯逻辑 4 项与显式 FFmpeg 夹具 1 项；当前全量测试为 Contracts 13、Core 45、Media 84、Windows 104、App 21，共 267 项通过。

## 当前状态与尚未验收

- WPF 已可按已验证资源启动/停止画面、声音或音画宿主，并在画面启动前完成本机 H.264 逐候选探测；以下仍保持“代码已接入·待验收”或“正式需求·待实施/未接入”：ZLMediaKit/RTMPS 网络联调、断线重试策略、GPU 编码器矩阵、30 分钟稳定性和最终 PCM 的目标网络时序。当前 UI/宿主边界不等价于真实网络推流通过，未完成这些门禁前不宣称可用。
- WPF 导入、重排、移除、清空和媒体上一项/下一项切换会先停止活动 RTMP 宿主，避免旧媒体身份继续发布；宿主不自行监听播放池，其他调用方仍需在身份变化时主动停止。

## 最新本地交付包（v43，2026-09-03）

- `artifacts/csharp-windows-controller-20260903-v43` 已包含当前视频 EOF 观察、媒体变更生命周期、WPF 可访问性文案、最新 IPC/运行资源白名单修复和音频 EOF 取消源生命周期修复；正式安装根目录已移出 PDB/XML，仅 8 个运行文件、1,027,815 bytes，`GpAutoLive.exe` 162,816 bytes；符号独立放在 `artifacts/csharp-windows-symbols-20260903-v43`。
- 外置媒体 staging 仍为 13 个文件、352,365,694 bytes，5/5 资源大小与 SHA-256 全部匹配；本机 H.264 探测、FFmpeg→FinalPcmBus→PortAudio 夹具和真实 mpv `Original/Cpu4/Gpu83` 播放时间/EOF 夹具已通过，真实 ZLMediaKit/RTMPS 网络仍未验收。

## 本轮回归包（v44，2026-09-03）

- `artifacts/csharp-windows-controller-20260903-v44` 保持 8 个运行文件，根目录 1,037,031 bytes；PDB/XML 独立在 `artifacts/csharp-windows-symbols-20260903-v44`（7 个文件、324,989 bytes）。
- 外置媒体运行时继续为 13 个文件、352,365,694 bytes；资源清单 5/5 哈希匹配。267 项全量测试、真实 mpv 三模式、FFmpeg→PortAudio 健康观察夹具均通过；真实 ZLMediaKit/RTMPS 网络仍未验收。

## 发布验证包（v14）

- `artifacts/csharp-windows-controller-20260902-v14`：发布根目录 15 个文件、1,114,706 bytes；`GpAutoLive.exe` 162,816 bytes。
- 外置媒体资源清单 4/4 SHA-256 匹配；媒体 staging 树 352,053,564 bytes，未内嵌到主 EXE。
- 本机启动约 1.8 秒采样：标题 `GpAutoLive`，私有字节 81,223,680、工作集 137,871,360、21 线程、1,077 句柄；`CloseMainWindow` 退出码 0，退出后无 `GpAutoLive/mpv/ffmpeg/ffprobe` 残留。该冒烟不是性能达标证明。
