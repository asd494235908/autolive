# 下一周期音轨预热与无阻塞切换实施方案

> 状态：Phase 0–2 已完成代码实施与静态/单元验证；Phase 3–4 的时钟观测、自动重对齐、UI 与清理已落地，目标 Windows 设备长测仍待完成
> 日期：2026-08-20
> 代码基线：`dev-2.0` / Phase 1 提交 `d8b8e19`；本文同步 Phase 2–4 当前工作区实现
> 适用范围：桌面端 React 调度、Rust/Tauri 音频生命周期、FFmpeg 流式处理、PortAudio 输出

## 1. 结论

用户提出的“20 多套声音预先创建，随机时直接选择”应落成以下结构：

- p01～p20 的预设定义继续在启动时全部加载；p21 保持现有排除规则，p22 仍只允许手动选择。
- 不创建 22 个常驻 FFmpeg 进程，也不把 22 条完整 PCM 音轨放入内存或写入缓存文件。
- 当前周期运行期间，立即抽样下一周期，并只预创建 **1 个下一周期候选**；默认在切换前约 4 秒启动 FFmpeg，短周期则立即启动。
- 任意时刻最多存在 **1 个当前解码任务 + 1 个候选解码任务 + 1 个 PortAudio 流**。
- 候选提前定位到下一次切换的目标媒体时间窗，生成有界 PCM 后停止前进并等待。
- 到切换点只做身份校验、时钟校验、30ms 交叉淡化和主总线指针切换；不得在切换点启动 FFmpeg、构造滤镜图或等待数秒预热。
- 候选失败、过期或没有覆盖当前画面时继续播放旧轨，不关闭 PortAudio，不立即回退 WebView。

这能达到“随机时直接选择已准备音轨”的目的，同时避免 22 个进程带来的 CPU、内存、线程、句柄和同步压力。

## 2. 为什么不能常驻预创建 22 条音轨

### 2.1 资源成本不可控

当前每个周期可以选择 1～4 个预设支路，实际播放的是这些支路经 `asplit → 分支滤镜 → amix → loudnorm` 得到的一条主总线。若把每个预设都做成独立常驻 FFmpeg：

- 单预设需要 22 个进程；多轨组合不是 22 种，而是 1～4 轨组合的数千种组合。
- 44.1kHz、双声道、f32 的 72 秒完整 PCM 单轨约 24.2MiB；22 条约 533MiB，尚未包含队列、滤镜和 FFmpeg 进程内存。
- 预设支路混合后的结果不能由 22 条最终响度归一化音轨简单相加替代，否则会改变 `amix` 后统一 `loudnorm` 的声音契约。
- 写入完整音轨文件会重新引入已取消的“生成并保存处理文件”路径。

### 2.2 预设已经预创建，缺的是执行结果预热

`runtime-parameter-scheduler.ts` 已静态定义全部预设及参数值。随机时的主要耗时不是“读取预设”，而是：

1. 创建 FFmpeg 子进程；
2. 初始化滤镜图和 `loudnorm`；
3. 等待候选 PCM 覆盖不断前进的画面；
4. 暂停旧任务并把候选写入 PortAudio 环缓。

因此应预热“下一周期组合的一条候选主总线”，不是复制 22 套运行时。

## 3. 当前问题与根因

### 3.1 候选启动时机太晚

当前周期到点后，React 才执行：

```text
抽样本周期
  → start_media_processing 更新声音流版本
  → sync_audio_output_source
  → Rust 创建候选 FFmpeg
  → 最多等待 5 秒
  → 校验追赶进度
  → 切换或继续旧轨
```

切换时刻同时承担创建、预热、追赶和提交，必然产生 1～数秒延迟。

### 3.2 `-re` 与“从当前位置追赶画面”互相冲突

实时 FFmpeg 使用 `-re` 后，候选 PCM 只能接近 1×速度产生。若 FFmpeg 启动和滤镜初始化耗时为 `D`，候选从当前画面位置开始解码，则经过 `T` 毫秒只能覆盖约 `T-D` 毫秒，而画面已前进约 `T` 毫秒。

现有 `candidate_required_prebuffer_samples` 要求候选覆盖“已等待时间 + 50ms”，在非零启动延迟下很难满足。这是反复出现以下错误的直接原因之一：

```text
候选音轨尚未追上画面
音轨预热超时，无有效 PCM
```

修复原则不是取消 `-re` 让 FFmpeg 无限制冲刺，而是提前知道下一次切换位置，直接预热该位置附近的 PCM 时间窗。

### 3.3 `ready` 是一次性布尔值，不代表提交时仍可用

`AudioMixerTask::wait_until_ready_with_reason` 只读取一次性 `ready` 原子值。候选就绪后画面继续前进，提交前没有持续维护“候选覆盖的媒体时间范围”。因此可能出现：

- 预热阶段已经 `ready=true`；
- 提交阶段画面又前进；
- 剩余 PCM 小于 50ms；
- `validate_commit_at_position` 或 `commit_at_position` 再次失败。

应把就绪条件改为实时覆盖范围：

```text
candidate_window_start_ms
candidate_buffered_end_ms
commit_target_ms
required_tail_ms
```

只有 `candidate_buffered_end_ms >= commit_target_ms + required_tail_ms` 才能提交。

### 3.4 改造前问题：当前任务和候选任务都直接写共享输出

Phase 2 改造前，每个 `AudioMixerTask` 都持有共享 PortAudio 输出槽，并由自己的混音线程写入环缓。切换需要依次暂停旧写入者、检查消费者、向同一环缓追加候选、替换任务、停止旧任务，形成较复杂的锁和背压关系；该直接写入路径现已删除。

最终结构应改为单一输出所有者：

```text
当前 FFmpeg 解码任务 ──► 有界 PCM 队列 ──┐
                                         ├─► audio_cycle_output 单线程
候选 FFmpeg 解码任务 ──► 有界 PCM 队列 ──┘       │
                                                    ▼
                                          SPSC f32 环形缓冲
                                                    │
                                                    ▼
                                          PortAudio callback
```

只有 `audio_cycle_output` 可以写 PortAudio 环缓；PortAudio callback 仍只读环缓并补零，不加 Mutex、不执行 FFmpeg、不访问 UI。

### 3.5 音画同步必须使用统一的媒体时间轴

源视频时长与音频流时长可能存在十几毫秒差异；视频元素循环与独立音频时钟长期运行后会累积漂移。当前实现不再依赖单个 `-stream_loop -1` 进程跨越媒体边界，而是在有限输入 EOF 后顺序重启 FFmpeg；候选身份仍必须包含循环序号，时间比较必须使用未回绕的绝对媒体时间：

```text
absolute_media_ms = loop_index * source_duration_ms + current_position_ms
```

视频仍是主时钟。音频实际送达时间由 callback 已消费 frame、采样率和 PortAudio 输出延迟换算，不能仅依赖一次 UI `currentTime`。

## 4. 目标与非目标

### 4.1 目标

- 周期切换点不启动 FFmpeg，不等待滤镜初始化。
- 默认随机池保持 p01～p20；p21/p22 规则不变。
- 继续支持每周期随机 1～4 个预设支路、等权混合和相邻周期不重复规则。
- 最终滤镜保持：各支路处理 → 等权 `amix` → `highpass=f=50` → 有限值护栏/`adenorm` → `loudnorm=I=-16:TP=-1.5:LRA=11` → `aresample` → f32 PCM。
- 候选失败时当前轨不断音；只有当前轨和硬件出口都不可用时才回退 WebView。
- 暂停、停止、跳转、换源、循环、参数变更都能取消过期候选并回收 FFmpeg/线程句柄。
- 连续长时间播放时音画偏差有可观测数据和明确上限。

### 4.2 非目标

- 不实现完整 DAW，不提供 22 条可编辑物理轨道。
- 不生成或保存 22 个处理后音频文件。
- 恢复 ASIO 的可见筛选入口，但只有 PortAudio 运行时真实枚举到 ASIO 输出设备时才启用；当前随包 DLL 未启用 ASIO，不扩展为 ASIO 运行库构建任务。
- 不改变 p21/p22 的产品规则。
- 不加入实时话术幻化、VAD/ASR/LLM/TTS。
- 本次不新增第三方依赖；复用现有 FFmpeg、`ringbuf`、PortAudio 和取消机制。

## 5. 目标状态机

### 5.1 周期状态

```text
Idle
  └─ 开启声音处理/开始播放 ─► Active(current=N)

Active(current=N)
  ├─ 抽样 N+1 ─► Preparing(next=N+1)
  ├─ 暂停/跳转/换源/配置变化 ─► Cancel next
  └─ 停止/关闭声音处理 ─► Stopping ─► Idle

Preparing(next=N+1)
  ├─ 目标时间窗 PCM 足够 ─► Ready(next=N+1)
  ├─ 失败/过期 ─► Active(current=N)，记录原因并重建最新候选
  └─ 取消 ─► Active(current=N)

Ready(next=N+1)
  ├─ 到期且身份/时间窗有效 ─► 30ms Crossfade ─► Active(current=N+1)
  ├─ 未覆盖当前目标 ─► 保持 N，短暂延后提交或重建候选
  └─ 过期/暂停/跳转/换源 ─► Cancel next
```

### 5.2 必须保持的不变量

1. FFmpeg 活跃 PID 数量始终 `<= 2`。
2. 当前任务、候选任务各最多 1 个。
3. PortAudio 流始终最多 1 个。
4. PortAudio 环缓始终只有一个生产者：`audio_cycle_output`。
5. 任何锁内都不等待 FFmpeg 退出、不 Join 线程、不等待 IPC。
6. 候选提交失败不能清空当前可播放 PCM。
7. 停止完成前必须取消并 Join 当前任务和候选任务，不能丢弃句柄。
8. 状态更新必须携带 `candidate_id` 和版本身份，旧结果不得覆盖新请求。

## 6. 数据与身份设计

### 6.1 `AudioCycleId`

候选使用单调递增的周期 ID，不使用预设 ID 组合充当任务身份：

```text
playback_generation
loop_index
source_media_reference
base_audio_stream_revision
audio_timeline_revision
audio_output_epoch
cycle_number
candidate_id
```

`candidate_id` 由前端创建并回传；Rust 同时校验其余字段，防止旧 IPC、旧 FFmpeg 或旧循环结果被提交。

- `audio_timeline_revision` 在手动跳转、停止、换源、倍速变化时递增，普通播放位置上报不递增。
- `audio_output_epoch` 在 PortAudio 设备、采样率、声道或内存环缓重开时递增。
- 不能只使用 `playback_generation`：手动跳转需要使音频候选失效，但不应连带判定已经生成的视频缓存失效。

### 6.2 候选配置

候选配置至少包含：

- `seed`
- `preset_ids`
- `weights`
- 主参数快照
- `audio_variants`（最多 4 条）
- `sample_rate_hz`（只允许 44100/48000）
- `playback_rate`
- 预计切换的绝对媒体时间
- 预热窗口起点与目标尾部余量

所有声音参数继续走 Rust 统一校验；不能因为是预热 IPC 就绕过 `AudioResearchParams::validate()`、未映射字段检查、有限值检查和数量上限。

候选配置在 prepare 阶段只保存在候选对象中，不得提前写入 `PlaybackCore`，也不得提前递增 `audio_stream_revision`。只有 commit 成功后，当前声音参数、`audio_variants` 和 revision 才作为一个事务切换；否则会出现“快照已经显示下一轮、实际仍播放上一轮”，并再次触发现有 revision 同步 Effect 创建重复候选。

### 6.3 有界预热窗口

建议首版常量：

| 参数 | 初始值 | 含义 |
|---|---:|---|
| `candidate_pre_roll_ms` | 250ms | 预计切换点之前保留的容错窗口 |
| `candidate_post_roll_ms` | 500ms | 预计切换点之后可提交/淡化的 PCM |
| `switch_crossfade_ms` | 30ms | 旧轨 1→0、新轨 0→1 |
| `minimum_commit_tail_ms` | 100ms | 30ms 淡化结束后仍必须保留的连续 PCM |
| `candidate_prepare_timeout_ms` | 5000ms | 只约束准备结果，不关闭当前轨 |
| `decoder_queue_blocks` | 8 | 复用现有有界队列原则 |

候选提交基础余量为 `30ms crossfade + 100ms post-fade tail = 130ms`，实际提交要求取 `max(130ms, 当前动态播放水位)`，最高不超过 750ms Scheduled 时间窗。上述值必须通过真实设备长测调整，不暴露为第一版 UI 配置。候选 PCM 上限按时间窗计算，不使用 1024KiB PortAudio 环缓容量充当候选缓存上限。

## 7. 调度时序

### 7.1 首轮

1. WebView 原声继续播放，保证用户立即有声音。
2. 开启 PortAudio 后准备首个当前周期候选。
3. 候选覆盖 `当前视频位置 + 输出延迟` 后提交。
4. PortAudio 确认开始消费后再静音 WebView。
5. 首轮提交成功后立刻抽样下一周期，并登记约 4 秒提前量的预热计划。

首轮失败时保留 WebView，不把 PortAudio 标记为永久不可用。

### 7.2 普通周期

假设当前为 N，周期时长为 P：

1. N 提交成功时立即确定 N+1 的 `seed/preset_ids/weights/参数快照`。
2. 根据视频绝对媒体时间、P 和 `playback_rate` 计算预计切换点。
3. 本地计时到“预计切换前 4 秒”时启动 prepare；周期不足 4 秒则立即 prepare。这个提前量先作为可测量默认值，后续只能依据真实预热 P95 调整。
4. FFmpeg 直接 seek 到 `预计切换点 - 250ms`，使用 `-readrate playback_speed × 1.1` 生成约 750ms 的候选 PCM；填满后由有界队列反压。
5. 候选缓冲达到目标窗口后由有界队列自然背压，不继续增长。
6. 到期时最终效果窗口只发送 `commit(candidate_id, live_position)`。
7. Rust 用同一份时钟快照计算实际可听目标，验证候选覆盖范围。
8. `audio_cycle_output` 在 30ms 内同时对旧轨降益和新轨升益，然后把新轨设为当前轨。
9. 旧 FFmpeg 在锁外取消并 Join；随后抽样并计划 N+2。

### 7.3 候选未就绪

- 不递增已生效轮次，不关闭旧轨，不清空 PortAudio 环缓。
- 在 UI 中显示“下一轮准备中/已延后”，而不是“PortAudio 失败”。
- 若候选仍在有效目标窗内，可在最多 500ms 的有界宽限期内重试提交。
- 超过目标窗或身份过期则取消候选，基于最新位置重新准备。
- 只有 FFmpeg 确认退出、当前轨也无 PCM 或 PortAudio 硬件停止时才进入 WebView 回退判断。

### 7.4 暂停、停止、跳转与换源

| 操作 | 当前轨 | 候选轨 | 恢复行为 |
|---|---|---|---|
| 暂停 | 暂停 callback 消费并冻结时钟 | 取消并 Join | 继续后按新目标重建 |
| 停止 | 立即静音/停止消费 | 取消并 Join | 清空周期状态，不自动重建 |
| 拖动进度 | 暂停/静音旧 PortAudio PCM，临时由 WebView 原声跟随新位置 | 立即取消 | 新位置稳定后重建当前轨，成功后再准备下一候选 |
| 换源 | 停止并 Join | 停止并 Join | 新 generation 从首轮流程开始 |
| 视频循环 | 以新 `loop_index` 重建身份 | 旧循环候选失效 | 使用绝对时间轴准备下一周期 |
| 参数/预设池变化 | 当前轨保持至新候选可用 | 旧候选取消 | 用最新配置重建 |

候选槽只有一个，事件优先级固定为：

```text
换源/跳转 > 视频循环边界 > 普通随机周期
```

随机周期与视频循环边界过近时，将“新循环 0ms”和下一随机配置合并成一个候选；不能让普通随机候选占住槽位，导致循环边界继续播放旧时间轴 PCM。

## 8. IPC 与模块边界

### 8.1 React

新增 `desktop/ui/src/audio-cycle-prewarm-coordinator.ts`：

- 只负责当前/下一周期状态、随机抽样时机、预计切换时间和候选 ID。
- 不直接处理 PCM、FFmpeg 或 PortAudio。
- 使用可注入时钟，单元测试不依赖真实 `setTimeout` 睡眠。

保留 `runtime-parameter-scheduler.ts`：

- 继续作为预设定义、随机抽样、权重和不重复规则的唯一事实源。
- 不加入 IPC 和 React 状态。

`App.tsx`：

- 只装配 coordinator、播放状态和 Tauri IPC。
- 将“抽样并立即应用”拆成“提交当前样本”和“准备下一样本”。
- `audioVariationCycle` 只在 Rust 确认 commit 成功后递增。
- `appendAudioCycleSnapshot` 只记录实际提交成功的 current；失败或取消的 next 不得伪装成已生效快照，也不得影响下一次相邻去重。
- 停止、暂停、换源和卸载时对称取消候选。
- 候选状态通过低频事件或现有状态快照展示，不增加高频轮询。

跨窗口时钟所有权：

- 主窗口负责随机抽样、周期计划和参数配置。
- 最终效果窗口拥有实际 `<video>` 元素，prepare/commit 边界由现有 BroadcastChannel 通知该窗口执行。
- commit 必须读取最终效果窗口当下的 `video.currentTime`；不能使用主窗口低频同步回来的 `mediaState.current_time` 代替。
- 100ms 定时器只检查本地截止时间，prepare 和 commit 各发一次 IPC，不增加高频状态 IPC。

### 8.2 Rust/Tauri

新增 `desktop/src-tauri/src/audio_cycle_switch.rs`：

- 拥有当前/候选任务状态机、身份校验、覆盖窗口、提交和取消。
- 是唯一知道“当前 + 候选”关系的模块。
- 不持有 Tauri `Window`，不反向依赖 UI。

调整 `audio_mixer.rs`：

- `AudioMixerTask` 只负责 FFmpeg 解码、滤镜后 PCM 和有界输出队列。
- 删除每个任务直接写 `AudioOutputSlot` 的职责。
- 就绪信息改为可查询的时间窗快照，不再使用一次性 `ready: bool` 作为提交依据。
- 明确区分首次/跳转后的 `CatchUpNow` 与普通周期的 `Scheduled` 准备策略；后者按固定目标时间窗就绪，不能继续使用“已等待时长 + 50ms”的追赶公式。
- 解码任务继续持有取消 token、FFmpeg PID、decoder/mixer JoinHandle，并提供幂等停止。

调整 `commands.rs`：

- IPC 边界只校验 DTO、调用状态机并转换结构化错误。
- 将现有 `sync_audio_output_source` 拆分/收敛为以下明确操作：
  - `prepare_audio_cycle_candidate`
  - `commit_audio_cycle_candidate`
  - `cancel_audio_cycle_candidate`
- 初始 PortAudio 启动可复用同一 prepare/commit 路径，不保留第二套候选算法。
- prepare 不修改当前配置/revision；commit 成功后才把候选配置提交到 `PlaybackCore`。
- 旧的隐式“audio_stream_revision 变化即同步并等待 5 秒”路径在迁移完成后删除。

调整 `autolive-portaudio-output`：

- 保持 callback 只读 SPSC 环缓并补零。
- 暴露现有 callback frame、DAC 时间和输出延迟快照供主时钟换算。
- 不新增第二个生产者，不在 callback 内增加锁或交叉淡化逻辑。

### 8.3 建议 IPC 契约

```text
prepare_audio_cycle_candidate(request)
  -> { candidate_id, state, target_absolute_ms, buffered_window, ffmpeg_pid }

commit_audio_cycle_candidate(request)
  -> { candidate_id, committed, active_cycle, reason, av_offset_ms }

cancel_audio_cycle_candidate(request)
  -> { candidate_id, cancelled }
```

错误码至少区分：

- `audio_candidate_stale`
- `audio_candidate_not_ready`
- `audio_candidate_window_missed`
- `audio_candidate_ffmpeg_failed`
- `audio_candidate_cancelled`
- `audio_output_inactive`

其中前三项是可恢复的切换结果，不应直接触发 WebView 回退。

## 9. 音画同步算法

### 9.1 单一主时钟

视频是主时钟。每次 prepare/commit 使用一份不可分割的时钟快照：

- UI 视频 `currentTime`
- `playback_generation`
- `loop_index`
- `playback_rate`
- Rust 最近播放位置和更新时间
- PortAudio callback 已消费 frame
- 实际采样率
- DAC/output latency
- 环缓待播 frame

### 9.2 提交目标

不得在不同时间分别读取上述字段后拼接。提交函数内部一次采集并计算：

```text
video_absolute_ms
audio_audible_absolute_ms
candidate_window_start_ms
candidate_window_end_ms
```

只有候选覆盖 `audio_audible_absolute_ms + minimum_commit_tail_ms` 才允许切换。

现有环缓中旧轨仍会先被 callback 消费，因此候选 PCM 的起点必须对应“视频位置 + 旧环缓待播时长 + 硬件输出延迟”。若后续改为原子清空旧环缓，则必须同步移除旧环缓待播项，禁止重复计入延迟。

### 9.3 漂移处理

第一阶段只做可证明的边界校正：

- 每次候选提交重新锚定视频绝对时间。
- 每次视频循环强制重建音频时间锚点。
- 记录 `av_offset_ms`，连续 3 次超过 80ms 才触发重对齐，避免抖动。
- 重对齐优先准备新候选并交叉淡化，不直接清空环缓制造静音。

不在首版实现复杂 PLL 或任意变速拉伸；只有长测证明边界校正仍不足时，再单独评估小范围 `aresample async`/补偿策略。

## 10. 分阶段实施

### Phase 0：回归测试与观测基线

实施状态（2026-08-20）：已完成固定时间窗 readiness、30ms 淡化加 100ms 淡化后尾部，并按动态播放水位将总提交余量提高到 130–500ms（极端值封顶 750ms）；已覆盖绝对媒体时间和配置 revision 原子提交测试。运行态继续复用 current/pending PID 与任务数。真实 CPU、内存、xrun 和 A/V 偏差基线仍需目标 Windows 设备长测，未以单元测试替代。

先写失败测试并补齐状态字段，不改变播放行为。

- Rust：复现“ready 后画面继续前进，提交只剩 32ms”的竞态。
- Rust：复现 `-re` 从当前位置启动后无法追上移动目标。
- Rust：记录 current/candidate PID、任务数、候选覆盖起止时间、提交目标、A/V 偏差。
- 前端：测试下一周期抽样、相邻不重复、取消过期 candidate ID。

验收：日志可以回答每次切换“准备了谁、覆盖哪里、为什么提交/拒绝、两个 PID 是否退出”。

### Phase 1：下一周期提前抽样和未来时间窗预热

实施状态（2026-08-20）：已完成。React 使用独立 coordinator 保存 current/next，N+1 预选后立即通过最终效果窗口发起 prepare；Rust 新增 `prepare_audio_cycle_candidate`、`commit_audio_cycle_candidate`、`cancel_audio_cycle_candidate`，Scheduled 候选使用 `-readrate playback_speed × 1.1` 和固定 750ms 时间窗，prepare 不修改当前参数/revision，commit 成功后才提交已验证配置并递增一次 revision。首次启动的 CatchUp 路径保留，且通过 pending token 防止旧等待误取新候选。

- 引入前端 coordinator。
- 当前周期提交后立即生成 N+1，但不改当前 UI 参数事实源。
- 默认到切换前约 4 秒才启动候选 FFmpeg；候选样本可以提前展示，但只有 commit 成功后才能升级为 current。
- 新增 prepare/cancel IPC。
- FFmpeg seek 到预计切换点前 250ms，只缓冲有界时间窗。
- Phase 1 曾暂时保留直接写输出的兼容提交路径；Phase 2 已将其删除。

验收：切换时刻的进程日志中不出现新的 FFmpeg spawn；任意时刻 PID `<=2`。

### Phase 2：单一混音线程与真实 30ms 交叉淡化

实施状态（2026-08-20）：已完成。`AudioMixerTask` 只向各自有界 PCM 队列生产数据；`audio_cycle_output` 按值持有唯一 `PortAudioOutput`，是 SPSC 环缓唯一生产者。提交需要 `max(130ms, 当前动态播放水位)` 的连续候选 PCM，原子追加 30ms 线性交叉淡化，成功后才切换 current，并在状态锁外停止、Join 旧任务。普通切换不清空旧环缓；短暂 commit 原子门禁阻止新 prepare/cancel 在“输出已切换、状态槽尚未替换”的间隙使候选失效，暂停/停止仍可立即打断。

- 解码任务改为写各自有界 PCM 队列。
- `audio_cycle_output` 成为 PortAudio 环缓唯一生产者。
- 提交在 PCM 样本域执行旧轨 `1→0`、新轨 `0→1` 的 30ms 等功率或线性 ramp；首版沿用已确认的线性 ramp。
- 旧任务停止和 Join 移到状态锁外。
- 删除双任务直接争用输出环缓的旧代码和未使用字段。

验收：切换无硬切、无双写、无候选 prime 背压；callback 路径保持无锁读取。

### Phase 3：统一绝对时间轴与循环校正

实施状态（2026-08-20）：代码与单元测试已完成，实机漂移验收未完成。输出线程以 callback 已消费 frame、实际采样率和输出延迟维护可听绝对媒体时间；UI 展示 `av_offset_ms`，连续 3 次超过 80ms 后触发现有源同步路径，带 10 秒冷却。循环、跳转、换源和配置代际校验继续拒绝旧候选。

- 引入 `loop_index + current_position` 的绝对媒体时间。
- prepare/commit 使用统一时钟快照和实时覆盖窗口。
- callback frame 参与实际音频消费位置计算。
- 循环、暂停、跳转、换源、倍速变化全部使旧候选失效。

验收：72 秒左右素材连续 50 轮后 A/V 偏差不累积；循环边界不出现数秒错位。

### Phase 4：错误分类、UI 与清理

实施状态（2026-08-20）：主体已完成。UI 已展示下一周期预设、状态、candidate ID、倒计时和 A/V 偏差；旧的 Web Audio PCM 写入 IPC、双任务直接写 PortAudio 和无调用者字段已删除。`sync_audio_output_source` 仍作为首次启动、拖动和偏差重对齐的兼容入口保留，不在未完成实机验证前删除。

- UI 展示“当前轮 / 下一轮 / 预热请求中 / 已接收待切换校验 / 延后原因”。
- 当前轮展示实际预设、seed、weights、提交时间和实际出口；下一轮展示候选预设、倒计时、candidate ID、预热耗时、ready 提前量和取消/失败原因。
- 可恢复候选错误不再显示成“PortAudio 已回退 WebView”。
- 收敛旧 `sync_audio_output_source` 隐式等待；仅保留首次启动、拖动和偏差重对齐入口，删除重复 ready 布尔值和已无调用者的辅助函数。
- 更新架构、参数契约和原音频方案文档。

验收：停止/暂停按钮立即生效；状态文案与实际输出一致；没有未使用代码、重复路径或遗留调试日志。

## 11. 测试计划

### 11.1 TypeScript 单元测试

- p01～p20 默认池不变；p21/p22 规则不变。
- 当前只有一个预设时允许连续重复。
- 多预设时 N 与 N+1 不重叠，N+2 可重新出现 N。
- 下一周期在当前周期提交后生成，切换到点不重新抽样。
- 候选失败/取消不改变 current，也不参与下一轮的“相邻不重复”基准。
- pause/stop/seek/source change 会取消候选并拒绝旧回调。
- 手动重新随机创建新 candidate ID，旧结果不能覆盖。
- 周期提交失败时已生效轮次不增加，当前参数不被候选状态污染。

### 11.2 Rust 单元/集成测试

- 候选覆盖窗口而非一次性 ready 决定是否可提交。
- 预计目标前 250ms、后 500ms 的窗口可容忍定时器抖动。
- 总余量低于 `max(130ms, 当前动态播放水位)` 时拒绝，达到该阈值时允许。
- stale generation/loop/revision/candidate ID 全部拒绝且保留当前轨。
- current + candidate 任务上限为 2；第三个 prepare 先取消并 Join 旧候选。
- 停止、取消、FFmpeg 失败、队列断开均可 Join，无孤儿 PID。
- 交叉淡化首尾增益、总帧数和有限值正确。
- PortAudio 环缓只有单一生产者，callback 欠载只补零。
- 44.1kHz/48kHz、1～4 支路、循环边界和倍速路径均覆盖。

### 11.3 本地人工验收

测试矩阵：

- 采样率：44100、48000。
- 支路数：1、2、4。
- 周期：固定 5 秒、5～10 秒随机。
- 控制：播放、暂停、继续、停止、拖动、换源、开关 PortAudio。
- 设备：默认 MME/WASAPI 可用设备；ASIO 入口可见，只有运行时真实枚举到设备时才纳入输出验收。

至少执行：

1. 连续 50 轮周期切换，记录每轮 A/V 偏差、xrun、underrun、PID 和内存。
2. 连续 30 分钟播放，确认无持续漂移、无静音、无孤儿 FFmpeg。
3. 切换期间连续执行暂停/停止各 20 次，确认 200ms 内进入静音/暂停状态。
4. 切换前后拖动进度和触发视频循环，确认旧候选不会提交。
5. 人工听辨 30ms 切换无电流声、爆音和数秒空白。

## 12. 验收门槛

- [x] 代码路径保证切换点前候选已创建，commit 不启动 FFmpeg；实机日志抽查仍待长测。
- [ ] 活跃 FFmpeg PID 始终不超过 2，停止后 2 秒内归零（代码有上限和 Join；仍需实机确认）。
- [x] 当前/候选解码任务上限为 2，PortAudio 输出线程和流上限为 1。
- [x] 可恢复的候选未就绪不关闭当前轨、不回退 WebView。
- [ ] 正常周期切换无超过 100ms 的无声段；目标为 30ms 连续淡化。
- [ ] 稳态 A/V 偏差目标 `|offset| <= 50ms`，连续 30 分钟不超过 80ms 且不持续增长。
- [ ] 50 轮内无 `candidate_not_caught_up` 风暴、无环缓背压导致永久失声。
- [ ] 暂停/停止时声音同步停止，恢复时不提交旧候选。
- [x] 1～4 支路仍执行等权 `amix` 和最终 -16 LUFS 管线。
- [x] 没有新增处理音频文件或 22 条完整 PCM 常驻缓存。

验收阈值必须结合本地设备测量；若硬件固有输出延迟超过阈值，应单独记录实际 latency，不能把硬件延迟误判为调度漂移。

## 13. 文件改动清单

| 文件 | 计划改动 |
|---|---|
| `desktop/ui/src/runtime-parameter-scheduler.ts` | 保持预设/抽样事实源；仅补充可复用的 next-cycle 测试入口 |
| `desktop/ui/src/audio-cycle-prewarm-coordinator.ts` | 新增纯调度状态机、候选 ID、预计目标时间和取消语义 |
| `desktop/ui/src/audio-cycle-prewarm-coordinator.test.ts` | 新增可控时钟和周期竞态测试 |
| `desktop/ui/src/App.tsx` | 装配 prepare/commit/cancel；只在提交成功后更新生效轮次 |
| `desktop/src-tauri/src/audio_cycle_switch.rs` | 新增当前/候选生命周期和时间窗提交状态机 |
| `desktop/src-tauri/src/audio_cycle_output.rs` | 唯一 PortAudio 所有者和生产者；执行初始 prime、正常供给、30ms crossfade、暂停/停止和 callback 时间轴 |
| `desktop/src-tauri/src/audio_mixer.rs` | 解码任务改为有界 PCM 生产者；删除直接写共享输出职责 |
| `desktop/src-tauri/src/commands.rs` | 新增明确 IPC，删除迁移后的隐式同步等待路径 |
| `desktop/src-tauri/src/main.rs` | 注册新增 prepare/commit/cancel Tauri 命令 |
| `desktop/src-tauri/src/media_engine.rs` | 复用现有音频参数校验和滤镜图构建，不复制契约 |
| `desktop/crates/autolive-portaudio-output/src/lib.rs` | 保持 callback 简单；按需补充时钟快照，不加入业务状态机 |
| `桌面客户端架构.md` | 同步单一混音线程、下一候选和视频主时钟结构 |
| `媒体参数范围与默认值.md` | 记录内部预热时间窗/采样率边界，不把内部值误写为 UI 缓冲参数 |
| `产品需求文档.md` | 明确普通声音下一周期预热不是实时话术候选 |
| `系统架构总览.md` | 补充桌面执行面的 current/next 滚动候选链路 |
| `长任务开发总计划.md` | 增加分阶段实施和长时间稳定性验收项 |
| `docs/superpowers/plans/2026-08-18-声音预设多轨随机合并与实时音频架构计划.md` | 将“预加载”章节链接到本方案并更新实施状态 |

## 14. 本地验证命令

实施阶段按受支持配置在本机执行，不上传服务器构建：

```powershell
cd E:\aotlve\desktop\ui
npm run lint
npm run test
npm run build

cd E:\aotlve\desktop\src-tauri
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo check --workspace --all-targets --all-features
```

若 PortAudio 原生 Feature 不能与 `--all-features` 组合，按仓库实际支持的 Windows x64 Feature 分别执行并记录原因；不得把未运行项写成通过。

## 15. 实施顺序与回滚点

1. 先完成 Phase 0，保存当前错误率、预热耗时、CPU、内存和 A/V 偏差基线。
2. Phase 1 单独提交；若失败，可关闭新 coordinator，恢复当前到点创建候选路径。
3. Phase 2 单独提交；保留编译期内的旧输出适配仅到本阶段验证完成，随后删除，不长期维护双主路径。
4. Phase 3 单独提交；循环/时钟变更必须通过长测后再进入 main。
5. Phase 4 清理旧代码和同步文档，执行完整本地门禁。

每阶段只在 `dev-2.0` 当前工作区开发，不使用 worktree，不启动远程构建。进入下一阶段前必须审查 PID/线程/锁/队列所有权和取消路径。

## 16. 最终产品行为

用户看到的仍是“从 20 多套声音预设中，每周期随机选择 1～4 套并混成一条输出”。内部行为改为：当前轮播放时下一轮已经在后台准备好；随机切换只提交已准备候选，不在切换时临时创建音轨。这样既保留随机效果，也把 FFmpeg 启动和滤镜初始化从实时切换路径移走。
