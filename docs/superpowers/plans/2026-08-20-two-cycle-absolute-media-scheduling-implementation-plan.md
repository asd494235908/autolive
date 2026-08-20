# N+2 预选与绝对媒体时间统一调度实施方案

> 状态：已实施，待 Windows 实机长时验收
> 日期：2026-08-20
> 代码基线：`main` / `d512f11`
> 前置方案：`2026-08-20-prewarmed-audio-cycle-switching-implementation-plan.md`
> 适用范围：桌面端普通声音处理、视频运行时参数、最终播放窗口、Rust/Tauri 音频候选生命周期、PortAudio 输出

## 0. 实施结果（2026-08-20）

- 已新增纯 TypeScript 两周期计划器，声音和视频均只保留 N+1/N+2 两个轻量计划；N+2 不含 candidate ID、PCM、PID 或 Rust 任务。
- 最终播放窗口发布带 generation/source revision/epoch/sequence 的绝对视频媒体时钟；主窗口只使用该时钟盖目标和判断到期，`Date.now()` 不再参与周期目标计算。
- 跨窗口音频 prepare 已删除旧 `target_at_ms`，只接受 `target_absolute_position_ms`；最终窗口不再进行“剩余墙钟时间 → 媒体时间”的二次换算。
- 已增加独立/联动模式。联动范围取声音和视频范围交集；无交集时拒绝启用。联动目标到期而音频候选未就绪时整轮保持旧效果。
- Rust 仍只接收 N+1，保持 current + pending 最多两个 FFmpeg 音频任务；循环完成命令增加 generation + target loop 幂等校验。
- PortAudio 环缓待播量和硬件输出延迟按 playback rate 从墙钟时间换算成媒体时间后再参与提交位置校正；callback 数据路径未增加业务逻辑或锁。
- 同步/提交请求现已携带 generation、循环代次、视频时长、单轮位置和绝对位置；FFmpeg seek 使用单轮位置，PortAudio 时间线始终使用未回绕绝对位置，跨 72–75 秒循环不再丢失整轮。
- 后端在替换 pending 前校验 generation/revision；旧 revision 错误会删除旧 candidate，读取最新快照后用新 candidate ID 重建。过早 commit 返回可重试错误并保留已预热候选。
- 播放水位由固定 50ms 改为 `max(200ms, 8 × callback 帧时长)`，上限 500ms 且不超过环缓容量；1024 KiB 仍只是默认内存容量。PCM 恢复保留旧 current/环缓，先预热 pending，成功后再交叉淡化替换。
- 声音处理播放期间 PortAudio 自动启动：WebView 保持原声，CatchUp PCM 覆盖当前绝对画面位置和动态水位后自动接管；UI 不再提供硬件出口开关，设备、内存缓冲、测试音和运行诊断继续保留。
- CatchUp 预热与提交统一使用外层 `preparation_started_at` 单调时钟；提交前按最新画面、环缓和 DAC 延迟每 10ms 重新计算覆盖，最长 5 秒。`audio_mixer_candidate_not_caught_up` 属于可重试错误，日志包含需要缓冲、实际缓冲、缺少时长和安全水位，不关闭 PortAudio。

实现采用“最终窗口发布唯一事实时钟、主窗口维护两周期纯队列”的最小装配方式；目标时间和到期判断仍完全来自最终窗口真实视频元素，不使用定时器累计时间。

## 1. 结论

本次改造采用“预选两轮、只预热一轮、视频时钟唯一”的结构：

- 当前生效轮为 N；前端同时保存 N+1、N+2 两个未来计划。
- N+1、N+2 都提前确定周期长度、目标绝对媒体时间、随机 seed、预设/参数和权重。
- 只有 N+1 可以进入 Rust 并启动候选 FFmpeg；N+2 永远只是轻量元数据。
- 任意时刻最多保留 1 个当前 FFmpeg、1 个 N+1 候选 FFmpeg 和 1 个 PortAudio 流。
- 所有到期判断均以最终播放窗口的未回绕视频绝对媒体时间为准。
- `Date.now()`、`setInterval` 和消息到达时间只负责唤醒检查，不参与计算目标媒体时间。
- 声音、视频可选择独立周期或联动周期。联动模式共享同一个目标；独立模式各自拥有 N+1/N+2 队列。
- PortAudio callback frame、实际输出延迟和 A/V offset 继续负责“音频真正听到哪里”的运行时校正，不替代视频主时钟。

本方案不预创建 20 多个 FFmpeg，不创建 N+2 PCM，不扩大进程、线程、句柄和环缓数量。

## 2. 功能边界

### 2.1 本次目标

1. 将声音和视频的未来计划深度统一为 2：N+1、N+2。
2. 消除周期目标对墙上时间的依赖。
3. 为声音/视频提供“独立周期”和“联动周期”两种明确语义。
4. 保持现有 PortAudio 单一输出所有者、30ms 双向 Gain ramp 和最多两个音频任务的资源边界。
5. 继续用 callback 消费帧、硬件延迟和 A/V offset 发现并修正长期漂移。

### 2.2 非目标

- 不预创建全部声音预设对应的 FFmpeg 或完整 PCM。
- 不为 N+2 启动 FFmpeg、分配 PCM 队列或创建 Tauri 后台任务。
- 不引入新的音频库、调度库或状态管理库。
- 不实现实时话术幻化、speech-to-speech 或候选话术换轨。
- 不在 PortAudio callback 中加入锁、随机抽样、日志、IPC 或业务判断。
- 不承诺视频处理尚未准备完成时仍能做到样本级音画效果原子切换；本次先保证两者使用同一媒体目标和明确的成功/失败状态。

## 3. 当前实现差距

### 3.1 声音只有 N+1，视频只有一个下一周期数值

当前声音使用单个 `nextAudioCyclePlanRef`，提交后才抽样新的下一轮。视频使用 `nextVideoPeriodMsRef`，到期时才抽样参数和下一个周期。两者都没有完整 N+2 参数快照。

这会造成：

- 下一轮提交后才开始决定再下一轮；短周期时留给准备的时间不足。
- UI 展示的“下一周期”不是完整的双层未来队列。
- 声音和视频没有可复用的共同目标模型。

### 3.2 主界面用墙上时间调度

改造前的协调器保存 `targetAtMs`，主界面用 `Date.now()` 判断 prepare/commit；最终播放窗口再把剩余墙上时间换算为视频绝对位置。

当主线程卡顿、窗口后台节流、消息排队或播放倍速变化时，这种二次换算会把目标推迟或提前。目标必须直接表达为：

```text
absolute_media_ms = loop_index * source_duration_ms + current_video_position_ms
```

### 3.3 音频与视频周期没有显式同步模式

两个范围分别随机时，只能保证各自到达自己的目标，不能保证同一时刻变化。若产品要求声音与视频同时变化，必须由同一个联动计划生成同一个 `target_absolute_position_ms`。

### 3.4 现有 A/V 校正不能代替正确调度

callback frame、PortAudio 输出延迟和 A/V offset 能判断实际听到的音频位置，但它们是运行时观测和偏差修正手段。若目标本身由错误的墙上时间生成，校正只能不断追赶，不能消除根因。

## 4. 目标架构

```text
主界面
  ├─ 抽样声音 N+1 / N+2 元数据
  ├─ 抽样视频 N+1 / N+2 元数据
  └─ 将未来计划安装到最终播放窗口
                  │
                  ▼
最终播放窗口（唯一视频时钟所有者）
  ├─ 计算并维护单调 absolute_media_ms
  ├─ N+1 目标盖章后立即触发一次 prepare
  ├─ 在媒体时间跨过目标时触发 commit/apply
  └─ 联动模式对音频和视频使用同一目标 ID
                  │
                  ▼
Rust/Tauri
  ├─ current FFmpeg（最多 1）
  ├─ N+1 candidate FFmpeg（最多 1）
  ├─ N+2：不存在 Rust 任务
  └─ audio_cycle_output → SPSC → PortAudio callback
                                      │
                                      ▼
                       callback frame + DAC latency
                                      │
                                      ▼
                              A/V offset 校正
```

职责必须保持单向：

- 主界面负责随机抽样和用户配置。
- 最终播放窗口负责时钟、到期判断和跨窗口调度事件。
- Rust 负责实际音频候选、资源上限、提交校验和硬件输出。
- PortAudio callback 只消费样本并补零。

## 5. 数据模型

### 5.1 视频时钟快照

新增或收敛为一个明确的时钟契约：

```ts
type FinalPlaybackClock = {
  playbackGeneration: number;
  sourceRevision: number;
  clockEpoch: number;
  clockSequence: number;
  loopIndex: number;
  durationMs: number;
  positionMs: number;
  absolutePositionMs: number;
  playbackRate: number;
  paused: boolean;
};
```

约束：

- `absolutePositionMs` 只能由最终播放窗口中的真实视频元素和循环序号生成。
- 同一 `clockEpoch` 内 `clockSequence` 和绝对媒体时间必须单调不减；接收方拒绝乱序消息。
- seek、换源、重新打开最终播放窗口或无法连续解释的时间跳变必须递增 `clockEpoch`。
- 循环播放只增加 `loopIndex`，正常循环不重置绝对时间。
- `complete_playback_loop` 必须携带 generation 和目标 `loopIndex` 做幂等设置，不能继续无条件 `+1`。
- 字段必须是有限数、安全整数并有合理上限。

### 5.2 通用目标

```ts
type MediaCycleTarget = {
  planId: string;
  sequence: number;
  targetAbsolutePositionMs: number;
  periodMediaMs: number;
  playbackGeneration: number;
  sourceRevision: number;
  clockEpoch: number;
  plannedFromClockSequence: number;
};
```

周期单位是“视频媒体时间毫秒”。例如播放倍速为 2× 时，10 秒媒体周期约在 5 秒墙上时间后到达；目标本身不改变。

### 5.3 声音与视频计划

```ts
type PlannedAudioCycle = MediaCycleTarget & {
  kind: 'audio';
  seed: number;
  presetIds: string[];
  weights: number[];
  parameterSnapshot: AudioResearchParams;
};

type PlannedVideoCycle = MediaCycleTarget & {
  kind: 'video';
  seed: number;
  parameterSnapshot: RuntimeVideoParams;
};

type AudioCandidateExecution = {
  planId: string;
  candidateId: number;
  baseAudioStreamRevision: number;
  status: 'preparing' | 'ready' | 'committing' | 'discarded' | 'committed';
};
```

未来计划和物理候选必须分离。N+2 没有 `candidateId`、`baseAudioStreamRevision` 或执行状态；它晋升为 N+1 时，才从最新已提交快照绑定 `baseAudioStreamRevision` 并创建 `AudioCandidateExecution`，否则会因 N+1 提交后 revision 增加而立即过期。

### 5.4 队列约束

```text
audioFuturePlans.length <= 2
videoFuturePlans.length <= 2
preparedAudioCandidateCount <= 1
activeAudioFfmpegCount <= 2
```

不创建通用无限队列，不持久化 PCM，不把 N+2 发送到 Rust。

## 6. 独立周期与联动周期

### 6.1 独立周期

- 保留现有声音最小/最大秒和视频最小/最大秒。
- 声音、视频分别生成 N+1/N+2。
- 两条队列都以同一个最终视频绝对媒体时钟判断到期。
- 只能承诺声音按声音目标、视频按视频目标准时，不能承诺两者同时变化。

### 6.2 联动周期

- 新增“声音与视频联动周期”开关。
- 不增加第三套周期输入，也不覆盖声音/视频原范围。共享范围取两者交集：`sharedMin = max(audioMin, videoMin)`、`sharedMax = min(audioMax, videoMax)`。
- 没有交集时拒绝开启联动并显示明确错误，不能静默采用其中一套范围。
- 每个未来轮次只抽样一个 `MediaCycleTarget`，声音与视频 payload 共享其 `planId`、`sequence` 和 `targetAbsolutePositionMs`。
- 音频 prepare 可提前发生，视频也可提前准备；到目标时两者使用同一个目标触发提交。

例如声音为 5～10 秒、视频为 8～15 秒，联动共享范围为 8～10 秒。联动的定义是“共享目标”，不是把四个输入框永久合并；关闭联动后继续使用原来的两个独立范围。

### 6.3 联动失败语义

首版采用“同目标、到点前预检、失败不阻塞”的策略：

- 音频候选和视频参数均已 ready：在共同目标发起音频提交，确认提交后立即应用同一计划的视频参数。
- 任一路在目标前预检未 ready：本轮两种效果都保持旧值，计划标记过期，不做迟到硬切。
- 已通过预检但提交时发生不可预见失败：保持能够保持的旧效果，记录音频/视频各自结果并从实际后端快照重建计划，不能假装原子回滚。
- 到点判断不等待 FFmpeg；提交结果通过异步事件返回，最终播放窗口不得冻结画面。

若未来产品要求严格原子切换，必须先为视频路径增加可提交候选和明确回滚协议，不能在 UI 线程上等待实现。

## 7. N+1 / N+2 滚动规则

### 7.1 初始化

1. 主界面抽样 N+1/N+2 的周期和参数 payload，但不生成绝对目标。
2. 主界面将两个 payload 一次性安装到最终播放窗口。
3. 最终播放窗口在接收计划的同一时刻读取真实 `FinalPlaybackClock` 并盖上目标：`N+1.target = clock.absolutePositionMs + N+1.periodMediaMs`。
4. 最终播放窗口设置 `N+2.target = N+1.target + N+2.periodMediaMs`，再返回安装确认和两个绝对目标供主界面展示。
5. 正常滚动时最终窗口继续持有队列；主界面只追加新的 N+2 payload，不用墙上时间推算目标。

声音预设相邻不重复规则保持：N 与 N+1 不重复，N+1 与 N+2 不重复；因此 N 的预设允许在 N+2 再次出现。可选池只有一个时允许连续重复。

### 7.2 预热

- 最终播放窗口为 N+1 盖上绝对媒体目标后立即发送一次 prepare，不再等待固定墙钟预热窗口。
- Scheduled FFmpeg seek 到目标前 250ms，使用 `-readrate playback_speed × 1.1` 填充固定 750ms PCM；达到有界上限后由 channel 反压，因此提前启动不会无限增长内存或扩大到第三个进程，同时避免严格 1× 在调度抖动或加速播放时发生 PCM 欠载。
- 暂停会取消并 Join pending；恢复后只要目标仍合法，就重新绑定当前 revision 并立即 prepare。
- N+2 即使距离目标很近，也必须先成为 N+1 后才能 prepare。
- prepare 身份包含计划、播放代次、源版本和时钟 epoch；重复消息幂等。
- 1～4 秒短周期仍可能小于 FFmpeg 实际初始化时间，但立即 prepare 会提供当前结构下的最大准备时间；N+2 预选仍不会突破最多两个进程的限制。

### 7.3 到期与滚动

当最终窗口观测到“视频绝对媒体时间 + 环缓实际待播时长 + DAC 延迟”覆盖 N+1 目标：

1. 对 N+1 发出一次 commit/apply。
2. 后端若判断仍未真正到点，保留同一个已预热候选并回到 `prepared`，下一次检查重试；版本过期或候选已销毁时不得重试旧 revision。
3. N+2 原样提升为新的 N+1，目标时间不能重新抽样。
4. 新 N+1 从最新后端快照绑定 `base_audio_stream_revision` 并生成新的 candidate ID。
5. 以新的 N+1 为基准抽样一个新 N+2 payload，由最终窗口设置 `new N+2.target = new N+1.target + period`。
6. 旧 current/候选完成切换或取消并 Join 后，才允许新的 N+1 进入 Rust prepare。

这样切换点不会临时决定“下一轮是什么”，也不会让 N+2 提前消耗 FFmpeg。

## 8. 最终播放窗口调度协议

### 8.1 移除墙上时间目标

将跨窗口消息中的：

```ts
target_at_ms
```

替换为必填：

```ts
target_absolute_position_ms
playback_generation
source_revision
clock_epoch
clock_sequence
plan_id
sequence
```

禁止保留“若绝对目标缺失则使用 `Date.now()` 推算”的兼容分支。非法或旧版本消息直接拒绝并记录一次结构化原因。

### 8.2 调度位置

- 计划队列由纯 TypeScript 模块维护，便于单元测试。
- 队列的运行态所有者、目标盖章和 due/prepare 判断装配在最终播放窗口，因为只有它持有真实视频元素；主界面只保存 UI 镜像。
- 可使用低频定时器、`timeupdate`、`seeked`、`ratechange`、`play`、`pause` 和循环事件唤醒检查。
- 每次检查必须重新读取视频绝对媒体时间；不得用定时器累计值代替。
- 不向 Rust 或主界面高频发送时钟 IPC。只在初始化、状态变化、目标跨越和诊断采样时发消息。

### 8.3 跨越判断

检查逻辑使用区间跨越，而不是要求命中某一毫秒：

```text
previous_absolute_position_ms < target_absolute_position_ms
current_absolute_position_ms >= target_absolute_position_ms
```

这能容忍 100ms 检查间隔和短暂 UI 卡顿。若一次跨过多个目标，按 sequence 顺序将已错过目标标记 expired，只允许最新合法 N+1 执行一次，不能并发补跑多轮 FFmpeg。

## 9. Rust 与 FFmpeg 资源约束

### 9.1 IPC 校验

`prepare_audio_cycle_candidate` 和 `commit_audio_cycle_candidate` 统一校验：

- `target_absolute_position_ms` 为有限、安全、非负媒体时间。
- `target > current_absolute_position_ms` 才能 prepare。
- 目标范围不得超过允许的未来周期调度上限；立即 prepare 不放宽现有 60 秒 Rust horizon。
- `playback_generation/source_revision/clock_epoch/plan_id/candidate_id` 全部匹配；`clock_sequence` 必须不早于后端最后接受的同 epoch 视频观测。
- N+2 不存在 Rust DTO，也没有 prepare 命令。

### 9.2 任务上限

Rust 状态继续只保留：

```text
current: Option<AudioMixerTask>
pending: Option<AudioMixerTask>
```

新 prepare 到来时：

- 相同 candidate ID：幂等返回现有状态。
- 不同 N+1：先校验 generation/revision；校验通过后取消旧 pending，在状态锁外等待 FFmpeg 退出并 Join，再启动新 pending。
- current 不因候选失败而停止。
- commit 期间使用现有原子门禁，禁止 prepare/cancel 破坏正在进行的交叉淡化。
- stop 必须取消 current/pending、确认进程退出并回收线程句柄。

任何日志和状态快照中的实际任务数不得超过 2，活跃 FFmpeg PID 不得超过 2。计数必须覆盖“已 spawn 尚未装入 pending”“已取出正在 crossfade”“已提交尚未替换 current”等局部变量过渡态，不能只数两个状态槽而短暂漏报。

### 9.3 N+2 禁止事项

不得为 N+2：

- spawn FFmpeg；
- 创建 PCM channel；
- 写 PortAudio 环缓；
- 创建 Rust async/blocking task；
- 占用 PID、JoinHandle 或候选槽；
- 进行 FFmpeg 滤镜图初始化。

N+2 只是一份可序列化的小型参数快照。

## 10. PortAudio 与 A/V 运行时校正

保留现有计算链：

```text
callback_pcm_frames_total
  → 按实际 sample_rate 和 playbackRate 换算已消费媒体时长
  → 将环缓等待与 outputBufferDacTime - currentTime 乘以 playbackRate，换算为媒体时间
  → 得到 audible_audio_absolute_position_ms
  → 与 final_video_absolute_position_ms 比较
  → av_offset_ms
```

规则：

- PortAudio callback 仍只读 SPSC、写输出、补零和更新原子计数。
- 不用主界面计划目标替代 callback 实际消费位置。
- 环缓等待和硬件输出延迟是墙上时间，加入绝对媒体位置前必须乘以 `playbackRate`；0.5×、1×、2× 分别测试，不能沿用当前未缩放行为。
- A/V offset 连续 3 次超过 80ms 才触发重对齐，保留现有 10 秒冷却，避免抖动。
- 重对齐生成新的时钟/音频代际并使旧候选失效；不得直接清空当前轨制造静音。
- 周期切换与 A/V 重对齐共用同一串行音频操作门，避免同时创建两个 pending。
- 状态轮询维持低频；高频 callback 指标只存原子值，不通过 IPC 每帧上报。

## 11. 生命周期与失效规则

| 场景 | 元数据队列 | N+1 FFmpeg | 时钟处理 |
|---|---|---|---|
| 正常播放/循环 | 保留并滚动 | N+1 目标盖章后立即创建 | 循环只增加绝对媒体时间 |
| 暂停 | 保留 N+1/N+2 | 取消仍在生产的 pending；恢复时按需重建 | 媒体时间冻结 |
| 继续 | 复用仍合法目标 | 立即 prepare | 从同一绝对位置继续 |
| seek | 清空并重新抽样两轮 | 取消并 Join pending | `clockEpoch + 1` |
| 换源/重新导入 | 清空 | 停止并 Join current/pending | 新 playback generation/source revision |
| 停止 | 清空 | 停止并 Join current/pending | generation 结束 |
| 倍速改变 | 目标媒体时间保留；重新快照声音配置 | 取消旧 pending 后重建 | 目标不按墙上时间重算 |
| 周期范围/联动模式改变 | 从当前视频时钟重建两轮 | 取消旧 pending | `plan generation + 1` |
| 预设池改变/手动随机 | 重建声音两轮 | 取消旧 pending | 视频队列按模式决定是否重建 |

所有异步结果在写状态前再次检查 plan generation 和各 revision。旧的未提交结果只能丢弃；但如果 Rust 返回 `committed=true`，即使前端 plan ID 已过期，也必须接受实际后端快照并基于新 revision 重建计划，不能让 UI 与真实 PortAudio 状态分裂。

## 12. UI 行为

- 增加“声音与视频联动周期”开关。
- 独立模式显示现有声音范围、视频范围。
- 联动模式继续显示原有两套范围，并只读展示计算出的有效交集；没有交集时开关不能生效。
- 声音和视频分别展示 N+1、N+2：周期长度、目标绝对媒体时间、seed、预设/参数摘要。
- N+1 声音额外展示 `planned/preparing/ready/committing` 和 candidate ID。
- N+2 明确标记“仅预选，未创建 FFmpeg”，避免误导用户。
- 联动模式用同一目标编号展示声音和视频，失败时分别显示结果。
- 不在 React render 中执行随机、IPC 或时间推进；状态只由事件和纯 reducer 更新。

## 13. 代码拆分与预计改动文件

| 文件 | 职责与改动 |
|---|---|
| `desktop/ui/src/media-cycle-planner.ts` | 新增纯函数：N+1/N+2 队列、绝对目标、滚动、联动/独立模式、失效规则 |
| `desktop/ui/src/media-cycle-planner.test.mjs` | 覆盖确定性时钟、两层预选、跨循环和模式切换 |
| `desktop/ui/src/audio-cycle-prewarm-coordinator.ts` | 移除墙上时间目标；只保留声音候选状态转换，或在迁移后删除重复逻辑 |
| `desktop/ui/src/audio-cycle-prewarm-coordinator.test.mjs` | 更新为绝对媒体时间和单候选语义；删除过时墙上时间测试 |
| `desktop/ui/src/playback-control-message.ts` | 增加 generation、clock epoch/sequence、loop、绝对位置和 playbackRate 的严格消息契约 |
| `desktop/ui/src/playback-control-message.test.mjs` | 覆盖乱序、循环、seek、非法数字和旧 epoch 拒绝 |
| `desktop/ui/src/App.tsx` | 装配设置、随机 payload、最终窗口计划协议和 UI；移除 `nextAudioCyclePlanRef`/`nextVideoPeriodMsRef` 的重复事实源 |
| `desktop/ui/src/runtime-parameter-scheduler.ts` | 复用现有随机和相邻排除逻辑，不复制预设表 |
| `desktop/src-tauri/src/commands.rs` | 收紧绝对目标/revision 校验和任务上限；保留 current/pending 两槽 |
| `desktop/src-tauri/src/audio_cycle_switch.rs` | 统一计划身份和绝对目标类型，删除过时字段 |
| `desktop/src-tauri/src/audio_cycle_output.rs` | 保持 callback frame、输出延迟、A/V offset 和单一输出线程；仅补必要状态字段 |
| `desktop/crates/autolive-portaudio-output/src/lib.rs` | 原则上不改业务逻辑；只在缺少实际延迟观测字段时补充无锁原子快照 |
| `产品需求文档.md` | 记录独立/联动周期和 N+2 预选产品行为 |
| `系统架构总览.md` | 记录最终播放窗口为唯一媒体时钟所有者 |
| `桌面客户端架构.md` | 更新调度、任务上限、生命周期和校正链路 |
| `媒体参数范围与默认值.md` | 说明联动使用声音/视频范围交集；明确周期单位为媒体时间秒 |
| `长任务开发总计划.md` | 增加分阶段门禁与长时间音画同步验收 |
| `docs/superpowers/plans/2026-08-20-prewarmed-audio-cycle-switching-implementation-plan.md` | 链接本增量方案并标记后续绝对时钟改造 |

`App.tsx` 只保留装配，不把新的队列状态机继续堆入单文件。迁移中发现的未使用 ref、类型、消息字段和测试夹具随阶段删除。

## 14. 分阶段实施

### Phase 0：冻结契约并补失败测试

- 为现有墙上时间转换、缺少 N+2、联动目标不一致写失败测试。
- 记录当前字段、消息方向和 current/pending 任务计数。
- 确认现有 callback/A-V 校正测试作为不可回退基线。

验收：测试能稳定复现差距，不启动桌面端、不依赖真实睡眠。

### Phase 1：纯 N+1/N+2 计划器

- 新建 `media-cycle-planner.ts`。
- 实现初始化、滚动、过期、相邻排除、独立/联动目标。
- 计划器只接收显式时钟和随机源，禁止内部读取 `Date.now()`。
- 将声音和视频的单值 ref 迁移为两个未来计划。

验收：N+2 在 N+1 提交前已确定；滚动时 N+2 原样晋升，目标不重抽。

### Phase 2：最终播放窗口绝对时钟

- 建立 `FinalPlaybackClock` 和计划安装/确认协议。
- 最终窗口拥有 due/prepare 检查。
- 将 `target_at_ms` 全量迁移为现有 Rust 字段语义一致的 `target_absolute_position_ms`。
- 删除墙上时间换算兼容分支。
- seek、换源和窗口重建递增 clock epoch/playback generation 并拒绝旧计划；正常循环使用幂等目标 loop index。

验收：全仓搜索不到参与目标计算的 `target_at_ms`；倍速和循环不改变绝对目标语义。

### Phase 3：N+1 唯一预热与 Rust 校验

- 只允许队首 N+1 调用 prepare。
- N+2 不序列化给 Rust。
- N+2 晋升为 N+1 时才绑定最新 `base_audio_stream_revision` 和 candidate ID。
- 收紧 candidate/plan/playback generation/source/clock epoch/sequence 校验。
- 验证替换候选时先取消、进程退出、Join，再创建下一候选。
- 修正环缓等待和硬件延迟按 `playbackRate` 换算媒体时间。

验收：运行态 current + pending `<=2`，N+2 无 PID、无任务、无 PCM。

### Phase 4：视频队列和联动模式

- 视频参数也在 N+1/N+2 时预先抽样。
- 增加联动开关，使用声音/视频周期范围交集生成共享目标。
- 共同目标到期前完成两路 readiness 预检；任一路未准备好则两种效果都保持旧值，UI 不等待。
- 所有入口调用统一计划提交/快照函数。

验收：联动模式两条计划目标完全相等；独立模式各自按范围生成。

### Phase 5：校正、清理、文档和长测

- 保留并验证 callback frame、实际采样率、DAC latency 和 A/V offset。
- 删除旧 ref、墙上时间字段、重复 coordinator 分支和未使用代码。
- 更新架构、参数、产品和长任务文档。
- 完成本地 Windows x64 长测，不在服务器构建。

验收：50 轮和 30 分钟播放无持续漂移、无任务增长、无孤儿 FFmpeg。

## 15. 测试计划

### 15.1 TypeScript 单元测试

- 初始化一次生成严格两个未来计划。
- N+1 到期后，旧 N+2 不变地晋升为 N+1。
- 新 N+2 目标等于新 N+1 目标加新周期。
- N+2 永远不会产生 prepare action。
- 多预设时相邻计划不重复；N 可在 N+2 再出现。
- 只有一个可选预设时允许连续重复。
- 独立模式的声音、视频目标可不同。
- 联动模式的声音、视频目标、sequence 和 plan ID 相同。
- 0.5×、1×、2× 播放时目标媒体时间不变。
- 跨视频循环时绝对媒体时间单调。
- 循环完成消息按 generation 和目标 loop index 幂等处理，乱序消息不能多加一轮。
- seek/source/epoch 变化拒绝旧计划和旧回调。
- 联动范围取交集；无交集时拒绝联动。
- 一次跨过两个目标时不并发补跑两个音频候选。
- pause 不推进目标，resume 从当前视频绝对时间继续。

### 15.2 Rust 单元/集成测试

- prepare/commit 必须使用合法绝对媒体目标和匹配 generation/epoch/revision。
- stale plan/candidate/generation/source/clock 全部拒绝且保留 current。
- current + pending 上限为 2。
- 任务/PID 计数覆盖 preparing、crossfade 和状态槽替换过渡阶段。
- 替换 pending 时旧 FFmpeg 已退出并 Join。
- commit 门禁期间 prepare/cancel 不破坏当前交叉淡化。
- 候选失败不停止 current、不清空环缓、不回退 WebView。
- commit 结果返回 `pending/committed/discarded`，前端不能重试已销毁候选。
- callback frame 和 DAC latency 计算可听绝对媒体时间；环缓/硬件延迟按 0.5×、1×、2× 正确缩放。
- A/V offset 的连续阈值和冷却不被计划器改造破坏。
- NaN、Infinity、溢出和非法目标范围均被边界校验拒绝。

### 15.3 本地人工验收

矩阵：

- 模式：独立、联动。
- 采样率：44100、48000。
- 支路：1、2、4。
- 周期：固定 1 秒、5 秒、10 秒，以及 5～10 秒随机。
- 倍速：0.5×、1×、2×。
- 操作：播放、暂停、继续、停止、seek、循环、换源、自动 PortAudio 接管与 WebView 回退。

至少执行：

1. 连续 50 轮，记录 N+1/N+2、PID、任务数、目标/实际提交媒体时间和 A/V offset。
2. 连续 30 分钟，确认 offset 不持续累积，CPU/内存/句柄没有阶梯式增长。
3. 联动模式核对声音和视频每轮共享目标，实际触发差记录在允许阈值内。
4. 切换前后暂停/停止/seek 各 20 次，确认旧计划不能提交。
5. 人工听辨无数秒静音、电流声、硬切和持续画音错位。

## 16. 验收门槛

- [ ] 声音、视频未来计划深度均为 2，且无第三个未来计划。
- [ ] N+2 只含元数据，没有 FFmpeg、PCM、Rust task 或句柄。
- [ ] 活跃 FFmpeg PID 始终 `<= 2`，停止后可确认退出并 Join。
- [ ] 所有周期目标由最终播放窗口根据 `absolutePositionMs` 盖章。
- [ ] `Date.now()` 不参与目标计算，只允许用于诊断耗时、超时保护或唤醒。
- [ ] 联动模式声音/视频目标完全相同；独立模式文案明确不保证同时。
- [ ] 切换点不启动新的 N+1 FFmpeg；新的 N+1 只能在前一提交/取消完成后准备。
- [ ] PortAudio callback 无 Mutex、无 IPC、无 FFmpeg 和业务状态机。
- [ ] callback frame、实际采样率、输出延迟和 A/V offset 校正全部保留。
- [ ] 独立模式候选失败继续旧轨；联动模式预检失败保持旧声音和旧视频；两者都不制造静音、不让视频主线程等待。
- [ ] 50 轮无孤儿进程、任务数增长和候选提交风暴。
- [ ] 30 分钟 A/V offset 不持续增长；稳态目标 `|offset| <= 50ms`，连续超过 80ms 才触发校正。

## 17. 本地验证命令

实施阶段只在本地开发机执行：

```powershell
cd E:\aotlve\desktop\ui
npm run test
npm run build

cd E:\aotlve\desktop\src-tauri
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo check --workspace --all-targets --all-features
```

补充静态检查：

```powershell
cd E:\aotlve
rg -n "target_at_ms|Date\.now\(\).*target|nextAudioCyclePlanRef|nextVideoPeriodMsRef" desktop/ui/src
rg -n "spawn|Command::new|pending_audio" desktop/src-tauri/src
```

若原生 PortAudio Feature 不能与 `--all-features` 组合，按仓库实际 Windows x64 Feature 分别执行并如实记录，不能把未运行项写成通过。

## 18. 提交与回滚顺序

1. Phase 0/1：纯计划器和测试，单独提交。
2. Phase 2：最终窗口绝对时钟协议，单独提交；可通过关闭新计划器回滚到旧调度，但不得长期保留双事实源。
3. Phase 3：Rust 绝对目标校验和唯一 N+1 候选，单独提交。
4. Phase 4：视频队列、联动设置和 UI，单独提交。
5. Phase 5：删除旧代码、同步文档并完成本地长测。

每阶段均在当前分支和工作区实施，不使用 worktree，不启动远程构建。进入下一阶段前由主线程审查时间所有权、任务上限、锁范围、取消/Join 和过时代码清理。

## 19. 最终用户行为

用户可以选择：

- 独立周期：声音和视频分别按各自范围变化；都准时，但不承诺同时。
- 联动周期：声音和视频从两套范围的交集抽样，共享同一个目标；两轮未来结果都已预选。范围没有交集时 UI 明确拒绝开启联动。

内部始终只为最近的声音 N+1 准备一个候选。N+2 让系统提前知道“下一轮之后是什么”，但不增加 FFmpeg 和 PortAudio 压力。所有切换跟随最终画面的绝对媒体时间，硬件实际播放位置再由 callback frame、输出延迟和 A/V offset 持续校正。

## 20. 本次静态验收记录

- `desktop/ui npm test`：151 项通过。
- `desktop/ui npm run build`：TypeScript 检查和 Vite 生产构建通过；仅保留既有的大 chunk 警告。
- `desktop/crates/autolive-portaudio-output cargo test`：22 项通过。
- `desktop/src-tauri cargo test`：使用独立临时 target 避开正在运行 EXE 的文件锁，Rust 单元/集成/文档测试 210 项全部通过。
- `cargo clippy --bin autolive-desktop-core -- -D warnings`：通过。
- `cargo clippy --all-targets -- -D warnings`：未通过，现有 `media_engine.rs` 测试和 `tests/media_engine_contract.rs` 有 9 处 `field_reassign_with_default`，与本次音频时钟/水位改动无关，未借机修改。
- `rustfmt --check --edition 2021 src/commands.rs`：通过。
- `git diff --check`：通过，仅有工作区既有的 LF/CRLF 提示。
- 未启动或重启桌面端，未执行 Windows 声卡实机长时播放；50 轮、暂停/跳转、倍速和循环边界听感仍属于发布前手工验收。
