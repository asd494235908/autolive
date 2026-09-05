# GpAutoLive C# 与 Rust 桌面端功能同步长任务实施方案

- 版本：v1.77
- 日期：2026-09-05
- 状态：Phase 2 C# 黄金路径修复已接入；导入/媒体池选择、媒体池编辑控件、拖放候选白名单、mpv CPU4 视频效果及四值运行时回读、基于真实播放 PTS 的 5–8s 视频周期触发、FFmpeg 实时声音子集（含已知时长淡出、自然动态模式和确定性本地音色预设）、按可听位置预载并按 PortAudio 输出帧目标提交的声音周期候选、视频音轨→PortAudio、系统生成 EQ dB 单位、GPU83 完整 83 项 C# 映射与完整 shader 资源包、启动/更新固定白名单回读、完整 shader 哈希门控下的 WPF GPU83/旧包 CPU4 选择、音频 N/N+1 单一输出切换、基于 PortAudio `timeInfo` 的可听时钟投影和正常容量范围内的 PCM 背压、播放态视频效果更新后的 mpv 下一帧号观察、视频启动后的 mpv 首帧帧号门禁、视频启动 `GPU83 → CPU4 → Original` 单向降级、WGC `FrameArrived` 事件唤醒取帧、同一捕获线程的有界轮询、WGC 绑定顶层最终效果窗口 HWND、普通 WPF 与实际 `FinalEffectWindow` 的 WGC 帧池启动/停止隔离门禁、WGC D3D11 设备的 `BGRA_SUPPORT|VIDEO_SUPPORT` 创建标志、真实硬件 adapter 选择和 WGC surface→DXGI texture 解包已接入；视频播放中切换声音开关会停止并从 mpv 当前位置重建 FFmpeg 声音会话；视频画面启动不再因独立 PortAudio 设备失败而回滚；音频计划与输出采样率不一致时在启动前 fail-closed；RTMP 无音频停止路径已修正为只释放实际取得的音频串行锁；App 测试程序集已固化串行执行，消除共享 WPF Dispatcher/真实 mpv 资源的并行竞态。GPU83 原生 Win32 窗口和实际 WPF `FinalEffectWindow` 最终像素均已通过显式夹具，目标显卡矩阵、真实声卡稳定性、过载重建、远端输出与人工页面验收仍待验收
- 本轮并行增量：`MediaPoolOwner.ReplaceAll` 已修复为仅按替换后池长度校验，`Append` 继续按旧池加新候选限制 100 项；C# 通用 D3D11 硬件工厂已使用 `BgraSupport | VideoSupport`，与 Rust WGC 视频处理能力声明对齐；WGC surface→DXGI texture 解包和实际 WPF GPU83 最终像素闭环已通过显式夹具；声音启动现在必须在有界预算内产生首批 PCM，最终 PCM 总线的 RTMP 分支不再反向阻塞本机声音；主窗口真实导入、运行包失败保留旧池、未授权导入和真实 EOF 自动换源已补齐边界证据；mpv IPC 断开时控制器不再误报播放中，停止失败会向上返回；RTMP 启动失败、Pump/Producer 失败和停止超时均保留真实失败状态与可重试资源；底部主导入按钮恢复可见，mpv `glsl-shader-opts` 字符串回读按固定键集合兼容；复审并撤回未形成闭环的 RTMP stdout 进度草稿，修正无音频停止时的 `_audioSerial` 释放条件；生成音频快照的正式频域字段已进入 FFmpeg `-af`，CPU4 动态更新已与启动滤镜使用固定标签，视频声音会话在首次输出失败后可随暂停/恢复从 mpv 位置有界重试；App 测试程序集已加入串行门禁，保护共享 WPF Dispatcher、环境变量和真实 mpv 窗口资源；`导入列表` 已接入版本化 JSON 本地路径列表，成功后复用既有 FFprobe 与 `ReplaceAll` 原子提交；导入探测与原子提交现在持有共享播放命令串行门，关闭最终效果窗口复用统一输出停止路径，RTMP 共享最终 PCM 总线按实际声道数构造解码、混音和分流链；C# 启动入口新增用户本地文件句柄单实例门禁，实测第二次启动立即退出且进程数保持为 1。以上只修改 C# 代码、测试和专项文档，未改变登录门禁、媒体原子提交、WARP 禁止或目标 GPU/下游发布门禁边界。
- 实施端：`desktop-csharp-windows`（C# / WPF / Windows）
- 只读参考端：`desktop`（Rust / Tauri）；本长任务禁止修改 Rust/Tauri 源码、依赖、锁文件、生成物和测试产物

- v1.76 本轮收口：媒体池导入忙碌态结束后，WPF 列表尚未完成绑定时的操作按钮会有界回退到当前媒体池索引，绑定完成后仍以真实列表选择为准；RTMP 声音启动必须在 3 秒有界预算内观察到至少一帧最终 PCM 已转发到 FFmpeg，泵失败或无首帧则 fail-closed 并执行有界停止。Windows 受影响全量测试 `226/226`、Release x64 构建 `0` 警告/`0` 错误、冷启动 A/B 进程数 `1` 通过；真实声卡、远端 ZLMediaKit/RTMPS、人工页面和长稳仍待验收。

- v1.77 本轮收口：C# 播放状态与 Rust `Ready/Paused/Playing/Stopped` 合同对齐，停止后不再把存活的 mpv 进程猜测为可恢复会话，必须重新进入既有启动/首帧门禁；音高微移与播放速度同时启用时，FFmpeg `-af` 顺序调整为先音高再速度，与 Rust 实时声音链一致。Core 状态测试 `1/1`、真实 WPF 导入/启动/EOF 用例 `2/2`、Media 全量 `132/132`、Windows 全量 `226/226`、格式检查通过、Release x64 构建 `0` 警告/`0` 错误。没有新增播放器、线程、队列或依赖；真实声卡、远端 ZLMediaKit/RTMPS、人工页面和长稳仍待验收。

## 1. 方案定位

本文是后续 C# ↔ Rust 桌面端功能同步工作的长任务基线。后续每一个同步任务都必须先回到本文更新功能状态、边界、依赖和验收门禁，再进入代码实施；不能只根据某个页面、历史计划或单次测试数量宣称已经同步。

本文解决的是“产品行为、输入输出、错误语义、资源生命周期和验收标准同步”，不是要求两端代码结构、UI 像素或底层语言完全相同。对于 C# Windows 客户端，Windows 特有能力允许由 C# 直接管理；已经在 Rust 中形成较完整媒体语义的能力，C# 应只读参考相同的外部媒体契约和测试夹具，不要为追求代码对称而重复实现一套行为。当前发布产品中 RTMP 与 AkVirtualCamera 的 Rust/Tauri 代码只作为行为参考；本长任务不迁移、不修改 Rust 执行所有权，也不在 Rust 侧补功能。

本长任务的硬边界：所有实施代码、C# 测试、C# 工具和专项文档只能落在 `desktop-csharp-windows/`，以及本长任务方案文档本身；`desktop/` 下任何源码、`Cargo.toml`、`Cargo.lock`、Rust 测试、生成物和临时产物都不得修改。需要 Rust 变更的差异只能记录为“待单独授权/待另行任务”，不能作为本任务的隐含前置动作。

冲突处理顺序：

1. 用户当前明确要求与安全边界。
2. 根目录 [`AGENTS.md`](../../AGENTS.md) 与当前产品文档。
3. 本方案。
4. 历史实施计划、历史测试记录和已废弃入口，仅作审计资料。

本方案不改变当前产品范围：不恢复实时话术幻化、speech-to-speech、ASR/LLM/TTS 实时链路、研究报告 Worker、永久 MP4/版本队列、OBS 或多平台自动发布。

### 1.1 并行实施批次与主任务审查边界

为加快 C# 黄金路径修复，后续独立开发按以下两个侧边栏任务并行推进；两个任务均直接使用当前工作区，不创建 Git worktree：

| 侧边栏任务 | 允许修改范围 | 只读参考 | 当前目标 | 主任务合并条件 |
|---|---|---|---|---|
| C#媒体黄金路径 | `desktop-csharp-windows` 的媒体导入、媒体池、`GpAutoLive.Media`、播放编排及对应测试 | `desktop` Rust 媒体契约与测试夹具 | 解决导入、媒体池、声音/视频效果未进入真实执行链的问题 | 先失败测试；无越界文件；目标测试通过；不把 IPC/进程存活冒充最终效果 |
| C#WGC硬件链 | `desktop-csharp-windows` 的 WGC、D3D11、`WindowsVirtualCamera` 及对应测试 | `desktop/src-tauri/crates/autolive-virtual-camera-native` | 对齐 HWND 捕获、硬件设备能力、固定 YUY2 输出和生命周期边界 | 先失败测试；无越界文件；设备/资源释放有界；实机门禁如未具备则明确保留待验收 |

主任务在子任务返回后统一检查 `git diff`、项目作用域、测试结果和依赖变化；发现跨边界或无法证明真实效果的改动时，只保留证据，不把它计入同步完成百分比。子任务不得修改 `desktop/` 下任何 Rust 源码、依赖、锁文件、生成物或测试产物，也不得为了通过作用域检查回滚或删除用户已有变更。

## 2. 当前基线与差异结论

当前 C# 最新基线以 [`CSharp-Windows当前状态.md`](../../../desktop-csharp-windows/docs/CSharp-Windows当前状态.md) 为准：上一轮串行全量自动化测试为 498/498；当前最新受影响分组已复验 Windows `223/223`、Media `131/131`，App 稳定分组 `60/60`，App 全量 `79/79`，其中 App 黄金路径 `VideoPlaybackAudioFallbackTests` 为 `6/6`，媒体拖放边界 `7/7`，并以 `E:\下载\csharp-golden-av.mp4` 完成真实 FFprobe 导入、mpv Original/CPU4/GPU83 基线、完整 GPU83 83 项映射/资源/运行时更新、PortAudio 输出和有限音频 N/N+1 单一输出切换显式夹具；CPU4 启动/更新会逐项回读四个滤镜值，GPU83 启动/更新会逐项回读当前快照中的固定白名单 shader 参数，启动播放还会在控制器标记 `Playing` 前确认 `estimated-frame-number > 0`，可听时钟已具备 PortAudio 帧/延迟计算和 WPF 同身份进度投影的代码与单元证据，真实同设备有界重开恢复也已通过；视频声音中途切换已能按 mpv 当前位置重启 FFmpeg/PortAudio，真实下载夹具已证明偏移解码产生最终 PCM；生成音频快照的频域扰动、频谱盲区和高频扰动字段已转换为正式音频参数并生成受限 `-af` 链，视频恢复前会按 `time-pos` 尝试重新建立声音会话。视频周期快照现在同时生成并传递 Rust 对齐的正式 `VideoEffectParams` 与 `AdvancedEffectParams`，完整 GPU83 不再固定使用高级默认值；旧资源包 CPU4 仍只允许四项滤镜，视频处理关闭时统一选择 Original。WPF 现在在视频处理开启且完整 shader 哈希与当前 C# 资源一致时选择 GPU83，旧基线和缺失完整资源时保持 CPU4；FFprobe 对封面图视频流和无效平均帧率的导入边界已与 Rust 对齐。主窗口真实导入入口已通过 `E:\下载\csharp-golden-av.mp4` 的 FFprobe→原子入池→首项选中→mpv 播放夹具；GPU83 与 CPU4 均通过实际 `FinalEffectWindow` 最终视频表面像素哈希变化夹具；普通 WPF 与实际 `FinalEffectWindow` 顶层窗口的 WGC 帧池启动/停止隔离测试为 2/2；媒体池的实际编辑控件已恢复可见，顶栏与底部导入入口已纳入同一忙碌态投影，插话目录扫描也与两个媒体导入入口保持互斥；拖放预览现在对空路径、目录形态和不支持扩展名 fail-closed，不触碰文件 I/O；这不改变登录/设备授权门禁。开发脚本默认优先使用已校验的 `csharp-gpu83-real-v2`，缺失时才回退 v90；显式传入完整运行包仍可用于复验，避免把历史基线误当成完整能力。目标显卡矩阵、AkVirtualCamera 下游、真实声卡/RTMP、签名安装、长稳和人工页面验收仍未通过。因此“自动化通过”与“功能同步完成”不能等同。

本段历史基线中的“串行全量”以此前一次完整运行记录为准；v1.34 最新复核采用稳定分组口径：非 App 项目 `443/443`，App `53/53+1/1+1/1`，合计 `498/498`，因为两个真实视频窗口用例连续复用同一 WPF 测试宿主时存在收尾挂起风险。

Rust 的当前产品入口以 [`desktop/src-tauri/src/main.rs`](../../../desktop/src-tauri/src/main.rs) 注册的命令和当前产品文档为准。历史 `VariantTask`、研究 Worker、实时话术命令即使仍在源码中用于迁移审计，也不是同步目标。

| 能力 | Rust 当前基线 | C# 当前基线 | 同步判断 | 优先级 |
|---|---|---|---|---|
| 登录、设备、激活、心跳 | 已有控制面入口和桌面命令 | 已有 WPF/Windows 侧模型与流程 | 契约、刷新、离线恢复和错误语义需要共同门禁 | P0 |
| 本地媒体池 | 最多 100 项、探测后原子提交、顺序循环 | 已有池模型和 Windows 侧验证基础 | 统一探测规则、取消、重复路径、原子替换和 UI 状态 | P0 |
| 单窗口播放 | 单一 mpv 会话、活动源、暂停/恢复/换源 | 已有播放控制器和单窗口约束 | 补齐 EOF、停止、换源和退出时资源回收的共同验收 | P0 |
| 视频效果 | `gpu-next/libplacebo` GPU83，失败单向降级 CPU4，再到 Original；部分参数已有真实运行链 | C# 已以真实可验收的 mpv CPU4 `vf set` 链承载四项参数，并已接入 83 项契约映射、完整受限 shader 资源、调度输入和 manifest/SHA-256 校验；WPF 在完整 shader 哈希匹配时使用 GPU83，旧包保持 CPU4；启动和播放态更新都已接入有界 `estimated-frame-number` 首帧/下一帧观察；原生 Win32 与实际 WPF `FinalEffectWindow` 的 GPU83/CPU4 最终像素均已通过显式夹具 | 保持目标显卡矩阵、完整算法能力、单向降级和人工页面证据；不可把 IPC 回读或帧号前进单独当作视觉效果验收 | P0 |
| 普通声音 | 独立声音候选、PortAudio、N/N+1 和可听时钟边界 | C# 视频/音频播放均接入 FFmpeg PCM→PortAudio；处理开关把增益/EQ/倍速/内置滤镜音高微移/淡入/自然动态响度包络/确定性本地音色 EQ/混响/降噪/相位/颤音写入 `-af`，低/中/高 EQ 中心频率与 Rust realtime 链统一为 `200/1000/8000 Hz`，最终 PCM 可分流给 RTMP，有限音频项已接入一个预载候选和单输出边界切换，PortAudio `timeInfo` 已形成可听帧/延迟快照并用于同身份视频进度投影；动态范围、压缩没有 C# 正式对应参数，继续标记为未接入 | 补齐真实声卡时钟稳定性、过载重建、设备故障回退和 RTMP/声卡联合门禁 | P0 |
| 插话文件/固定话术 | 按当前范围执行，实时话术除外 | C# 有插话/`SAPI.SpVoice` 边界，但 SAPI 尚未进入最终 PCM 总线 | 对齐优先级、抢占、静音、取消、恢复；固定话术保持无模型 | P0 |
| 麦克风插话 | 已有正式需求和 Rust 侧边界，真实声学门禁未完成 | Windows 音频接入和状态边界可由 C# 管理，真实 AEC/降噪/AGC/VAD 待门禁 | 两端均标记待验收；VAD 仅作说话活动门控，不扩展为识别或变声 | P1 |
| RTMP/RTMPS 直推 | 受管 FFmpeg、GPU 编码探测、最终 PCM 分流、有限重试已有运行基础；UI 状态仍有旧标签 | 有 FFmpeg 主机、状态和重连协调器，但启动/断开/远端握手/联合轨道证据不完整 | 统一 3 轨道、重试、取消、脱敏、断开和恢复语义；不捕获桌面、不经过 Go | P0 |
| AkVirtualCamera | WGC/D3D11、固定 YUY2 720p30、独立 GPL sidecar 边界已有部分接入 | 有 WGC/D3D11/Vortice 和安装脚本基础，真实设备/sidecar/签名门禁未完成 | 可以由 C# 直接管理 Windows 采集与安装外壳，但必须保持固定协议和一次有界回读 | P1 |
| 抖音 M1 | 当前 Rust 只有扫码/探针入口，尚未形成完整 manager + 真实 sidecar 发送链 | C# Core 已有二维码状态、去重、随机回复、有界队列和 60 秒过期逻辑，但事件桥未接入真实消息字段 | C# Core 可作为队列语义实现候选；协议、许可、扫码、发送、自回显和风险停止必须按专项方案实测 | P1 |
| 本地运行时资源 | Rust 有受限资源状态/安装/导入/清理命令 | C# 有 WPF 安装、回滚、卸载、签名探针 | 保持平台差异；统一 manifest、版本、路径白名单、失败关闭和退出回收 | P1 |
| 缩略图、偏好设置、Windows 诊断 | Rust UI 没有同等 WPF 缩略图缓存；偏好和诊断模型不同 | C# 有有界 FFmpeg 缩略图缓存、INI 偏好、GDI/User/GC 采样 | 不作为强制功能镜像；只有产品行为需要时才补 Rust 对应能力 | P2 |
| 单实例/媒体输出所有权 | Rust 保留 Tauri 单实例，并在 Builder 前获取共享媒体 Mutex | C# 使用同名媒体 Mutex，进程探测仅作诊断 | 共享锁代码已接入；真实跨进程争用和资源矩阵待验收 | P0 |

### 2.1 已确认的关键差距

1. C# 的旧 `VideoEffectEditorState` 仍只映射有限字段；当前 WPF v2 系统生成快照已先转换为正式 `VideoEffectParams`，在完整 shader 哈希匹配时进入 GPU83、旧包进入 CPU4，启动已形成“首帧号有效”门禁，播放态更新已形成“参数回读 → 下一帧号前进”的运行时门禁，实际 WPF `FinalEffectWindow` GPU83/CPU4 最终像素回显也已通过显式夹具；剩余是目标显卡矩阵、完整算法能力和发布门禁。
2. C# 已将本地音频生产的 `FinalPcmBus` 共享给 RTMP 声音出口；有限音频项已连接一个 N+1 预载解码器、四分支总线切换器和单一 PortAudio/RTMP 输出边界，PortAudio `timeInfo` 已接入可听帧/延迟快照和同身份视频进度投影，主轨和候选在正常容量范围内按消费者水位背压，并以 `E:\下载\csharp-golden-av.mp4` 完成一次真实夹具切换测试。真实设备时钟稳定性、过载重建、设备故障回退和真实 RTMP 仍需单独门禁，不能把该测试等同于完整声音同步。
3. C# RTMP 有重连协调器，但底层进程退出、无进展、远端失败、联合音画轨道和最终 PCM 的真实连接仍需按 Rust 语义补齐；Rust UI 中“待实施/未接入”标签需要按真实代码和门禁重新整理。
4. 两端此前的媒体所有权协议不是同一个协议；本轮已新增 Rust 共享 Mutex 封装并在 Builder 前获取，当前剩余问题是真实跨进程争用和资源矩阵证据。
5. C# 的抖音队列核心比 Rust 探针更接近 M1 目标，但当前 sidecar 事件桥只映射粗粒度状态，没有把真实 `WebcastChatMessage` 载荷送入 `ObserveChatMessage`，所以不能宣称 C# 已完成抖音链路。

### 2.2 2026-09-04 历史实施增量

- C# `FfprobeSourceParser` 已与 Rust 的媒体分类边界对齐：音频容器中的 `attached_pic` 封面图不再被误判为视频；视频 `avg_frame_rate` 无效时回退 `r_frame_rate`，避免合法媒体因 `0/0` 帧率字段整批导入失败；两个边界均有 C# 失败先行测试。
- C# `WindowsMpvPlaybackController` 启动 mpv 后，在保持暂停的窗口内统一调用 `MpvPlaybackSession.UpdateEffects` 应用初始 `Original/Gpu83/Cpu4` 快照；CPU4 滤镜不再由 `MpvLaunchPlan` 静态参数和 IPC 动态命令重复安装。
- C# 新增 `UpdateEffectsAsync` 控制器入口，换源时保留并重新应用原效果快照，避免源切换后会话状态与 mpv 滤镜状态分叉。
- C# `WindowsAudioPlaybackController.ActiveFinalPcmBus` 与 `WindowsRtmpAudioSession` 已形成非拥有共享模式：本地音频会话作为最终 PCM 生产者，RTMP 只消费同一总线的稳定 RTMP 分支；停止播放/自然完成先回收 RTMP，未有本地音频总线时保留原独立 RTMP 解码路径。有限音频项已增加一个候选预载解码器、候选容量等待、EOF 提交和单一输出源切换；当前项与下一项之间不重启 PortAudio，RTMP 分支在已连接时参与同一切换边界。
- C# 有限音频项已补齐真实 N/N+1 生命周期：当前项进入独立最终 PCM 总线，下一项最多一个候选进入另一条有界总线；候选就绪后由 WPF 在当前项 EOF 边界提交，`FinalPcmBusTrackSwitch` 同步管理本机、RTMP、插话本机和插话 RTMP 四个分支，PortAudio 不重启，RTMP 有真实消费者时等待其分支排空后再切换。候选解码移除 `-re` 并受有界背压约束，主轨也会在活动消费者达到水位时暂停生产，取消/超时会清理候选并回退到原有停止后重启路径。已通过纯逻辑切换测试和 `E:\下载\csharp-golden-av.mp4` 真实 PortAudio N/N+1 夹具测试；可听时钟已补齐代码和单元证据，但设备拔插/重建、过载重建、真实稳定延迟、RTMP 远端和带插话跨边界仍待验收。
- WPF“视频处理”开关已在活动视频会话下调用 `UpdateEffectsAsync`，按资源门禁提交 `Original/CPU4/GPU83` 快照；播放态更新现在在参数回读后增加 `estimated-frame-number` 下一帧观察，启动也在标记播放前确认首帧号有效，无活动会话或暂停态不伪造运行时结果。GPU83 最终像素、完整渲染能力和真实发布门禁仍待验收。
- 2026-09-04 WGC 最终表面复验：按既有 C# WPF → 预留视频子 HWND → `Windows.Graphics.Capture` → D3D11→YUY2 链复跑 CPU4 与 GPU83 像素夹具；两条路径均在 `CreateForWindow` 阶段返回 `ItemUnavailable`，随后已停止并清理捕获会话，未把进程存活、mpv 首帧号或 IPC 参数回读当作像素通过。该结果将 WGC/预留 HWND 环境门禁保留为阻塞项，待可捕获的本机窗口环境重新验收。
- 2026-09-04 WGC 会话生命周期修复：C# `WindowsGraphicsCaptureWindowSession` 为 `CreateFreeThreaded` frame pool 注册 `FrameArrived`，事件只负责唤醒专用捕获线程；捕获线程集中调用 `TryGetNextFrame`、执行同设备 D3D11 转换并按停止信号退出，停止阶段取消订阅并保护事件回调与信号释放竞态。该修复补齐了“会话 Running 但仅靠自建轮询拿不到帧”的实现缺口；当前子 HWND 仍返回 `ItemUnavailable`，未把代码接入或进程存活计为 WGC 像素通过。
- 2026-09-04 WGC WinRT 工厂调用修复：C# 对 `GraphicsCaptureItem` 改用 `Windows.Graphics.Capture.GraphicsCaptureItem` activation factory 的 `IGraphicsCaptureItemInterop.CreateForWindow`，严格使用窗口句柄、目标接口 IID 和 `ref` ABI 签名，并在创建成功后释放原始 COM 指针；高版本 `TryCreateFromWindowId` 仍作为优先路径。该修复消除 C# 自定义 interop 与 WinRT ABI 不一致的代码风险，但同机预留子 HWND 的像素夹具仍返回 `ItemUnavailable`，未计为 WGC 像素通过。
- C# mpv 控制器新增固定 `vf` 属性回读入口；CPU4 真实夹具在提交初始快照后能从 mpv 回读包含 `@autolive_cpu4` 的活动滤镜链，并在 `UpdateEffectsAsync` 后回读新的亮度、对比度和饱和度值，形成“提交 → 运行时链存在 → 运行时参数变化”的有效回显。该回读不等于 GPU83 shader 参数回显或视觉帧级验收。
- 该增量只证明“受限效果快照可以进入 C# IPC 调度边界”，不证明 GPU83 完整 shader、WPF 只读周期结果、下一帧有效回显、真实渲染参数或跨设备输出已完成；这些仍保持 `代码已接入·待验收` 或 `正式需求·待实现/未接入`。
- C# GPU83 基线 shader 资源接线已完成：由 C# 项目维护受限、可审计的外置 shader 资源，并把资源纳入媒体 manifest 的名称、路径、大小和 SHA-256 校验；`MpvLaunchPlan` 仅在 `Gpu83` 模式绑定已验证的 `--glsl-shaders` 路径。该基线只承载当前 C# 可提交的亮度、对比度、饱和度和色相四项，不把 Rust 端完整 83 项参数或 GPU 矩阵门禁提前标记为完成。
- C# GPU83 完整契约接线已完成代码段：`MpvGpu83ShaderSnapshot` 固定维护 83 项字段、shader 选项、执行类别和能力类别；静态像素/合成字段进入受限 `glsl-shader-opts`，PTS 调度字段折叠为同一原子属性中的运行时输入，色域转换、切片最小长度和需要历史纹理的字段保持显式不可用。C# 自有 `Resources/gpu83.hook` 已纳入 `stage-media-runtime.ps1` 的资源清单；以 `E:\下载\csharp-golden-av.mp4` 的独立 C# 包完成完整 GPU83 启动、更新及静态/调度参数回读 1/1。该证据仍不等同于下一帧像素、目标 GPU 矩阵、WPF 默认路径或完整算法验收。
- C# GPU83 基线参数回读已补齐：`MpvVideoEffectSnapshot.TryCreateGpu83BaselineColor` 只生成四个固定 shader 参数键，`WindowsMpvPlaybackController` 的 GPU83 启动和更新都会回读并逐项匹配 `glsl-shader-opts`，不再把 IPC 写入成功冒充 shader 已生效；以 `E:\下载\csharp-golden-av.mp4` 启动真实 mpv 后，初始和更新回读均通过。旧 v90 资源包的 WPF 播放保持 CPU4，完整 C# v2 资源包按完整 shader 哈希选择 GPU83；GPU83 完整 83 项、下一帧像素回显和自动降级策略仍未完成。
- C# GPU83 基线参数回读已补齐：`MpvVideoEffectSnapshot.TryCreateGpu83BaselineColor` 只生成四个固定 shader 参数键，`WindowsMpvPlaybackController` 的 GPU83 启动和更新都会回读并逐项匹配 `glsl-shader-opts`，不再把 IPC 写入成功冒充 shader 已生效；以 `E:\下载\csharp-golden-av.mp4` 启动真实 mpv 后，初始和更新回读均通过。WPF 仅在已验证资源的 shader SHA-256 等于当前 C# 完整资源哈希时选择完整 GPU83，旧基线继续使用 CPU4；GPU83 下一帧像素回显和自动降级策略仍未完成。
- C# CPU4 参数回读已收紧：`MpvVideoEffectSnapshot.MatchesCpu4Readback` 对 `@autolive_cpu4` 活动链中的亮度、对比度、饱和度和色相逐项匹配；启动和更新均使用该检查，避免只看到滤镜标签却误报参数已经变更。真实 `E:\下载\csharp-golden-av.mp4` CPU4 夹具通过。
- 该音频增量已证明共享总线、候选预载、单一输出切换、正常容量背压和可听时钟投影进入 C# 编排，并通过一次真实 PortAudio 夹具切换；不证明真实声卡稳定时钟、设备恢复、过载重建或 ZLMediaKit/RTMPS 远端输出已完成。
- 历史基线曾记录 C# 串行全量测试 `459/459`；随后黄金路径修复基线为 `483/483`，补充 FFprobe 兼容边界后为 `485/485`；本轮新增视频启动模式选择、视频声音当前位置恢复、无 PortAudio 输出仍保持视频播放、主窗口真实导入/播放的 WPF 夹具和 CPU4 最终表面像素哈希夹具后，使用项目锁定 SDK 验证为 Contracts `28/28`、Installer `15/15`、Core `76/76`、Media `113/113`、Windows `209/209`、App `53/53`，合计 `494/494`，`dotnet format --verify-no-changes` 和 App Release 构建均通过。GPU83 像素有效回显、显卡矩阵、WPF 最终视频表面首帧、真实声卡稳定时钟、拔插/睡眠故障恢复、过载重建、真实声卡长稳和远端输出仍待验收；mpv 启动首帧和播放态下一帧号观察已通过真实 CPU4/GPU83 夹具。`tools/verify-scope.ps1` 当前按设计因工作区已有 `desktop/` Rust/Tauri 变化而 fail-closed；这不是本轮 C# 修改 Rust 的结果。Rust 测试不属于本长任务验收项，本轮未执行。

### 2.3 2026-09-04 C# 黄金路径修复

- 本轮严格只新增/修改 `desktop-csharp-windows` 内的 C#、WPF、C# 测试和 C# 专项文档；Rust 代码不作为本轮实现范围，Rust 只读用于理解行为契约。
- C# 作用域门禁 `tools/verify-scope.ps1` 已修正为同时检查 `desktop/` 下已跟踪和未跟踪路径；当前工作区已有 Rust/Tauri 源码、共享锁 crate 与 `target-douyin-test` 产物，因此门禁按约束返回失败。本轮不回滚、不删除这些 Rust 变化，也不把它们计入 C# 实施结果。
- `MediaRuntimeBoundary` 支持显式的 `AUTOLIVE_MEDIA_RUNTIME_ROOT` 安装根目录，仍由既有 `manifest.json`、路径、资源大小和 SHA-256 全量校验；未配置或不合法时继续回到应用目录并 fail-closed。这样开发运行目录可以使用已验收的外置资源包，不把 FFmpeg/mpv 复制进源码或绕过清单。
- WPF 媒体列表选择现在会提交 `MediaPoolService.SelectAt`；导入成功也统一经过同一媒体操作投影，保证首项选中、缩略图和上移/下移/移除按钮同步；导入先完成整批 FFprobe，全部通过后才在原子提交前停止输出并提交池，探测失败不会打断旧播放；播放中拒绝直接改写当前源并恢复列表选择，要求使用上一项/下一项，以防 UI 选中项与活动输出身份脱节。
- 视频启动不再只启动无音频的 mpv：有音轨的视频会同时建立现有 FFmpeg PCM、PortAudio 和 `FinalPcmBus` 会话；视频暂停/恢复同步暂停/恢复声音，视频自然结束时先回收旧声音再切换下一项。无音轨视频仍允许只播放画面。
- 视频启动的画面与声音失败语义已与独立输出边界一致：mpv 画面成功后，PortAudio 设备未选择、资源缺失或声音流启动失败只标记“声音输出不可用”，不关闭 mpv、不把媒体池回滚为 `Ready`；视频状态观察仍由 mpv 负责，后续重新播放或设备恢复时再建立声音会话。纯音频媒体继续在声音输出不可用时 fail-closed。
- 旧 v90 资源包的视频处理使用既有 mpv CPU4 路径作为可验收执行路径；完整 C# v2 资源包在 shader 哈希匹配时使用 GPU83，否则单向回退 CPU4。`MpvIpcCommand` 改为通过 `vf set` 一次提交带固定标签的完整 `lavfi` 链；不再用当前 mpv 会拒绝的内层 `vf-command` 增量命令。GPU83 完整 shader、帧级有效回显和 GPU 能力矩阵仍不标记为已生效。
- 普通声音处理接入 FFmpeg `-af`，当前正式接入的实时子集是输入/输出/响度增益、低/中/高三段均衡、倍速、内置滤镜音高微移、淡入、已知源时长的淡出、混响、降噪、相位扰动和颤音；低/中/高 EQ 中心频率已按 Rust realtime 链统一为 `200/1000/8000 Hz`。未知时长不伪造淡出位置，需要整轮反向缓冲的复杂变换与其他需要专用 DSP/第二输入的字段仍按实际状态标记待接入。处理开关关闭时不附加滤镜，纯音频切换开关会停止并重新建立同一有界会话；视频处理中途只重建声音会话，按 mpv 当前 `time-pos` 传入有界 FFmpeg `-ss`，不重启视频画面。
- WPF 开关接线已按真实交互收敛：视频处理由 `ShellState.VideoProcessing` 属性变化统一进入活动 mpv 的 `UpdateEffectsAsync`；声音处理保留单一 `Click` 入口，视频媒体在播放/暂停中切换时读取当前播放位置、只停止并重建声音 worker，暂停态重新保持暂停；位置不可读或恢复失败时 fail-closed，保留视频并显示错误。
- 系统生成音频快照的低频/中频/高频字段已按正式契约使用 `LowEqDb`/`MidEqDb`/`HighEqDb` 并以 dB 传入 FFmpeg，避免把实际 EQ 增益误标为百分比或遗漏中频。
- 系统生成音频快照现在额外生成范围受限的 `PitchShiftSemitones`，WPF 播放参数映射会把该值传入现有 FFmpeg 内置滤镜音高链；这只补齐已实现子集的真实消费，不把未接入的复杂 DSP 字段伪装成已生效。
- “重新生成本周期参数”会在纯音频活动会话或视频 mpv 活动会话中提交当前可用的效果快照；带音轨视频会复用当前位置声音重建入口，让新一轮可接入音频子集真正消费到当前会话，不重启视频画面。
- C# 开发启动脚本现在显式注入外置媒体运行时根目录；带音轨媒体启动时若 UI 尚未选择输出设备，会在同一受管资源边界内懒加载 PortAudio 设备，优先使用默认输出，驱动未返回默认索引时选择首个已验证输出设备。视频 CPU4 参数在播放和暂停状态都允许提交，纯音频声音开关在暂停状态重启后保持暂停。
- C# WPF 系统声音快照中的混响/降噪状态现在会映射到 `AudioEffectParams`，并由上述受限实时 `-af` 子集实际消费；动态范围/压缩/音色仍不单独扩展参数模型，避免把 UI 文案冒充成 Rust 端已有契约或用近似滤镜伪造生效。
- C# 视频快照路径文案改为“按资源选择”，明确完整 shader 哈希匹配时使用 GPU83、旧/不完整资源时单向回退 CPU4、关闭时为 Original；中央声音快照补齐中频 dB 卡片。
- 纯音频项自然结束后，若播放池下一项为视频，C# 会继续保持 `Playing` 并重新建立 mpv 视频表面及视频声音会话；不再因音频→视频类型切换错误地停止整个混合媒体池。
- Windows 音频控制器的重复 `StartAsync`、插话前置条件拒绝不再污染正在运行的主会话为 `Failed`；原有无活动会话的 `Pause/Resume` fail-closed 契约保持不变。
- 历史校验结果（N/N+1 与可听时钟补充前）：C# Contracts `28/28`、Installer `15/15`、Core `76/76`、Media `105/105`、Windows `203/203`、App `45/45`，WPF Release build `0 warning / 0 error`；使用 `E:\下载\csharp-golden-av.mp4` 的真实 FFprobe 导入 `1/1`、PortAudio 解码/输出 `1/1`、外置 mpv Original `2/2`、CPU4 `2/2` 夹具（CPU4 含初始与更新后的 `vf` 活动链回读）和 GPU83 基线 `2/2`（含四项 `glsl-shader-opts` JSON 对象字段回读）以及实时声音滤镜链到最终 PCM/PortAudio `1/1` 均通过；已知时长淡出和内置滤镜音高微移链的原始/处理 PCM SHA-256 均不同；媒体导入“先整批探测、后停止并提交”的顺序测试 `1/1` 通过；系统生成声音快照的音高范围测试 `7/7` 通过。GPU83 完整 83 项参数、下一帧有效回显、真实 WPF 登录、设备选择、首帧、带音轨视频端到端点击、RTMP/虚拟摄像头/抖音和发布门禁仍待实机验收。
- 本轮 WPF 真实点击补充：使用显式夹具完成导入 `1` 项、播放、暂停、关闭视频处理并看到“已提交给 mpv”、重新开启并看到“已提交给 mpv CPU4 实时滤镜”；视频声音切换的受管重建和当前位置 FFmpeg 夹具已通过。测试通过的是 C# WPF 状态提交、受管进程运行和声音偏移 PCM 输出，不宣称效果画面或可听频谱已通过。
- C# WPF 完整视频参数接线补充：系统生成快照现在先转换为正式 `VideoEffectParams`，再在完整 shader 哈希匹配的受管资源上创建 GPU83 快照；`锐度`进入正式 shader 参数，当前没有契约对应关系的 Gamma/曝光/降噪文案仍不伪造为已生效。旧资源包仍按能力门禁走 CPU4；完整 GPU83 的参数回读已有独立夹具，但下一帧/像素和显卡矩阵仍待验收。
- C# 视频启动模式修复：视频处理关闭时无论资源能力如何都启动 Original；开启时按完整 shader 哈希选择 GPU83，否则选择 CPU4。启动快照不再强制覆盖为默认 GPU83 baseline，而是与运行时更新复用当前已校验的系统生成参数，避免首次播放看起来没有视频变换。
- C# FFprobe 兼容性补充：按 Rust 只读对照修正封面图视频流分类和无效平均帧率回退；C# 媒体池导入的合法音频容器与可变帧率视频边界已由两个单元测试锁定，未改变播放池上限、原子提交或授权门禁。
- C# WPF 媒体池入口补充：真实的上移、下移、移除和清空控件此前位于零高度隐藏行，本轮恢复其可见布局；顶栏添加媒体按钮增加自动化标识，并与既有导入入口共享导入忙碌态。该修复只恢复已有 C# 媒体池能力的操作入口，不放宽登录/设备授权，不改变整批探测失败保留旧池的语义。
- C# 主窗口真实导入/播放夹具补充：使用 `E:\下载\csharp-golden-av.mp4` 通过 `RunImportAsync` 完成运行时清单校验、FFprobe、原子入池、列表首项投影和 mpv 播放；无效 PortAudio 设备时仍保持视频 `Playing`。插话目录扫描改为复用同一媒体导入入口忙碌态，避免顶栏入口与旧入口在扫描期间并发。

## 3. 目标架构与职责边界

### 3.1 统一的产品契约

两端共同遵循以下契约，契约可以用 JSON fixture、枚举、错误码和行为测试表达，但不强行共享语言代码：

- `MediaEffectParams`、`AudioEffectParams`、`VideoEffectParams`、`AdvancedEffectParams` 及媒体参数范围文档。
- 播放池、活动源、循环/EOF、播放状态、取消和资源生命周期。
- 视频处理模式 `Gpu83 → Cpu4 → Original` 的单向降级与有效参数回显。
- 音频候选 N/N+1、最终 PCM 总线、可听时钟、插话优先级和故障恢复。
- RTMP/RTMPS 三种轨道组合、编码器选择、重试退避、状态脱敏和取消。
- AkVirtualCamera 固定 `GpAutoLive Camera`、YUY2、`1280×720@30fps`、WGC/D3D11、`zero_copy=false` 和 sidecar IPC。
- 抖音 M1 的扫码登录、单直播间、`WebcastChatMessage`、本地 1～100 条回复池、有界串行队列、满时丢最旧、60 秒过期、自回显过滤和风险停止。
- 运行时资源 manifest、SHA-256、路径白名单、许可证、安装/卸载/回滚和 fail-closed 规则。

### 3.2 实现所有权

本长任务只实施 C# Windows 适配层；Rust/Tauri 下面的实现仅用于只读确认契约，不在本任务中变更：

```text
WPF/C# App
  ├─ Windows UI、设置、控制面、设备/权限、安装、诊断、进程/IPC 生命周期
  ├─ Windows 音频设备、SAPI、WGC、D3D11、虚拟摄像头和 RTMP 外壳
  └─ 通过统一契约调用本地媒体引擎和 sidecar

Rust/Tauri App
  ├─ 保持现有跨模块媒体语义、mpv/libplacebo 运行链和 Rust 资源所有权
  ├─ 保持已有 PortAudio/音频候选/播放池行为基线
  └─ 通过同一契约提供等价的桌面产品行为
```

这不是把所有功能永久分成“C# 版”和“Rust 版”两套事实源。一个能力只能有一个行为契约和一个可核验的状态定义；实现可以因平台而不同。C# 可以直接管理 Windows 外壳，但不得绕过 Rust 已确认的媒体语义、错误分类和安全门禁。若要把当前产品的 RTMP 或 AkVirtualCamera 执行所有权从 Rust/Tauri 改到 C#，必须先单独修改根目录产品架构文档并重新通过发布门禁。

### 3.3 统一状态标签

所有 UI、文档和验收报告只使用以下标签：

- `正式需求·待实现/未接入`：只有需求、模型、默认值或校验，尚未进入真实输出。
- `代码已接入·待验收`：代码路径存在，但缺真实设备、外部服务、长稳或发布门禁。
- `自动化已通过`：指定自动化测试通过，不代表真实业务完成。
- `真实环境已通过`：真实设备/服务/干净安装/长稳证据通过，且证据可复核。
- `非当前范围`：当前版本明确不实施的能力。

## 4. C# 可以直接管理的能力

“C# 更好”仅指 Windows 桌面场景下的成熟度、系统集成和维护成本更优，不表示 C# 在所有媒体算法上优于 Rust。以下是 C# 客户端可以直接管理的能力，并通过契约与 Rust 对齐；其中 RTMP/AkVirtualCamera 的 C# 条目表示“C# 客户端的可行实现边界”，不改变当前发布产品仍由 Rust/Tauri 执行的架构约束：

| 能力 | C# 直接管理方式 | 为什么适合 C# | 必须遵守的 Rust/产品边界 |
|---|---|---|---|
| WPF 操作界面与窗口 | WPF、现有 App 分区、命令和 ViewModel | Windows 原生窗口、输入法、DPI、焦点、托盘和安装引导更直接 | 对齐状态、错误和黄金路径，不复制 Rust UI 像素 |
| 控制面 HTTP 与本地认证 | `HttpClient`、`System.Text.Json`、现有 DTO；Credential Manager/DPAPI 保存安全引用 | .NET BCL 已覆盖取消、超时、连接池、JSON 和 Windows 安全存储 | 不保存明文密钥；API 契约仍以 OpenAPI/服务端为准 |
| 本地配置与迁移 | 现有 `UserPreferences`/INI 原子存储、不可变快照 | WPF 窗口与 Windows 用户配置集成成本低 | 只存非敏感偏好；敏感凭据使用安全存储且不进入日志 |
| 播放池和本地状态 | C# Core 的有界池、状态机和原子提交 | 与 WPF 文件选择、拖放、取消和错误展示直接连接 | 最多 100 项、全部探测成功后整批提交，不能创建版本队列 |
| mpv/FFmpeg/FFprobe 外壳 | `System.Diagnostics.Process`、受限 JSON IPC、Windows Job Object、超时/取消/Join | BCL 对子进程、句柄和 Windows 退出树管理成熟；不需要嵌入解码器 | 复用同一 mpv/FFmpeg 参数、错误类别和 GPU/CPU 回退语义 |
| 音频输出外壳 | PortAudio 既有契约、C# PCM 总线、受控线程/取消 | 可直接接 Windows 设备枚举、权限和设备变更；避免再造音频后端 | 只保留一个最终 PCM 总线，N/N+1 必须真正可听并可供 RTMP 分流 |
| 固定话术系统朗读 | 当前 `SAPI.SpVoice` COM STA 适配器 | Windows SAPI 无需新增 NuGet，设备与语音安装集成自然 | 仅固定话术；必须纳入优先级、静音、取消和恢复；不调用模型 |
| WGC/D3D11 捕获与 GPU 转换 | `Windows.Graphics.Capture` + 现有 `Vortice.Direct3D11`/`Vortice.D3DCompiler` | C# 对 WinRT/COM/窗口句柄和 Direct3D11 的 Windows 集成直接 | 不做桌面捕获、GDI 截图、WARP 冒充 GPU 或 CPU 重做 GPU83 |
| RTMP/RTMPS 进程管理 | C# 直接托管受管 FFmpeg、重试协调、状态脱敏和停止 | Windows 进程、Job Object、网络错误和 UI 状态组合更易维护 | 三轨道、最终 PCM、5 次退避 `1/2/4/8/15s`、取消和断开语义保持一致 |
| AkVirtualCamera 安装外壳 | C# WPF 安装/修复/卸载、Authenticode/WinVerifyTrust、路径白名单 | 安装器、注册、签名、UAC 和 Windows 诊断是 C# 的强项 | GPL sidecar 必须独立、固定版本和许可证可审计；捕获帧仍由 WGC/D3D11 产生 |
| 抖音 M1 队列与状态 | 现有 `DouyinLiveManager` 管理去重、随机回复、队列、TTL 和暂停/停止 | C# Core 已有较完整的纯逻辑实现，适合先做契约测试 | sidecar 负责协议；真实事件字段必须接入；凭据只在内存，单房间、无模型、无 Go |
| Windows 性能与资源诊断 | GDI/User/GC、进程、句柄、窗口和设备状态采样 | Windows 指标覆盖面比跨平台抽象更完整 | 诊断不能泄露密钥/正文；指标不能替代真实功能验收 |

## 5. 库与技术选型决策

### 5.1 选用或继续使用

| 领域 | 决策 | 说明 |
|---|---|---|
| UI | WPF + .NET BCL | 已是 C# 客户端基线；不引入第二套桌面 UI 框架。 |
| HTTP/JSON/取消 | `HttpClient`、`System.Text.Json`、`CancellationToken`、`Immutable` 类型 | 足够覆盖控制面、sidecar NDJSON、状态快照和取消；优先复用现有代码。 |
| 队列/背压 | 仅在真实生产者/消费者边界使用有界 `Channel` 或现有 Core 队列 | 不使用无界队列，不为了“以后扩展”加消息框架。 |
| GPU/窗口捕获 | `Windows.Graphics.Capture` + 当前 `Vortice.Direct3D11`/`Vortice.D3DCompiler` 3.8.3 | 现有项目已使用，适合直接 COM/D3D11；先补真实门禁，不换库。 |
| 媒体 | 外部受管 mpv、FFmpeg、FFprobe + JSON/进程边界 | 与 Rust 共享可核验媒体工具和参数，不在 C# 内重新写解码器或滤镜引擎。 |
| 音频 | PortAudio 既有契约；必要时使用成熟原生绑定，但不引入第二生产后端 | 保持两端 PCM、设备和时钟语义一致。 |
| 固定语音 | Windows SAPI COM (`SAPI.SpVoice`) | 当前适配器已存在；无需新增 `System.Speech` 包。 |
| IPC | Named Pipe/NDJSON + 明确消息大小、超时、取消和 ACL | 适合本机 sidecar；不暴露任意命令或任意文件操作。 |
| 进程与发布 | Windows Job Object、Authenticode/WinVerifyTrust、现有安装/回滚脚本 | 直接解决 Windows 退出、签名和发布门禁。 |

### 5.2 延后或明确不选

| 候选 | 决策 | 原因 |
|---|---|---|
| NAudio | 默认不选；只允许作为一次有明确指标的 Windows 设备探针或实验分支 | 引入第二音频后端会造成设备、时钟、PCM 和故障语义分叉；不能解决当前“未接入最终 PCM 总线”的核心问题。 |
| `System.Speech` NuGet | 不新增 | 当前 SAPI COM 适配器已满足固定话术，新增包只会增加运行时和部署变量。 |
| SharpDX | 不选 | 已停止维护；现有 Vortice 能覆盖 D3D11/编译器边界。 |
| Silk.NET | 不选作核心 GPU 层 | 泛化范围大，但不能替代当前明确的 WinRT/WGC/D3D11 契约，且会增加依赖面。 |
| LibVLC | 不选 | 会引入第二媒体播放器和不同的滤镜、时钟、硬件加速语义；当前产品已经以 mpv/libplacebo 为基线。 |
| FFmpeg.AutoGen/OpenCV | 不选作默认媒体引擎 | 增加 ABI、许可证、内存和升级风险；当前外部 FFmpeg 已满足进程边界需求。 |
| RestSharp/Polly/自建进程框架 | 不选 | BCL 已覆盖 HTTP、取消、超时和进程生命周期；新抽象不能替代真实验收。 |

新增 NuGet 只有在现有 BCL、Vortice、Windows API 或外部工具明确无法满足时才允许。新增前必须记录真实调用点、版本锁定策略、许可证、安全记录、包体积、替代方案和移除路径。

## 6. 分阶段实施路线

### Phase 0：基线冻结与契约测试（P0）

目标是先让两端对“同一个功能是否完成”有相同定义。

当前进度：已建立 13 项能力同步矩阵、7 个跨语言 JSON fixture/错误类别样例及 PowerShell 5.1/7 无依赖校验入口；两端读取相同 fixture 并产出相同状态/错误类别的行为测试仍待补齐。

- 建立一份机器可读的同步矩阵：能力、端、状态标签、依赖、验收证据、负责人和更新时间；当前文件为 [`2026-09-04-CSharp与Rust桌面端功能同步同步矩阵.json`](./2026-09-04-CSharp与Rust桌面端功能同步同步矩阵.json)。
- 为媒体参数、播放状态、音频候选、RTMP、虚拟摄像头和抖音 M1 建立最小 JSON fixture/错误码样例，集中放在 [`2026-09-04-csharp-rust-sync-fixtures`](./2026-09-04-csharp-rust-sync-fixtures/)；使用 [`tools/verify-csharp-rust-sync-fixtures.ps1`](../../../tools/verify-csharp-rust-sync-fixtures.ps1) 做无依赖静态校验。
- 清理或标注 Rust/C# UI 中与当前真实状态不一致的标签；不删除历史审计代码，除非另立代码清理任务。
- 把历史“458/458”等计数明确标为对应测试集结果，不作为真实设备、远端服务或发布门禁结果。

退出条件：两端能够用相同输入 fixture 得到相同的状态、错误类别、降级顺序和终态；尚未接线的能力不会显示为已完成。

### Phase 1：安全边界、所有权与生命周期（P0）

当前进度：Rust 已通过最小 `autolive-media-output-ownership` 封装消费与 C# 相同的 `Local\\GpAutoLive.MediaOutput.Owner.v1`；跨进程启动竞态、实际输出资源争用和发布环境矩阵仍待验收。

- 统一媒体输出所有权名称、互斥策略和启动顺序；启动 mpv、PortAudio、RTMP、虚拟摄像头前都必须通过同一仲裁。
- 补齐 C# 与 Rust 的取消、超时、Job Object、线程 Join、管道关闭和窗口关闭回收。
- 统一敏感数据脱敏：RTMP URL、Token、Cookie、二维码凭据、sidecar 原文和用户回复正文不得进入普通日志或 Renderer。
- 统一单实例、换源、停止、崩溃恢复和重复启动的错误语义。

退出条件：故障注入和第二实例测试均 fail-closed；没有孤儿进程、无人管理线程或可绕过的任意 IPC 命令。

### Phase 2：C# 媒体播放与效果闭环（P0）

这是 C# 同步工作的核心阶段。

当前进度：C# 已把初始效果快照和后续更新入口接入单一 mpv IPC 命令路径，完成四参数 GPU83 基线 shader 的受管资源接线、完整 83 项参数映射、更新后的固定 `glsl-shader-opts` 白名单回读及真实夹具证据；WPF v2 结果快照现在已成为正式参数源，并按完整 shader 哈希选择 GPU83、旧包选择 CPU4；启动已增加 `estimated-frame-number > 0` 首帧门禁，播放态更新已增加下一帧观察，原生 Win32 与实际 WPF `FinalEffectWindow` 的 GPU83 最终像素均已通过显式夹具；完整渲染能力、目标 GPU 矩阵和真实媒体设备门禁仍未完成。

1. 将 `MediaEffectParams` 等正式模型与范围校验接入 C#，保留三态/禁用字段语义。
2. 把 WPF 草稿接到 C# 播放会话：`draft → validated snapshot → MpvPlaybackSession.UpdateEffects → JSON IPC → next-frame effective echo`。
3. 实现与 Rust 一致的 GPU83/CPU4/Original 状态、失败单向降级和证据记录；CPU4 仅允许规定的四项能力。
4. 将 N/N+1 音频候选连接到单一最终 PCM 总线，再分别连接 PortAudio 和 RTMP；实现基于实际输出帧/延迟的可听时钟、过载重建、取消、旧候选丢弃和恢复。
5. 把插话文件、固定 SAPI 朗读和麦克风门控接入统一的优先级/静音/取消/恢复规则；不加入实时话术模型。
6. 复用已有缩略图、偏好和窗口组件，但不把平台便利能力误报为 Rust 已同步。

退出条件：从导入、播放、调参、换源、暂停、恢复、EOF、插话到停止均有真实状态；参数变化和音频切换能在实际输出中验证，而不只存在于 ViewModel 或纯逻辑测试。

### Phase 3：输出与受管 sidecar（P1）

#### 3.1 RTMP/RTMPS

- 统一视频/声音/音画三种轨道组合和当前活动源/最终 PCM 的输入。
- 以 FFmpeg 进程退出、启动无进展、远端断开和可分类握手失败驱动重试；使用固定有界退避 `1/2/4/8/15s`，不无限重试。
- 保证取消、换源、停止和退出后 FFmpeg、管道、读取线程、重试任务全部结束。
- UI 只显示脱敏原因、实际编码器和当前轨道；未通过远端 ZLMediaKit/RTMPS 门禁前保持“代码已接入·待验收”。

#### 3.2 AkVirtualCamera

- C# 直接负责安装/修复/卸载、签名、版本和白名单；WGC/D3D11 负责最终效果 HWND 捕获、缩放和 BGRA→YUY2。
- 只允许一次有界 CPU 回读给独立 GPL sidecar；固定设备名、格式、帧率和 IPC framing。
- 禁止桌面/第三方窗口捕获、GDI、CPU 重做 GPU83、WARP 冒充 GPU 和未审计 DLL。
- 通过 DirectShow、x86/x64、Win10/11、签名、干净安装、卸载和 2 小时稳定门禁后再标记真实环境通过。

#### 3.3 抖音 M1

- 先完成授权、固定上游提交、许可证、SHA-256、Conda 锁定环境和 TLS 校验，再接正式 sidecar。
- C# `DouyinLiveManager` 或 Rust manager 只能有一个会话所有者；推荐先复用 C# 已有纯逻辑规则，并以跨端 fixture 验证。
- sidecar 事件必须包含经脱敏的消息类型、消息 ID、房间 ID 和文本长度/正文边界，使 manager 能实际调用去重、自回显、随机回复和队列过期逻辑。
- 只读 `WebcastChatMessage`，单房间、单账号、扫码登录、内存凭据、串行发送；不调用模型、不经过 Go、不持久化登录、不自动规避风控。
- 真实发送、429/验证码、登录失效、断网、停止和退出均需有明确终态；未知发送结果不得盲目重发。

### Phase 4：平台差异与 UI 完整度（P2）

- 保持 C# WPF 作为 Windows 原生操作界面，不要求 Rust UI 做像素级复制。
- 只有当产品验收要求跨端一致的缩略图、偏好设置或诊断字段时，才为 Rust 补等价行为；否则保留 C# 的 Windows 优势。
- 将 C# 安装/回滚/签名能力和 Rust 运行时资源能力统一到同一 manifest/发布清单，不强行合并实现。

### Phase 5：分层验收与发布（P0/P1）

验收必须分层记录：

1. 代码检查：格式化、静态分析、未使用代码/导入/类型/配置检查、依赖审计。
2. 自动化测试：单元、契约、集成、取消/超时、并发/竞态、权限和资源回收。
3. 本地构建：C# 在本机或 CI 构建，禁止把源码放服务器构建；不使用 Git worktree。
4. 容器健康：只对实际涉及的 Go/PostgreSQL 环境报告，桌面专属任务不得用“容器健康”冒充通过。
5. 页面点击：按需要在 WPF/WebView 实机操作验证 loading、empty、error、offline、recovery 和焦点/DPI。
6. 外部模型返回：当前同步范围不调用实时模型；如未来新增模型阶段，单独报告供应商返回、usage 和凭据边界。
7. 真实业务结果：真实媒体、真实音频设备、ZLMediaKit/RTMPS、虚拟摄像头下游、抖音测试房间、干净安装和长稳分别报告。

### 测试资产来源

- 后续收到或需要定位的外部测试文件、媒体夹具和测试日志，优先从 `E:\下载` 查找。
- 当前在该目录发现 `E:\下载\load_log` 和本轮生成的明确媒体夹具 `E:\下载\csharp-golden-av.mp4`；前者不作为功能通过证据，也不把其中的敏感内容写入代码、文档或日志，后者仅用于本轮 C# FFprobe/mpv/PortAudio 显式验证。
- 仓库内 `desktop-csharp-windows/tests/` 继续作为 C# 自动化测试主目录；`desktop/` 只读，不作为本长任务的测试修改目录。

## 7. 黄金路径与验收门禁

### 7.1 黄金路径

```text
登录/激活
  → 导入 1～100 个媒体并原子提交
  → 单窗口播放并循环
  → 修改视频参数并验证有效回显
  → 修改声音参数并验证可听候选切换
  → 插话文件/固定话术/麦克风按优先级执行
  → 选择 RTMP 音画轨道并验证直推
  → 启停虚拟摄像头并验证下游画面
  → 启动抖音 M1 并验证一条弹幕到一条随机回复及自回显过滤
  → 停止、换源、断网、退出并确认资源回收
```

### 7.2 必须同时满足的门禁

- 参数模型、默认值、范围和单位与 [`媒体参数范围与默认值.md`](../../../媒体参数范围与默认值.md) 一致。
- 任何“已实现并生效”都必须有真实输出或可复核的运行时证据；UI 值、默认值、模型字段和探针通过不够。
- 失败必须可见且可分类；不能把回退、停止、未发送或未连接伪装成成功。
- 所有外部进程、线程、窗口、设备、管道和临时文件都有所有者、取消和退出策略。
- C# 新增代码必须同时更新相关项目文档；本方案和根目录约束不能被实现悄悄突破。

## 8. 风险、回滚与变更规则

- C# 先以契约测试和外部进程边界接入，避免在未证明需求前引入 FFmpeg.AutoGen、LibVLC、第二音频后端或新的消息框架。
- 如果某个库在真实设备上不能满足时钟、性能或发布门禁，优先回退到现有 BCL/Vortice/外部工具边界；更换库前更新本文的选型表和风险记录。
- GPU83、PortAudio、RTMP、AkVirtualCamera 和抖音均采用 fail-closed：能力未通过门禁时可显示待验收，但不得自动切换到不受控的桌面捕获、模型调用或无限重试。
- 任何破坏性清理、删除历史入口或删除依赖都必须先确认调用方、迁移影响和可恢复路径；本长任务默认不顺手清理无关历史代码。
- 本方案发生边界、库、验收或状态变化时，先更新本文，再更新 [`长任务开发总计划.md`](../../../长任务开发总计划.md) 的阶段记录，最后实施代码。

## 9. 后续任务执行模板

后续每次进入本长任务时，按以下顺序执行：

1. 指明本次处理的能力、端、阶段和非目标。
2. 阅读当前产品文档、相关源码、调用方、依赖和测试，并更新同步矩阵。
3. 先写失败测试或等效静态/行为门禁，再实现最小闭环。
4. 对跨模块且相互独立的开发任务，才拆分子线程；主线程在子线程完成后审查 diff、检查未使用代码和统一验证。
5. 宣布并执行一次最小化检查，删除没有真实验收依据的抽象、fallback、队列、配置和重复实现。
6. 分别报告代码检查、自动化测试、本地构建、容器健康、页面点击、外部模型返回和真实业务结果；未执行项明确写“未执行/不适用”。
7. 更新本文状态、剩余风险和下一步，不把历史测试数量复制为本轮结果。

## 10. 本阶段交付物

本阶段已完成方案文档、Phase 1 共享媒体输出所有权锁，以及 Phase 2 C# mpv 初始/开关效果 IPC 和本地音频→RTMP 共享最终 PCM 的第一段接入。后续实施仍按阶段产出：

- 本文持续维护的方案、决策记录，以及 [`2026-09-04-CSharp与Rust桌面端功能同步同步矩阵.json`](./2026-09-04-CSharp与Rust桌面端功能同步同步矩阵.json)。
- Phase 1 跨客户端媒体/输出所有权锁：Rust 与 C# 已消费同一命名 Mutex；真实进程争用仍待验收。
- Phase 2 C# 媒体接入第一段：视频处理开关可向活动 mpv 会话提交受限快照，CPU4 夹具可回读活动 `vf` 链，GPU83 基线夹具可回读四项 `glsl-shader-opts`；本地音频会话可作为最终 PCM 生产者供 RTMP 非拥有消费，有限音频项已完成真实 N/N+1 预载和单一 PortAudio 输出切换，可听时钟已接入实际输出帧/延迟并用于同身份视频进度投影；完整 shader 效果回显、真实时钟稳定性、设备恢复、过载重建和远端输出仍待验收。
- 两端可复用的契约 fixture、错误码和状态标签测试；当前 fixture 校验入口为 `powershell -NoProfile -ExecutionPolicy Bypass -File tools/verify-csharp-rust-sync-fixtures.ps1`。
- C# 媒体运行时闭环、音频总线、RTMP、虚拟摄像头和抖音 M1 的实现与分层验收记录。
- 本轮 C# 真实夹具追加：`E:\下载\csharp-golden-av.mp4` 经受管 FFprobe 成功导入媒体池，并分别通过 mpv Original、CPU4 播放时间/seek/EOF 与 PortAudio 解码输出；这只证明外部工具边界和音视频输入链可运行，不替代 WPF 点击、真实视觉参数、远端输出和设备恢复验收。
- 2026-09-04 显式夹具复验：使用 `E:\下载\csharp-golden-av.mp4` 和 `artifacts/csharp-gpu83-real-v2`，C# Windows 真实媒体夹具通过 FFprobe/mpv 完整 GPU83 导入、播放时间、seek、EOF 与初始/更新 shader 参数回读 `2/2`；同一运行包下声音控制器真实 PortAudio/FFmpeg、实时效果、N/N+1 预载切换测试 `11/11` 通过。该结果仍不替代 WPF 原生点击、像素效果、声卡稳定性和远端 RTMP 验收。
- 2026-09-04 视频独立输出故障夹具：使用 `E:\下载\csharp-golden-av.mp4`、C# 当前 WPF 代码和无效 PortAudio 设备索引，验证声音启动失败时 mpv 会话仍保持活动、媒体池保持 `Playing`，该夹具 `1/1` 通过；不替代真实声卡恢复、原生点击和带音频设备的端到端验收。
- 2026-09-04 音频 EQ 同步修复：只修改 C# `FfmpegAudioFilterBuilder` 及其两个滤镜链测试，将低频 EQ 中心频率从 `120 Hz` 对齐到 Rust realtime 链的 `200 Hz`；中频/高频保持 `1000/8000 Hz`。没有新增音频后端、DSP 抽象或依赖；动态范围、压缩和音色仍保持“正式需求·待实现/未接入”，因为当前 C# 正式参数模型没有可验收的对应输入。Rust 端差异仅做只读契约比对和待办记录；不在本长任务实施 Rust 代码、依赖、锁文件、生成物或测试修改。
- 2026-09-04 外部夹具与 WPF 层复验：后续外部测试资产固定优先从 `E:\下载` 查找，本次只使用 `E:\下载\csharp-golden-av.mp4`，未读取 `E:\下载\load_log`。在 `artifacts/csharp-gpu83-real-v2` 包根下显式运行，C# Windows 真实 mpv CPU4 `2/2`、GPU83 `2/2`、FFmpeg/PortAudio 音频路径 `2/2` 均通过；WPF `VideoPlaybackAudioFallbackTests` 正确完成主窗口导入→FFprobe→原子入池→首项选中→mpv 播放和无效 PortAudio 设备时视频保活，共 `2/2` 通过。该证据仍不替代原生人工页面点击、下一帧/像素效果、真实声卡稳定性、远端 RTMP、虚拟摄像头和发布门禁；Rust 端本轮保持只读且未修改。
- 2026-09-04 CPU4 像素效果门禁补充：新增仅在 `AUTOLIVE_TEST_PIXEL_EFFECTS=1` 显式开启的 C# WPF 夹具，使用 `E:\下载\csharp-golden-av.mp4`、最终效果视频 HWND、现有 WGC/D3D11 GPU→YUY2 受管转换链，对比 Original 与极端 CPU4 参数提交后的最终帧 SHA-256；`VideoEffectPixelFixtureTests` `1/1` 通过，证明“mpv 参数提交→最终 WPF 视频表面像素变化”。该测试保持普通套件隔离，不新增运行时逻辑、播放器或依赖；GPU83 像素变化、下一帧有效回显、目标显卡矩阵、真实人工点击和其他发布门禁仍待验收。
- 2026-09-04 下一帧有效回显接线与复验：C# `MpvIpcProperty.EstimatedFrameNumber` 复用 mpv 有界帧号属性；播放态视频效果更新在已有参数回读后增加 2 秒有界轮询，确认下一帧号前进，否则返回 `EffectiveFrameNotObserved`；暂停态不伪造等待结果。使用 `E:\下载\csharp-golden-av.mp4` 验证 CPU4 和完整 GPU83 真实夹具各 `1/1`，并覆盖关闭效果回到 Original；新增固定 IPC JSON 契约测试 `1/1`，干净环境全量 C# 测试 `494/494`。该证据仍不替代 GPU83 最终像素、目标显卡矩阵和人工页面首帧验收；Rust 端保持只读且未修改。
- 2026-09-04 启动首帧门禁接线与复验：C# `WindowsMpvPlaybackController.StartAsync` 新增可选的 `waitForFirstFrame`，主窗口正式播放路径和真实 CPU4/GPU83 夹具均在控制器标记 `Playing` 前，以既有 `estimated-frame-number` 属性进行 2 秒有界轮询；首帧未出现返回 `FirstFrameNotObserved` 并释放 mpv 运行时，不把进程存活或 IPC 成功冒充画面可用。使用 `E:\下载\csharp-golden-av.mp4` 验证 CPU4、完整 GPU83 各 `1/1` 通过，Rust 端保持只读且未修改。该证据仍不替代 WPF 最终视频表面首帧、GPU83 像素、目标显卡矩阵和人工页面验收。
- 2026-09-04 WGC 复验补充：新增 GPU83 像素夹具的尝试未计入通过；实际复跑发现 WPF 预留子 HWND 无法被 `GraphicsCaptureItemInterop.CreateForWindow` 取得，CPU4 复跑也同样返回 `ItemUnavailable`。临时诊断代码和日志已移除，没有留下运行时改动或新增依赖；GPU83 像素门禁继续保持“代码已接入·待验收”。
- 2026-09-04 开发日报：已新增 [`2026-09-04-CSharp复刻Rust桌面端开发日报.md`](./2026-09-04-CSharp复刻Rust桌面端开发日报.md)。按同步矩阵 12 项当前实施能力计算，C# 复刻整体开发进度暂记约 `70%`；今日任务完成率记为 `100%`。该百分比不把“代码已接入·待验收”冒充真实环境完成，也不把非当前范围的实时话术幻化计入分母。C# WPF 桌面端已重新打开供用户截图，Rust 端仍只读且未修改。
- 2026-09-04 WGC 事件取帧补充：C# WGC frame pool 已补齐 `FrameArrived` 唤醒和专用线程排空 `TryGetNextFrame` 的生命周期，回调仅投递有限信号并在释放阶段安全忽略已关闭信号；`dotnet build`、`dotnet format --verify-no-changes` 和 C# 全量测试 `496/496` 通过。当前同机预留子 HWND 复验仍为 `ItemUnavailable`，所以 GPU83/WGC 最终像素门禁继续保持待验收；本轮未修改 Rust。
- 2026-09-04 WGC 捕获目标修正：C# 虚拟摄像头/WGC 绑定改为最终效果窗口的顶层 HWND；mpv 继续使用 `ReservedVideoSurface` 子 HWND 作为渲染宿主，避免把 WGC 目标和播放器宿主混为同一句柄。顶层句柄通过 `WindowInteropHelper` 在窗口显示后取得，未新增窗口或播放器；随之切换的可选像素夹具未在有界时间内完成，已中止并清理测试宿主，未计为像素通过，待独立可捕获窗口环境复验。
- 2026-09-04 Rust 实时声音预设同步：C# `FfmpegAudioFilterBuilder` 增加 `NaturalVoiceMode.NaturalDynamic` 的低幅度周期响度包络和 `VoiceLibraryId` 的确定性本地 EQ 预设；WPF 系统生成的“随机预设”现在将当前预设 ID 和 4 秒周期送入同一受管 FFmpeg `-af` 链。新增 2 个滤镜链测试，确认真实消费、同 ID 稳定输出和不引入 `loudnorm`；动态范围、压缩等没有 Rust/C# 正式字段对应关系的展示值仍不伪装为已生效。仅修改 C# 项目和本方案文档，Rust 端保持只读。
- 2026-09-04 C# 真实 UI 黄金路径复验：通过本机 WPF UI Automation 点击“添加媒体”，在文件对话框选择 `E:\下载\csharp-golden-av.mp4`，界面显示媒体池 `1` 项并选中当前源；点击“播放或暂停”后界面显示“正在播放”，实际 `mpv.exe` 与 `ffmpeg.exe` 均从已校验的 `csharp-gpu83-real-v2` 运行时启动。随后分别切换视频处理、声音处理开关并恢复，媒体池仍为 `1` 项、界面仍为“正在播放”，两个处理开关均恢复为开启；截图已更新为导入并播放状态。该证据补强真实页面交互，但不替代 GPU83/WGC 最终像素、真实声卡稳定性、远端输出和发布门禁；Rust 端保持只读。
- 2026-09-04 WGC 窗口隔离门禁复验：新增 C# `GpAutoLive.App.Tests` 的两个真实 WPF 测试，分别创建普通可见 WPF 窗口和实际 `FinalEffectWindow`，绑定顶层 HWND，启动/停止 `CreateFreeThreaded` WGC frame pool，结果 `2/2` 通过。该证据确认 WGC 会话、WinRT 工厂调用和顶层窗口目标边界本身可工作；mpv 挂载到预留视频子 HWND 后的最终像素夹具仍未证明。预装 WGC 的实验性像素夹具改动因出现无界等待已撤回，没有保留生产路径变化；串行 C# 全量测试为 `498/498`，Rust 端保持只读。
- 2026-09-04 WGC 可选像素夹具等待收敛：C# 测试辅助将 WPF 异步导入/启动等待改为有界 `DispatcherFrame`，将 WGC 帧等待改为有界后台线程等待，移除嵌套同步 `Dispatcher.Invoke` 和临时诊断日志；生产播放、WGC 捕获和窗口绑定路径未改变。普通 WPF/实际 `FinalEffectWindow` 隔离测试仍为 `2/2`，mpv 挂载后的最终像素仍不计为通过，串行 C# 全量测试与 Release 构建均通过，Rust 端保持只读。
- 2026-09-04 WGC 无帧门禁复验：C# 以 activation factory + `CreateForWindow` 对齐 Rust 入口，并在同一捕获线程加入 100ms 有界轮询；原生 Win32 持续重绘窗口、普通 WPF 窗口和实际 `FinalEffectWindow` 均可建立 `Running` frame pool，但当前 Windows 11 26200/虚拟显示适配器环境在 5 秒内仍无 `FrameCount`，因此未伪造像素通过。临时 `FrameCount` 强断言、置顶/改字和阶段日志均已撤回，启动/停止隔离测试恢复为 `2/2`；Rust 端保持只读。
- 2026-09-04 C# 运行时链复核：在已打开的 C# `GpAutoLive` 实例上读取受管 mpv 命名管道和进程命令行，确认当前源为 `E:\下载\csharp-golden-av.mp4`，mpv 实际启动参数包含 `--vo=gpu-next`、C# 完整资源包中的 `gpu83.hook` 和当前源路径；同一实例的 FFmpeg 子进程实际带有增益、周期响度包络、确定性 EQ、倍速、降噪等 `-af` 参数，mpv `estimated-frame-number` 与 `time-pos` 可读且处于播放态。该证据证明“C# 播放控制器→受管 mpv/FFmpeg”真实链路已接通，但不替代 GPU83 最终像素、真实声卡稳定性和 WGC 下游门禁；Rust 端保持只读。
- 依赖、许可证、SHA-256、发布包、签名、安装/卸载和长稳证据。

本文作为后续 C# ↔ Rust 功能同步工作的长任务基线。

### v1.36 并行增量复核记录（2026-09-04）

- 媒体黄金路径子任务先以失败测试复现 `MediaPoolOwner.ReplaceAll` 的容量边界，再将容量校验收窄为仅对 `Append` 叠加旧池长度；已有 100 项时 `ReplaceAll(1)` 现在可以原子替换为 1 项，`Append` 的 100 项上限和失败保留旧快照语义不变。
- WGC 硬件链子任务先以测试锁定 D3D11 创建能力，再将 C# `WindowsD3D11HardwareContextFactory` 的实际创建调用统一改为 `DeviceCreationFlags.BgraSupport | DeviceCreationFlags.VideoSupport`；未新增依赖、未扩展到未经验证的 VideoProcessor 实现。
- 主任务使用项目锁定 SDK 复验：Contracts `28/28`、Installer `15/15`、Core `77/77`、Media `115/115`、Windows `210/210`，App 常规 `53/53` 和两个真实窗口用例单独 `1/1+1/1`，稳定分组总计 `500/500`；C# Windows 项目 Release 构建 `0` 警告/`0` 错误，完整 App 另以 `-p:OutDir` 隔离输出构建 `0` 警告/`0` 错误。
- Rust `desktop` 仍包含用户既有脏变更，作用域门禁按设计 fail-closed；本轮未修改、未回滚、未删除 Rust 文件。GPU83 最终像素、真实声卡/远端输出、WGC 子 HWND 最终画面和发布门禁仍保持待验收。

### v1.37 C# 音频独立 RTMP PCM 接线（2026-09-05）

- 发现并修复 C# 独立 `WindowsRtmpAudioSession` 未把当前声音处理开关/参数传给 `FfmpegPcmDecodePlanBuilder` 的缺口；现在 WPF 开启声音处理时使用当前 `AudioEffectParams` 生成同一受管 `-af` 链，关闭时保持原始 PCM。共享本地最终 PCM 的 PortAudio/RTMP 分流路径不新增生产者。
- 失败先行测试在修复前返回 `StartFailed`，修复后在 RTMP 宿主启动前返回 `InvalidArguments`；仅涉及 `desktop-csharp-windows`，Rust 端保持只读。
- 本轮未执行真实 ZLMediaKit/RTMPS、真实声卡、远端可听性或长稳验收；不能将该自动化结果写成真实业务通过。

### v1.38 C# WGC 输出上下文接线（2026-09-05）

- 发现并修复生产虚拟摄像头链只提交 WGC/GPU→YUY2 帧、却未同步 `VirtualCameraOutputContext` 的缺口。主窗口现在在媒体快照提交时同步播放态、视频源、暂停/停止和当前媒体身份；身份变化会使上一媒体的有效帧失效，暂停恢复仍可复用同一媒体的最后有效帧。
- `WindowsVirtualCameraGpuOutputSession` 在首次成功 GPU 回读并提交帧后标记 `HasValidFrame=true`，在启动、停止和释放时清除旧有效帧事实。该改动只复用现有 `VirtualCameraOutputManager`、WGC、D3D11 和 sidecar writer，不新增播放器、捕获路径或依赖。
- 先加入 `VirtualCameraOutputContextWiringTests` 作为 C# 接线门禁；本机缺少项目 `global.json` 要求的 .NET SDK `10.0.400`，未能执行 MSTest、构建和真实 WGC/sidecar 验收。静态检查确认实现前生产代码缺少两处接线；本轮不把 IPC、进程存活、帧号或启动状态当作像素通过，Rust 端保持只读。

### v1.39 主线程代码总监复核（2026-09-05）

- 复核确认 v1.37 的 `WindowsRtmpAudioSession` 只在没有共享最终 PCM 总线时创建独立解码器；声音开关开启时 `AudioEffectParams` 进入既有 `FfmpegPcmDecodePlanBuilder`，关闭时保持原始 PCM；共享 PortAudio/RTMP 分流仍只有原有最终 PCM 生产者，没有新增后端或重复解码。
- 复核确认 v1.38 的 `SyncVirtualCameraOutputContext` 会在媒体快照提交时同步播放态、源媒体、暂停/停止和媒体身份；WGC 首个成功 GPU→YUY2 回读后才标记 `HasValidFrame`，启动/停止/释放会清除旧事实；没有把 IPC、进程存活或帧号当作像素验收。
- 使用项目自带 SDK `desktop-csharp-windows/.tools/dotnet/dotnet.exe` 复验：Windows 受影响测试 `7/7`，WGC 接线测试 `1/1`，Windows 全量 `211/211`，App 常规（排除两个真实窗口用例）`54/54`，两个真实窗口用例分别 `1/1+1/1`；App Release 构建 `0` 警告/`0` 错误，`dotnet format --verify-no-changes` 通过，夹具校验 `7` 个 JSON 通过。
- Windows 全量首次出现一次测试夹具清理时的 `mpv.exe` 文件占用，单独重跑后通过；不归因于本轮业务代码。作用域脚本仍因工作区原有 Rust/Tauri 路径 fail-closed，本轮未修改、未回滚、未删除 Rust。
- 消融复核删除了视频子任务中错误假设的 XAML 点击测试/重复处理和无效初始上下文同步；保留真实输出上下文、有效帧生命周期和音频参数验证边界。真实 ZLMediaKit/RTMPS、声卡可听性、WGC 最终像素、sidecar/DirectShow 与发布门禁仍待验收。

### v1.40 当前任务受影响模块复验（2026-09-05）

- 主线程代码总监复核确认：媒体导入仍遵循 FFprobe 全部成功后原子提交；视频由单一 mpv 会话承载，声音由 FFmpeg PCM→PortAudio 独立承载；WGC 输出仅在首个真实 GPU→YUY2 帧转换并提交后进入 Ready。未发现需要删除的重复生产抽象、伪成功状态或无调用 fallback。
- 使用项目自带 SDK `desktop-csharp-windows/.tools/dotnet/dotnet.exe` 实测：Core `77/77`、Media `115/115`、Windows `213/213`、App `56/56`；`dotnet format --verify-no-changes` 通过；App Release 隔离输出构建 `0` 警告/`0` 错误；跨端同步 fixture `7` 个 JSON 全部通过。
- 当前工作区作用域脚本仍因用户既有 `desktop/` Rust/Tauri 路径和构建产物返回 fail-closed；本任务未修改、回滚或删除 Rust。真实声卡、RTMP/RTMPS、WGC 最终像素、AkVirtualCamera sidecar/DirectShow、目标 GPU 矩阵、长稳和人工页面点击仍未执行或未通过，不能标记为产品整体完成。
- 开发启动脚本默认运行包已调整为优先使用已校验的 `csharp-gpu83-real-v2`，缺失时才回退 v90；显式运行包参数、manifest/SHA-256 校验和登录/授权门禁保持不变。

### v1.41 黄金路径真实效果复验（2026-09-05）

- 使用 `E:\下载\csharp-golden-av.mp4` 和 `artifacts/csharp-gpu83-real-v2` 完成 WPF `VideoEffectPixelFixtureTests` `1/1`：通过最终效果窗口的受管 WGC/D3D11→YUY2 采样对比，确认视频处理参数提交后最终视频表面像素发生变化；该结果不把 IPC 回读或 mpv 进程存活当作视觉效果通过。
- 同一媒体与运行包完成 Windows `Download_fixture_consumes_realtime_audio_effects_when_explicitly_enabled` `1/1`：声音参数进入 FFmpeg `-af`，经真实 PortAudio 输出并向最终 PCM 总线发布帧；证明当前受限声音子集已进入实际执行链，不代表完整 DSP、声卡长期稳定或主观听感验收。
- WPF `VideoPlaybackAudioFallbackTests` `2/2` 通过：主窗口真实导入→FFprobe→原子入池→首项选中→mpv 播放，以及无效 PortAudio 设备时视频仍保持 `Playing`。媒体池容量、排序、删除和清空的既有单元门禁继续通过。
- 代码总监复核未发现本轮新增的未使用生产抽象、重复播放器/音频后端、伪成功状态或无调用 fallback；保留 FFprobe 整批探测、媒体池原子提交、单一 mpv、独立 PCM 总线和登录/授权 fail-closed 边界。真实 GPU83 最终像素、真实声卡稳定性、RTMP/RTMPS、WGC 下游、AkVirtualCamera sidecar/DirectShow、目标显卡矩阵、长稳和人工页面点击仍未执行或未通过，整体状态继续为“实施中”。

### v1.42 GPU83 WPF 最终表面复验（2026-09-05）

- 先用 `E:\下载\csharp-golden-av.mp4` 和 `artifacts/csharp-gpu83-real-v2` 运行较小的真实 mpv GPU83 夹具，完整 shader、参数更新、播放时间和 EOF `1/1` 通过，确认 GPU83 外部进程链本身可运行。
- 再尝试将完整 GPU83 接入 WPF 最终效果窗口并通过 WGC/D3D11→YUY2 对比最终像素；测试宿主约 3 分钟无 CPU/输出进展，已停止并清理本次测试进程，未产生生产代码变化，也未把该次尝试计为通过。GPU83 WPF 最终像素门禁继续保持“代码已接入·待验收”，后续应在可控的 WGC/窗口收尾环境中单独定位，不扩大为第二播放器或第二捕获链。

### v1.43 GPU83 原生窗口像素夹具收敛（2026-09-05）

- 为区分 WPF 收尾问题与 GPU83 播放链问题，新增仅在 `AUTOLIVE_TEST_MPV_GPU83_PIXEL=1` 显式开启的原生 Win32 窗口夹具；测试宿主主动重绘并使用同一 C# `Windows.Graphics.Capture`→D3D11→YUY2 链，目标是对比 GPU83 参数更新前后的最终帧哈希。
- 使用 `E:\下载\csharp-golden-av.mp4` 和 `artifacts/csharp-gpu83-real-v2` 复跑时，mpv 受管启动、首帧号门禁和 shader 参数链仍可运行，但当前 Windows 11 虚拟显示适配器环境在 10 秒内没有产出可转换 WGC 帧；该显式用例现在返回“跳过/不可判定”，不计为成功或失败，不改变生产播放代码。
- `WindowsMpvRealFixtureTests` 的原生窗口重绘辅助及有界不可判定路径通过 `dotnet format --verify-no-changes` 和 Windows 测试 `214/214`；Rust 端保持只读。GPU83 最终像素、真实声卡、WGC 下游、目标显卡矩阵和发布门禁仍保持待验收。

### v1.44 WGC 停止与虚拟摄像头结果边界修复（2026-09-05）

- 修复 `WindowsGraphicsCaptureWindowSession` 的停止信号生命周期：每个捕获 worker 持有自己的 `ManualResetEventSlim`，worker 完成 WinRT/D3D11 释放并退出后才清理信号；`DisposeAsync` 不再在线程仍存活时提前释放共享信号，停止等待也不再重复调用可能已释放的 `Set`。
- 修复 `WindowsVirtualCameraGpuOutputSession.StopAsync` 的结果映射：WGC 线程未能在有界预算内停止时返回 `CaptureFailed`，不再以 `Stopped`/成功语义掩盖失败；输出上下文仍会清除 `HasValidFrame`。
- 受影响的 WGC/虚拟摄像头测试 `5/5` 通过；本轮未修改 Rust，未新增播放器、捕获链、依赖或绕过授权的路径。WGC 最终像素、sidecar/DirectShow、真实声卡/RTMP、目标 GPU 矩阵和发布门禁仍待验收。

### v1.45 C# 视频周期触发接入（2026-09-05）

- 新增 `VideoEffectCyclePlanner`，复用 Rust 的“真实播放位置驱动周期”边界：视频处理开启且处于 `Playing` 时，按当前媒体 PTS 在 `5–8s` 有界范围内触发一次视频快照重生成；切源、回绕、明显回退、暂停或关闭处理均会重新布置目标，缺失位置/时长时不触发。
- 周期应用复用现有 `RunPlaybackCommandAsync` 串行闸门和 `ApplyVideoProcessingModeAsync`，不创建第二播放器、第二捕获链、后台无主线程或缓存。自动视频周期不强行重启 PortAudio；现有 `NaturalDynamic` 连续音频滤镜保持输出，音频 N/N+1 周期候选已接入按目标输出帧切换。
- 周期规划器测试 `2/2`、App 稳定测试 `53/53`、App Release 隔离构建 `0` 警告/`0` 错误、格式检查通过。真实 WPF 导入/播放全量夹具本轮在无 mpv/FFmpeg 子进程的宿主初始化阶段超过有界时间，已终止测试树，不计为通过；已有导入/播放黄金夹具证据继续有效。

### v1.46 C# 声音周期候选接入（2026-09-05）

- 新增 `AudioEffectCyclePlanner`：以 PortAudio `timeInfo` 投影的可听位置为事实源，长于一个周期的媒体在目标前约 `1s` 预载同一源媒体的新声音参数，目标周期固定为当前 C# 默认的 `4s`；短媒体、暂停、关闭声音处理或缺少可靠时钟时不触发。
- `AudioPcmTrackSwitchOutputSource` 与 `FinalPcmBusTrackSwitch` 新增有界按帧提交：PortAudio/RTMP 继续使用同一个最终 PCM 总线和唯一 N+1 候选，目标帧到达后切换，不等待整项媒体 EOF；旧 FFmpeg 解码器在周期切换后取消、Join 并释放。设备延迟被计入目标帧计算，避免声音提前切换；暂停会丢弃尚未提交的过期候选。
- App 音频完成观察器现在同时负责有限音频项完成和声音周期预载/提交；视频源的声音、纯音频源共用该路径，不创建第二 PortAudio 输出、第二音频后端或无主任务。系统生成的声音快照会进入候选 FFmpeg `-af` 链，既有 `NaturalDynamic` 连续处理仍保留。
- 新增声音规划器、按帧切换回归测试；媒体切换 `7/7`、App 声音/视频规划与状态 `16/16`、Windows 音频/时钟稳定筛选 `16/16`、App 稳定测试 `55/55`、Release 构建 `0` 警告/`0` 错误、格式检查通过。显式环境门控的真实 FFmpeg/PortAudio 8 秒音频周期夹具 `1/1` 通过，证明同一输出流在目标位置切到同源候选并正常完成；该证据不覆盖声卡长期稳定、RTMP/ZLMediaKit 和长稳门禁，Rust 端保持只读。

### v1.47 WPF 黄金路径单独复验（2026-09-05）

- `VideoPlaybackAudioFallbackTests.Main_window_imports_fixture_and_starts_video` 单独运行 `1/1` 通过，证明主窗口真实导入→FFprobe→原子入池→首项选中→mpv 播放仍可用。
- `VideoPlaybackAudioFallbackTests.Video_remains_playing_when_portaudio_device_cannot_start` 单独运行 `1/1` 通过，证明 PortAudio 设备失败不会关闭视频会话或把媒体池回滚。
- 两个用例合并到同一 WPF 测试宿主时超过 2 分钟无输出，已结束该次测试进程；该现象按宿主复用/收尾风险记录，未计入通过，也未改动生产播放代码。桌面控制服务本轮返回未配置，人工页面点击仍待可用环境复验。

### v1.48 纯音频处理中途保持可听位置（2026-09-05）

- 纯音频播放中切换“声音处理”或重新生成声音快照时，WPF 现在读取现有 PortAudio 可听时钟，并从该位置重建 FFmpeg PCM 会话；已知时长会将位置限制在媒体末尾前 1ms，暂停态仍按原状态恢复。视频声音路径继续复用 mpv `time-pos` 位置恢复。
- 本轮 App 稳定测试 `55/55`、格式检查通过；Release 构建 `0` 警告/`0` 错误。该改动没有新增音频后端、播放器、线程或队列；真实声卡长期稳定和断设备恢复仍待门禁。

### v1.49 声音周期候选避免误切媒体源（2026-09-05）

- 修复 WPF 音频完成观察器与唯一 N+1 候选槽的编排冲突：声音处理开启时不再预载下一媒体项，候选槽专用于从当前可听位置开始的同源周期效果；否则多媒体池会把下一项误当作效果候选提前提交，单项池也会从头重播而不消费新的声音快照。
- 声音处理关闭时继续使用原有下一媒体项预载与自然 EOF 切换路径；本修复不新增音频后端、播放器、线程或队列。App 稳定测试 `55/55`、Release 构建 `0` 警告/`0` 错误、格式检查通过；真实声卡长期稳定、断设备恢复和 RTMP 联合门禁仍待验收。

### v1.50 开发启动脚本同步实际 x64 二进制（2026-09-05）

- 修复 `tools/start-csharp-development.cmd` 的源码/产物分叉：脚本现在先使用仓库内项目锁定的 `.tools/dotnet/dotnet.exe`，明确以 `Platform=x64`、`--no-restore` 在本机编译当前 C# App，再启动脚本实际指向的 `bin\x64\Release` EXE；缺少 SDK 或构建失败会直接退出并显示原因。
- 该改动不把源码放到服务器，也不改变登录、运行资源 manifest、媒体导入或播放权限边界；它只确保用户启动时不会继续运行旧版本 EXE。启动脚本的本地构建命令已单独复核通过 `0` 警告/`0` 错误。

### v1.51 修复启动脚本默认运行包变量展开（2026-09-05）

- 修复 CMD 默认运行包分支的括号块变量展开错误：`DEFAULT_MEDIA_RUNTIME` 在同一块内被提前展开为空，导致不传参数时错误提示找不到运行包；现在改为无括号的顺序分支，默认优先选择 `csharp-gpu83-real-v2`，缺失时回退 v90，显式运行包参数行为保持不变。
- 默认运行包启动冒烟已通过：脚本完成本地 x64 构建并拉起目标 EXE；随后仅结束本次冒烟启动的进程。该冒烟不等于人工导入、播放或最终效果验收。

### v1.52 视频启动单向降级接入（2026-09-05）

- C# 新鲜视频会话现在按 Rust 对齐的单向顺序尝试 `GPU83 → CPU4 → Original`；每次失败由同一个 mpv 控制器完成有界清理后再尝试下一档，不创建第二播放器、不回升已降级会话。处理关闭或没有可用效果快照时直接使用 `Original`。
- `GPU83` 与 `CPU4` 的启动参数快照均在实际启动前生成并校验；成功降级后 UI 状态明确显示实际模式，避免只显示“处理开启”而掩盖效果未生效。该路径只补齐启动失败回退，不改变运行中 GPU83 周期快照更新和 CPU4 能力边界。
- App 稳定测试串行 `55/55`、真实 WPF 导入并启动视频 `1/1`、真实 FFmpeg/PortAudio 同源声音周期切换 `1/1`、格式检查通过；Release x64 构建已通过 `0` 警告/`0` 错误。GPU83 故障触发后的实机降级、目标显卡矩阵和 WGC 最终像素仍待门禁。

### v1.53 PortAudio 持续 xrun 有界重建（2026-09-05）

- C# PortAudio 输出快照新增 callback 状态标志计数和 xrun 计数；输出欠载/溢出标志或实际缺帧均只记录为健康事实，不在音频 callback 内执行重锁、文件、网络或 UI 操作。
- 音频会话健康观察器对齐 Rust 的持续异常边界：短暂启动欠载不触发恢复；连续观察达到 `1024` 个 callback 且至少 `75%` 为 xrun 时，复用现有单流 `RestartAsync` 保留 PCM 总线并重建输出流。每轮成功重建都会重置观察基线，连续重建最多 `3` 次，仍异常则以 `audio_output_overrun` fail-closed，避免无限重试或静默丢声。
- 新增 xrun 判定失败先行测试；PortAudio 输出/控制器相关测试 `28/28`、真实声音效果链 `1/1`、真实同源声音周期切换 `1/1`、App 稳定测试 `55/55`、WPF 导入并启动视频 `1/1`、格式检查和 Release x64 构建均通过。真实声卡拔插、驱动重置和 30 分钟长稳仍待目标设备门禁。

### v1.54 C# RTMP 状态收口与 CPU4 视频效果桥接（2026-09-05）

- 修复 C# RTMP 声音分流泵和解码生产任务无人观察的问题：泵/生产任务异常会投影固定失败分类、取消当前会话并有界停止 FFmpeg；正常停止取消不会被误报为失败。RTMP 进程终态通过脱敏 `SnapshotChanged` 通知 WPF，失败会显示“可重新连接”。
- 接通 WPF RTMP 重新连接按钮和现有有界协调器：失败会话按最多 `3` 次尝试重新建立同一条受管链路，失败/取消不伪造 Publishing；媒体池变更仍先停止 RTMP。
- C# RTMP 视频发布现在接收已校验的 CPU4 四项快照并生成固定 FFmpeg `eq + hue` 链；视频处理关闭时不加入 `-vf`，禁止把 mpv `@autolive_cpu4` 或任意自由滤镜字符串传入 FFmpeg。完整 GPU83 的 FFmpeg/libplacebo 直推仍未接入。
- 新增 CPU4/Original RTMP 命令计划测试和 RTMP 进程终态通知测试；相关媒体测试 `5/5`、Windows RTMP 测试 `13/13`、Release x64 构建 `0` 警告/`0` 错误通过。真实 ZLMediaKit/RTMPS 握手、远端首包、最终可听性、重连和长稳仍待实机门禁。

### v1.34 复核记录（2026-09-04）

- C# `WindowsGraphicsCaptureWindowSession` 创建 D3D11 设备时补齐 `D3D11_CREATE_DEVICE_VIDEO_SUPPORT`，与既有 `D3D11_CREATE_DEVICE_BGRA_SUPPORT` 合并使用；该标志用于 WGC 视频资源路径，属于 C# Windows 平台实现，不改变 Rust 端。
- 当前版本 `dotnet format --verify-no-changes` 通过，Release 构建 `0` 警告/`0` 错误；Contracts `28/28`、Installer `15/15`、Core `76/76`、Media `115/115`、Windows `209/209` 通过，App 测试按稳定分组 `53+1+1=55/55` 通过，合计 `498/498`。两个真实视频窗口用例分别单独运行均通过；连续放在同一 WPF 测试宿主时仍有宿主收尾挂起风险，未将该现象归因于媒体业务失败。
- `tools/verify-csharp-rust-sync-fixtures.ps1` 通过（7 个 JSON）；`tools/verify-scope.ps1` 仍按预期因工作区已有 Rust/Tauri 路径返回 fail-closed，本轮未回滚、删除或修改这些 Rust 变化。

### v1.55 WGC 句柄就绪与虚拟摄像头前置门禁（2026-09-05）

- C# `FinalEffectWindow` 的 mpv 视频子 HWND 和 WGC 顶层 HWND 统一经过 `IsWindow`、可见性和非零客户区校验；窗口显示/布局完成后才刷新虚拟摄像头绑定。
- 虚拟摄像头启动前新增当前视频播放、mpv 活动身份一致、运行时为 `Running` 的门禁，避免空窗口或无首帧时启动 sidecar 并持续发送黑帧。
- 新增关闭最终效果窗口后拒绝捕获句柄测试，WPF/WGC 隔离测试 `3/3` 通过；真实 WGC 可见帧、sidecar/DirectShow、GPU83 最终像素、目标 GPU 矩阵和发布门禁仍保持“代码已接入·待验收”，Rust 端本轮保持只读。

### v1.56 C# 媒体换源与声音分流状态收口（2026-09-05）

- `WindowsMpvPlaybackController.SwitchSourceAsync` 不再把 `loadfile` IPC 成功当作换源完成；返回成功前在 2 秒有界预算内确认 mpv `path` 已切到目标源、`eof-reached=false`、暂停/播放意图一致，播放态还必须观察到新源首帧。暂停态换源重新显式保持暂停，失败会停止并清理当前 mpv 会话。新增显式真实媒体换源夹具，先验证失败再接入等待逻辑。
- `FinalPcmBus` 默认不再向未接入的 RTMP 分支写入 PCM；RTMP 会话创建/停止时显式绑定或解绑，绑定切换会丢弃旧尾部，候选总线继承当前绑定状态。PortAudio 仍消费同一最终 PCM 事实源，RTMP 只读取真实接入后的新帧。
- `AudioProcessing` 改由 `ShellState.PropertyChanged` 统一触发重配置，移除仅依赖 WPF `Click` 的第二入口，避免程序化开关只更新标签而不重建 FFmpeg/PortAudio 声音链。已有实时音频滤镜子集保持不变；动态范围、压缩和音色展示字段仍明确不标记为已生效。
- 相关媒体/总线测试通过；真实 mpv 换源夹具 `1/1`、C# 真实 FFmpeg/PortAudio 声音消费夹具保持通过。直接双击未携带外置运行包的开发 EXE 仍会 fail-closed，必须使用发布包内 `runtime/media/<version>` 或 `tools/start-csharp-development.cmd`，不通过静默扫描任意目录规避 manifest 校验。

### v1.57 C# 多媒体池 EOF 黄金路径复验（2026-09-05）

- 新增主窗口真实导入→两个媒体原子入池→首项播放→第一项 EOF→自动切换第二项的显式夹具；测试使用同一真实媒体的独立临时副本，验证媒体池索引和播放状态均进入第二项 `Playing`，不只验证 `MediaPoolOwner` 的纯逻辑状态。
- 本增量只补齐 C# 媒体黄金路径证据，没有新增播放器、观察线程、缓存或绕过登录/运行包校验的入口；Rust 端继续只读。主窗口人工点击、GPU83 最终像素、真实声卡/RTMP 和 WGC 下游门禁仍按既有状态保留。
- 真实双项 EOF 夹具 `1/1` 通过；测试宿主与外置运行包均在本机有界执行。

### v1.58 C# WGC surface 解包与真实 GPU83 像素闭环（2026-09-05）

- 对齐 Rust `surface.cast()` 的 WinRT ABI：C# `WindowsGraphicsCaptureGpuYuy2Converter` 不再对 `Marshal.GetIUnknownForObject(frame.Surface)` 做二次 RCW 包装，改从 `IWinRTObject.NativeObject` 直接取得 `IDirect3DDxgiInterfaceAccess` 和 `ID3D11Texture2D`。失败先行诊断确认 WGC 已收到 `422` 帧但旧路径全部返回 `SurfaceUnavailable`，修复后显式 GPU83 像素夹具 `1/1` 通过。
- C# `WindowsGraphicsCaptureWindowSession` 对齐 Rust 的 adapter 边界：最多枚举 32 个 DXGI adapter，跳过 Software/Remote 设备，以 `D3D_DRIVER_TYPE_UNKNOWN` 在真实硬件 adapter 上创建 `BGRA|VIDEO` D3D11 设备；单个 adapter 初始化失败会释放本轮 COM/D3D 资源并继续尝试，全部失败则 fail-closed。
- 消融检查删除了不再使用的 DXGI access GUID 和旧 IUnknown→RCW 路径；保留单一 WGC 会话、同设备 GPU 转换、三槽有界回读、WinRT/COM 释放与输出首帧门禁。Windows 全量 `217/217`、App 全量 `62/62`、格式检查均通过；本机启动脚本重新以 x64 Release 构建并拉起最新 C# App，构建 `0` 警告/`0` 错误。
- 该显式像素证据使用可捕获的原生 Win32 宿主，证明 WGC surface 解包和 GPU83 参数变化确实改变最终 YUY2 帧；同一修复后的实际 WPF `FinalEffectWindow` CPU4 最终视频表面夹具也重新通过 `1/1`。GPU83 在 WPF `FinalEffectWindow` 子 HWND 挂载下的独立像素门禁、AkVirtualCamera sidecar/DirectShow、目标 GPU 矩阵、真实声卡/RTMP 和长稳仍需独立验收，Rust 端本轮保持只读。

### v1.59 WPF FinalEffectWindow GPU83 像素闭环（2026-09-05）

- `VideoEffectPixelFixtureTests` 新增显式 WPF GPU83 用例，沿用真实导入、原子媒体池、主窗口启动、`FinalEffectWindow` 顶层 HWND、WGC/D3D11 转换和 YUY2 哈希链；启动时开启视频处理，确认完整 shader 运行包走 GPU83，再提交极端 GPU83 参数并等待最终表面哈希变化。
- 该用例使用 `E:\下载\csharp-golden-av.mp4` 与 `artifacts/csharp-gpu83-real-v2`，结果 `1/1` 通过；CPU4 对照用例仍为 `1/1`，App 全量测试 `63/63`，App 格式检查通过。测试只扩展验证边界，不新增播放器、捕获线程、依赖或运行时分支。
- 现在 C# 视频效果的“参数更新→mpv 下一帧→实际 WPF 最终表面像素变化”已有 GPU83/CPU4 两条显式证据；目标显卡矩阵、AkVirtualCamera sidecar/DirectShow、真实声卡/RTMP、人工页面和长稳仍待独立门禁，Rust 端继续只读。

### v1.60 Windows 集成状态文案收口（2026-09-05）

- `WindowsCapabilityBoundary.Status` 从“正式需求·待实施/未接入”修正为“代码已接入·待验收”，与当前真实媒体导入、单一 mpv、声音分流、WGC/D3D11 和 GPU83/CPU4 像素夹具证据一致。
- 该变更只修正文案和边界注释，不新增抽象、依赖、运行时分支或权限；App 全量测试 `63/63`、Windows 格式检查通过。真实设备、下游兼容、签名和长稳仍按矩阵单独验收。

### v1.61 音频首帧门禁与 RTMP 非阻塞收口（2026-09-05）

- `WindowsAudioPlaybackController.StartAsync` 现在必须在 3 秒有界预算内观察到 FFmpeg PCM 已写入目标，才返回成功；取消、无 PCM 或解码提前结束会执行有界收尾并返回稳定错误，避免主窗口在实际没有可听声音时显示“声音处理已应用”。循环启动结果保留已观测首帧快照，避免下一轮解码重置本轮计数造成假 0 帧。
- `WaitForBusCapacityAsync` 只对本机 PortAudio 输出分支做节流；RTMP 分支遵循 `FinalPcmBus` 固定容量环缓的独立丢旧策略，不再因 RTMP 泵暂时未读而反向卡住本机声音和视频时钟。显式真实 FFmpeg/PortAudio 夹具覆盖循环、暂停/恢复、有限总线、RTMP 分流和带音轨视频效果消费 `2/2`，Windows 全量 `217/217`。
- 本增量未新增依赖、接口、第二播放器或 Rust 改动；真实声卡拔插/长稳、ZLMediaKit 远端和人工页面仍保持独立待验收。

### v1.62 C# 主窗口导入、媒体池与 EOF 黄金路径复审（2026-09-05）

- `MainWindow.MediaPool` 的导入入口继续先执行登录/设备授权门禁，再执行已校验运行包与 FFprobe 探测；运行包或探测资源缺失时保留旧媒体池并返回可操作的修复提示，不把旧 IPC 成功或单独的池索引变化当作导入/换源成功。
- `VideoPlaybackAudioFallbackTests` 新增未授权导入、运行包缺失保留旧池、PortAudio 失败时视频仍保持播放和真实主窗口导入播放边界；强化 EOF 夹具从真实两项原子入池、`Ready → Playing` 开始，等待第一项 EOF 后确认第二项活动身份、mpv 播放态和主窗口换源状态。该测试类 `5/5`，App 全量 `65/65`，App 项目格式检查和 Release 构建均通过。
- 主线程代码审查确认本增量没有引入额外播放器、观察线程、缓存或第二套媒体池；保留登录、manifest/SHA-256、FFprobe、原子提交、单一 mpv、首帧 PCM 门禁和有界队列。人工页面点击、真实声卡长期稳定、ZLMediaKit/RTMPS 远端、AkVirtualCamera/DirectShow 下游、目标 GPU 矩阵和发布签名门禁仍未验收，Rust 端继续只读。

### v1.63 mpv IPC 故障状态与暂停换源收口（2026-09-05）

- `WindowsMpvPlaybackController.Snapshot` 现在同时要求受管运行时为 `Running` 且 IPC 仍为 `Connected`；IPC 已关闭但 mpv 进程尚存时投影为 `Faulted`，不再把进程存活误当作播放成功。
- `ShutdownAsync` 不再吞掉底层 mpv 停止失败，向调用方返回 `StopFailed` 并保留故障状态；暂停态换源重新确认暂停意图，恢复后仍使用同一 mpv 会话。
- IPC 故障注入测试 `1/1`、暂停换源/恢复/停止真实夹具 `1/1`、控制器基线 `6/6`、Windows 项目格式检查通过。临时文件清理已在显式控制器释放后执行，避免测试收尾顺序掩盖资源释放结果；真实连续 EOF、多 GPU 和长稳仍待验收。

### v1.64 RTMP 启动失败与停止重试资源收口（2026-09-05）

- `WindowsRtmpOutputManager` 在停止预算内未确认进程退出时保留 PID、Job、stdin 和状态，后续 `StopAsync/DisposeAsync` 仍可重试；启动清理同样不把未退出进程伪装成已回收。自然进程退出会清理 PCM 输入并发出脱敏失败快照。
- `WindowsRtmpAudioSession` 分别观察 Producer、Pump 和插话任务；启动期 `PumpFailed/DecodeFailed` 不再返回成功。若底层宿主停止失败，即使工作任务已 Join 也保留会话资源，避免下一次 Stop 无法回收；Dispose 只有成功停止后才释放生命周期信号。
- RTMP 管理器/声音会话限定测试 `14/14`、Windows 全量 `220/220`、格式检查和 Windows 构建均通过。真实 ZLMediaKit/RTMPS 握手、远端断线恢复、真实声卡、三轨道和长稳仍未验收，Rust 端继续只读。

### v1.65 C# 导入入口与 GPU83 回读兼容收口（2026-09-05）

- 修复 `MainWindow.xaml` 底部主导入按钮仍为隐藏状态的问题；授权态下底部与顶栏入口均可见且可用，仍共同进入 `RunImportAsync`，未放宽登录/设备授权门禁。
- `MpvShaderOptionsSnapshot.MatchesMpvReadback` 兼容 mpv 合法返回的 `glsl-shader-opts` 字符串，并对对象/数字标量做有限规范化；固定键集合、字符/长度/数值校验和额外键拒绝保持不变，避免把任意滤镜正文当作成功。
- 新增授权后导入入口测试与字符串/数字/额外键回读测试；App `66/66`、Media `122/122`、Release x64 构建 `0` 警告/`0` 错误、App/Media 格式检查和 `git diff --check` 通过。真实页面点击、目标 GPU 矩阵、真实声卡/RTMP、AkVirtualCamera 下游和长稳仍待验收，Rust 端继续只读。

### v1.66 C# RTMP 草稿撤回与真实 mpv 播放池门禁（2026-09-05）

- 主线程复审第二批并行结果后撤回未形成闭环的 RTMP stdout 进度字段、读取任务、命令行参数和对应测试；当前 RTMP 契约继续只以已验收的受管进程、stderr、最终 PCM 分流、停止重试和脱敏状态为准，避免把半成品进度指标引入正式状态。
- 修正 `WindowsRtmpOutputManager.StopCoreAsync` 的 `_audioSerial` 生命周期：无音频 stdin 时不取得也不释放音频串行锁，只有实际取得锁的 PCM 收尾路径才释放，保留三轨道、取消、停止重试和进程树回收边界。
- 新增真实 mpv 连续 EOF→多项换源→新源实际播放，以及单项媒体池循环身份递增门禁；Windows 全量 `222/222`、Media 全量 `123/123`、App 全量 `66/66`，RTMP 受影响测试 `12/12`（Media 命令计划 `6/6`、Windows 管理器 `6/6`），Media/Windows/App 格式检查通过，启动脚本本地 Release x64 构建 `0` 警告/`0` 错误并拉起最新实例。真实页面点击、真实声卡、远端 ZLMediaKit/RTMPS、AkVirtualCamera 下游、目标 GPU 矩阵、签名和长稳仍待验收，Rust 端继续只读。

### v1.67 C# 音视频参数真实消费与声音故障恢复（2026-09-05）

- `GeneratedAudioEffectSnapshot.ToAudioEffectParams()` 现在把频域扰动、频谱盲区和高频扰动正式字段送入当前 `AudioEffectParams`；`FfmpegAudioFilterBuilder` 生成对应的 `afftfilt`、`bandreject` 和高频门控链。`DynamicRangeDb`、`Compression`、`Tone` 等没有正式算法字段的展示值继续不映射，避免把近似处理宣称为已生效。
- CPU4 启动链和运行时 `vf-command` 现在共享 `eq@autolive_cpu4_eq`、`hue@autolive_cpu4_hue` 固定标签，解决 IPC 命令成功但没有命中已安装滤镜的问题；新增回归测试确认动态更新目标与启动链一致。
- 视频首启声音失败时保留可用 mpv 画面；用户暂停后恢复会先读取当前 `time-pos`，再从该位置重新建立 FFmpeg PCM→PortAudio 会话。恢复失败或位置不可用时视频仍继续播放，并显示声音不可用，不回滚或伪造成功；真实无效设备 WPF 夹具覆盖首次失败、暂停、恢复和 fail-closed 文案。
- 本轮结果：Windows `222/222`、Media `126/126`；App 稳定分组 `57/57`、`VideoPlaybackAudioFallbackTests` `6/6`（其中声音故障暂停/恢复夹具单独 `1/1`）；App/Media/Windows `dotnet format --verify-no-changes --no-restore` 通过；启动脚本本地 Release x64 构建 `0` 警告/`0` 错误；同步夹具 `7` 个 JSON 全部通过。App 全量串行复跑在 WPF 多真实窗口收尾阶段超过两分钟无输出，未计入通过。未执行容器健康、人工页面点击、外部模型、真实远端 ZLMediaKit/RTMPS、有效声卡恢复成功、AkVirtualCamera/DirectShow、签名、目标 GPU 矩阵和长稳验收，Rust 端继续只读。

### v1.68 C# WPF 真实播放测试串行隔离与媒体池门禁收口（2026-09-05）

- `MainWindow.MediaPool.cs` 为媒体池导入、拖放、上移、下移、移除和清空统一补齐登录、关闭和导入忙碌门禁；导入探测/运行包失败在停止旧输出后，UI 改为投影 `_mediaPool.Snapshot` 的实际旧池，而不是用失败结果伪造新池，保持整批原子提交、顺序和旧池保护。
- `WpfTestApplicationHost` 使用有界 Dispatcher action、线程存活检查和串行门；真实视频夹具关闭前显式调用既有 `StopPlaybackCoreAsync`，WPF/WGC/媒体池/视频夹具共享的 App 测试程序集新增 `[assembly: DoNotParallelize]`，解决默认并行测试与共享 Dispatcher、环境变量、WPF HWND 和 mpv 资源的竞态。并行执行导致的 `70/72` 失败在串行执行下复验为 App `72/72`。
- `FfmpegPcmDecodePlan.TryValidateRealtimeConsumption` 对没有实时消费者的音频字段、未知时长淡出和采样率不一致 fail-closed；`MpvGpu83ShaderSnapshot`/`MpvVideoEffectSnapshot` 对 source fps、shader key 和已知未消费参数收紧白名单，防止 UI/快照字段被误标为已生效。
- 本轮复验：Windows `222/222`、Media `131/131`、App 稳定分组 `60/60`、App 全量串行 `73/73`，其中真实 `VideoPlaybackAudioFallbackTests` `6/6`。未执行容器健康、人工页面点击、外部模型、有效声卡恢复成功、真实 ZLMediaKit/RTMPS、AkVirtualCamera/DirectShow、签名、目标 GPU 矩阵和长稳验收，Rust 端继续只读。

### v1.69 C# 最终效果窗口任务栏呈现收口（2026-09-05）

- 保留独立且唯一的 `FinalEffectWindow` 播放窗口、`Owner=MainWindow`、mpv 视频子 HWND 和 WGC 顶层 HWND 绑定；仅将 `ShowInTaskbar` 从 `True` 改为 `False`，避免用户把同一 C# 进程的播放窗口误认为第二个桌面客户端。
- 新增 WPF 回归测试确认最终效果窗口不再创建第二个任务栏应用入口；不合并窗口、不新增播放器、不改变单实例 Mutex、登录授权、媒体池和输出生命周期。

### v1.70 C# 未接入操作入口明确化（2026-09-05）

- 顶部 `整理媒体`、`导入列表`、`保存配置`、`加载配置` 四个没有事件处理的视觉按钮改为明确禁用并提供边界提示；当前可用入口仍是媒体池中的 `导入媒体`/拖放，避免“导入文档”点击后静默无反应。
- 不新增配置/列表文档格式，不改变媒体池仅存在当前进程、最多 100 项、FFprobe 探测和登录授权门禁；若后续需要导入 PDF/DOCX 或播放列表，必须另立格式、路径权限和原子提交方案。

### v1.71 C# Rust 对齐的自动视频周期参数接线（2026-09-05）

- `GeneratedVideoEffectSnapshot` 保留旧 UI 投影和未接入字段语义，同时生成正式 `VideoEffectParams` 与 `AdvancedEffectParams`；生成规则对齐 Rust `sample_automatic_video_parameters` 的已验证范围。
- `MainWindow.Effects` 的完整 GPU83 路径传入当前周期高级参数，不再固定传 `AdvancedEffectParams.Default`；CPU4 仍只消费亮度、对比度、饱和度和色相四项，Original、登录、manifest、hash 和白名单门禁不变。
- 新增映射测试；App 全量串行测试 `74/74`、本机 Release x64 构建和格式检查通过。目标显卡矩阵、真实声卡、远端输出、下游虚拟摄像头和人工页面仍未验收；Rust 只读。

### v1.72 C# 媒体拖放与音频启动计划边界收口（2026-09-05）

- `MediaDropPayload.HasCandidateFiles` 现在复用现有媒体扩展名白名单，先拒绝空路径、目录形态、不支持扩展名和超限候选；`TryReadPaths` 仍只复制原生顺序，不做文件 I/O、FFprobe 或播放池写入，真实探测和原子提交继续由 `MediaImportCoordinator` 负责。
- `WindowsAudioPlaybackController.StartAsync` 现在在创建实际输出前校验计划声道与输出采样率；不一致直接返回 `invalid_plan` 并保持失败状态，避免声音处理开关改变但设备链路实际无声。没有新增依赖、播放器或 Rust 改动。
- 受影响复验：Windows 全量 `223/223`、Media 全量 `131/131`、App 全量串行 `79/79`，媒体拖放边界 `7/7`，音频控制器边界 `14/14`；App 显式 WPF GPU83 最终表面像素夹具 `1/1`，打包 FFmpeg 音频滤镜消费 `1/1`，格式检查和本机 x64 Release 构建 `0` 警告/`0` 错误通过。真实声卡、远端输出、下游虚拟摄像头、目标显卡矩阵、人工页面和长稳仍待验收；Rust 只读。

### v1.73 C# 版本化 JSON 媒体列表导入（2026-09-05）

- 顶部 `导入列表` 已接入单一 `RunImportAsync` 黄金路径；登录/设备授权、运行包校验、FFprobe、停止当前输出、整批原子提交、首项 `Ready` 和不自动播放语义保持不变。
- 列表格式复用既有 `VersionedJsonStore<T>`，当前只接受 `schema_version: 1`，业务数据仅允许 `data.items[].path`；路径必须是本地绝对媒体路径，拒绝相对路径、UNC/远程路径、URL、空值、未知字段、重复路径、不支持扩展名和超过 100 项的列表。
- 列表不信任或保存 FFprobe 元数据、效果参数、播放位置、凭据和 RTMP 地址；每个路径导入时仍由现有 `MediaImportCoordinator` 重新探测，任一项失败均保留旧播放池。
- 新增 Core 读取器与 Contracts DTO、Core/App 回归测试，并同步 C# 专项计划和状态文档；Core `81/81`、App `79/79`、本机 x64 Release 构建 `0` 警告/`0` 错误、相关格式检查通过。未修改 Rust、未新增第三方依赖。当前只实现“导入列表/ReplaceAll”，保存配置、加载完整配置和追加列表按钮仍不扩展为未定义契约。

### v1.74 C# 媒体命令串行与输出生命周期收口（2026-09-05）

- `RunImportAsync` 在文件选择/列表请求完成后取得共享 `_playbackCommandSerial`，并覆盖运行包校验、FFprobe、停止旧输出和原子媒体池提交；播放、停止、换源与导入不会在探测期间并发交错，取消或早退只释放实际取得的闸门。
- 最终效果窗口关闭改为复用 `StopMediaForMutationAsync`：成功路径按既有顺序停止 RTMP、虚拟摄像头、观察者、插话、PortAudio、mpv 和媒体池，并把实际池快照同步回主窗口；停止失败仍保留真实错误，不显示已释放假状态。窗口仍是同一 C# 进程的唯一独立播放窗口，任务栏入口保持隐藏。
- `WindowsRtmpAudioSession` 从共享 `FinalPcmBus` 派生解码计划、混音器和 PCM 泵的实际声道数，修复单声道总线被固定双声道链拒绝的问题；未提供共享总线的独立 RTMP 会话继续使用双声道默认值。
- 新增导入闸门、窗口关闭和单声道 RTMP 回归测试；Windows 全量 `224/224`。App 关闭窗口测试 `1/1`、两个真实 mpv 夹具隔离运行各 `1/1`；App 全量串行尝试为 `80/81`，唯一失败是顺序运行下真实 WPF/mpv 夹具的 90 秒宿主超时，不能记为全量通过。未修改 Rust、未新增第三方依赖；真实声卡、远端 ZLMediaKit/RTMPS、目标 GPU、虚拟摄像头下游、人工页面和长稳仍待验收。

### v1.75 C# 单实例进程门禁修复（2026-09-05）

- 真实冷启动复现确认旧 C# 命名 Mutex 门禁未阻止第二个 `GpAutoLive.exe`；新增 `WindowsSingleInstanceLease`，在 `%LocalAppData%\GpAutoLive\locks\csharp-instance.lock` 上持有 `FileShare.None` 文件句柄，进程退出或异常终止时由 Windows 自动释放句柄。
- `App.OnStartup` 先取得单实例租约，再取得跨客户端媒体/输出资源租约；第二次启动只返回并退出，不创建主窗口，也不影响已运行实例。锁文件不删除，避免清理与新启动之间的竞态。
- 新增同一路径二次获取拒绝、释放后重新获取回归测试；锁定 SDK 下测试 `1/1`，新版 Release x64 构建 `0` 警告/`0` 错误，冷启动 A/B 验证为 A 保持运行、B 已退出、进程数保持 `1`。
- 本轮未修改 Rust/Tauri、未新增第三方依赖、未删除生产代码；主控制窗口与最终效果窗口仍属于同一个 C# 进程，最终效果窗口继续作为单一 mpv/WGC 承载窗口。真实声卡、远端 ZLMediaKit/RTMPS、AkVirtualCamera 下游、人工页面和长稳仍按总计划验收。

### v1.76 C# 媒体池按钮与 RTMP 声音首帧门禁收口（2026-09-05）

- `MainWindow.MediaPool.SetMediaMutationButtonsEnabled` 在 WPF `ItemsSource` 尚未完成绑定的短暂窗口内，使用已验证范围内的 `MediaPoolService` 当前源索引作为按钮投影回退；对快照索引做池长度边界检查，避免陈旧索引越界。列表绑定完成后仍以用户实际选择为准，不改变播放池、排序或播放状态。
- `WindowsRtmpAudioSession.StartAsync` 不再把“分流泵任务已创建”当作声音启动成功；启动后在 3 秒有界预算内等待 `ForwardedFrames > 0`，泵错误、无 PCM 或取消都会返回稳定失败并执行有界停止，避免 UI 误报 RTMP 声音已消费。
- 新增/强化媒体池忙碌态与首帧门禁回归证据；Windows 全量 `226/226`、受影响目标测试 `28/28`、`dotnet format --verify-no-changes --no-restore` 和本机 Release x64 构建 `0` 警告/`0` 错误通过。未修改 Rust、未新增依赖或播放器；真实声卡、远端 ZLMediaKit/RTMPS、人工页面、AkVirtualCamera 下游、目标 GPU 矩阵和长稳仍待验收。
