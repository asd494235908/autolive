# mpv/libplacebo 实时 GPU 主链实施方案

> 2026-08-31 补充：本文继续作为 mpv/libplacebo、GPU83、CPU4 与 Original 能力基线；视频周期所有权、四阶段提交确认、原子换源和 React 职责删除的当前唯一实施入口改为 [`Rust 视频周期唯一所有者实施方案`](./2026-08-31-Rust视频周期唯一所有者实施方案.md)。本文中关于 WebView `prepare/commit`、主页 `sync_realtime_video_renderer`、硬超时直接降级和旧候选 owner 的历史记录只用于审计，不再代表当前生产边界。

> 日期：2026-08-28
>
> 状态：实施中（里程碑 1 已完成，当前 1080p Phase 1 实机门禁已通过；整改锁 `cff165...` 的断网冷构建已完成 mpv 链接，但在生成证据包时再次因宿主 Docker Desktop Linux Engine 管道消失而中断，未生成可提升候选）。本文是当前视频播放、GPU83、CPU4、Original 降级和音画同步的唯一实施入口；4K 延后，不属于当前开发与验收范围；旧 Phase 7A 报告不再准入当前锁，Phase 7B 法律/发布准入、完整 GPU83、生产 CPU4 降级、真实 PortAudio 闭环、Windows 多硬件和长稳验收仍不得标记为已交付。
>
> 覆盖范围：本文覆盖此前“逐 period FFmpeg 重编码 → `.partial/.ready` → Rust 分块 → WebView2 MSE”、视频 A/B、WebView2 CSS 视频效果以及“mpv/libplacebo 仅作预留”的视频主链描述。两份旧视频实施方案已按用户要求删除，后续不得从提交历史恢复为当前上下文。普通声音 N/N+1、PortAudio、插话、固定话术、播放池和导入原子性继续有效。

## 0. 当前实施进度（更新至 2026-08-30）

- 2026-08-30 完成“第 6 次周期后误降级并停止继续变换”的代码整改与非媒体门禁，待用户真实媒体验收：现场 mpv 进程、VO、PTS 和 shader 均存活，参数不是非法值；直接根因是 mpv `vo-passes.fresh.samples` 为最多 256 帧的滚动窗口，旧实现每秒重复计算整窗 P99，把同一个 `100.516ms` 历史尖峰连续计数三次并错误触发 GPU 降级。runtime 现保留完整滚动 P99 仅用于诊断，降级判定只消费相邻快照中新进入窗口的样本；固定 256 项窗口按最大后缀/前缀重叠提取新尾部，PTS 前进但某个 pass 样本恒定时按该 pass 最新一帧计入，PTS/计数器回退以及 seek、循环、换源、shader 提交和后端切换均重建健康基线，历史尖峰不跨边界继承。前端同时以 `playback_generation + backend_epoch + process_id + backend + launch_mode` 标识物理流水线 owner；owner 切换或同 owner 连续两次出现 `N active + N+1 缺失 + N+2 planned` 时，只作废一次旧候选并立即重建 N+1，避免降级后永久停在 N=6。代码总监复核补齐“恒定 pass 与变化 pass 并存”计时及同一新 owner 只恢复一次的边界；Rust runtime `98/98`、降级契约 `5/5`、前端同步/候选 `32/32`、视频运行时 `43/43` 和前端生产构建通过。未修改普通声音/PortAudio 功能、GPU 仍为 `79/83`、CPU4 为 `4/83`、周期仍为 `5–8s`、4K 仍延后；真实多文件、连续周期及长稳无卡顿尚待用户验证，未通过前不得作保证性宣传。
- 2026-08-30 完成用户现场再次反证后的周期提交与主页 seek 根因整改，待用户真实媒体验收：mpv IPC 已确认 shader 正常加载且播放 PTS 越过 N+1 目标后 83 项仍保持中性，根因是 Rust runtime 的观察分支只记录物理 PTS，从未在到点时执行 commit；先前 React 单次目标定时唤醒方案已从生产代码和测试删除，避免继续把窗口计时器作为视频事实源。现在 runtime actor 的首周期 `Available`、GPU `Active` 与 CPU4 `Active` 三条有界观察分支共享唯一 pending 提交边界，使用 `loop_index × source_duration + presented_pts_ms` 计算绝对 PTS，到达目标后在同一 actor 内原子应用一条完整 GPU83/CPU4 快照并把 N+1 提升为 N；前端只在轮询看到同身份 `Active + N.sequence` 后发出幂等确认以结算界面队列，迟到确认不会把已生效周期误判为 stale。主页拖拽此前只广播给最终效果窗，再依赖无源 HTML `<video>` 回传位置，首次新 epoch 可携带旧 PTS且后续正确位置不会再次触发 sync；现改为松手时直接用草稿目标提交 `sync_realtime_video_renderer`，连续拖拽分配严格递增 epoch，末尾限制为 `duration-1ms`，在同身份 mpv PTS确认前旧时钟不能覆盖草稿。Rust 对 `UserSeek/LoopBoundary` 等所有时间轴边界清除旧 N+1/N+2、保留当前 N并按新 cursor 重建。新增/更新前端同步与候选聚焦 `29/29`、Rust 三态自动提交/幂等/seek 候选清理 `4/4` 通过；未修改普通声音处理代码、GPU `79/83`、CPU4 `4/83`、`5–8s` 周期或 4K。真实参数回读、连续拖拽、多文件轮播及长稳仍须本轮开发端复验，未通过前不得宣称无卡顿。
- 2026-08-30 完成播放前/播放中开启视频处理及 EOF 双所有者竞态整改，待用户真实媒体验收：播放前开启继续由主页等待处理开关事务提交后再打开最终效果窗并启动，播放中开启继续在同一受管 mpv 上从 Original 中性态准备首个 GPU/CPU4 周期；两条路径从健康媒体时钟开始后共享同一 N/N+1/N+2 管线。现场反复冻结的直接证据为 PlaybackCore 已进入下一循环而 runtime 仍绑定旧 EOF，随后 supervisor 记录 `loop_index_mismatch` 并留下分裂状态。现有 React 仅凭 2 秒 DTO 轮询推断 `managedNativeVideoActive` 的瞬时布尔值已替换为黏性受管所有权：DTO 短暂缺失、旧身份和单轮 EOF 交接不把推进权或可见画面交回 WebView；当前/相邻上一循环的明确 terminal/无进程 Source 才释放。Rust 对任何同 generation 且仍有受管进程的外部 `complete_playback_item` 幂等不推进，并对“同代次、单视频源、Core 恰好领先一轮、runtime 仍为旧 EOF”的迟到事件复用既有两阶段 `advance_after_eof` 只收敛 runtime；失败时仅停止仍匹配的旧会话，其他错配继续 fail-closed。Presented/Failed 和 100ms 候选状态均绑定当前 generation/clock/loop，旧 active/failed 不再冒充当前周期。验证通过：Rust 桌面命令 `102/102`、主线程 EOF 聚焦 `3/3`、前端聚焦与开关/同步 `117/117`、TypeScript；未修改普通声音、GPU `79/83`、CPU4 `4/83`、shader、`5–8s` 周期和 4K，也未启动真实视频或长稳测试。
- 2026-08-30 完成“多文件 EOF 偶发 `runtime_eof_identity_mismatch` 后 Source 等待候选”的最小整改，待用户真实媒体验收：根因不是 shader 参数或 `source_media_index` 次序，而是 actor 在共享状态发布前先发送 EOF 事件，supervisor 偶尔读取上一份 runtime 快照并按严格合同拒绝。现统一为“观察并记录物理 EOF → 发布共享状态 → 发送监督事件”，容量 `1` 通道 Full 时后续 tick 重试，成功通知保持去重；`status.eof`、PID、backend/clock/loop 和物理 EOF 门禁均未放宽。前端同时以 plan/source/generation/source-revision/loop/switch-revision 清理源切换后残留的 active、pending apply 和资源等待 owner，作废旧 token，合法同身份仍单飞，旧异步回调不能恢复旧候选；Source 已有计划时显示“源画面播放中 · 正在恢复视频候选”。验证通过：前端视频候选 `38/38`、关联回归 `88/88`、TypeScript，Rust EOF 聚焦 `14/14`、生命周期 `4/4`、Cargo fmt/check；未修改普通声音、GPU `79/83`、CPU4 `4/83`、`5–8s` 周期、4K，也未运行真实媒体或长稳门禁。
- 2026-08-30 完成“当前 N 被 N+1 状态遮蔽、受管 mpv 与 WebView 重复推进循环、迟到 EOF 被反复拒绝”的最小整改，待用户真实媒体验收：前端先判断当前 N 是否已经 Presented，已生效时主状态固定显示“当前周期已生效”，下一候选的 preparing/ready 只作为次级描述；受管 mpv 活跃或 EOF 身份过渡期间，共享播放边界拒绝 WebView `<video>`、`timeupdate/ended` 和隐藏原声音轨回绕调用 `complete_playback_item`，只允许兼容 WebView 画面保留该兜底。Rust EOF 监督新增严格幂等分类：仅当同一 generation 的单一视频源处于 playing、`current.loop_index == eof.loop_index + 1`，且 mpv runtime 也已进入同一新循环、旧 EOF 已清除、进程仍播放时记录 `stage=ignored reason=already_advanced`；旧 runtime、单边推进、跨 generation 或相隔多轮仍 fail-closed，迟到事件绝不再次推进。前端聚焦回归 `62/62`、TypeScript 和 Rust EOF 定向 `3/3` 通过；本批未改 GPU `79/83`、CPU4 `4/83`、周期随机、声音功能或 4K，也未运行真实媒体和长稳门禁。
- 2026-08-30 完成视频后端 `physical_paused` DTO 生命周期整改的代码与非媒体门禁，待用户真实媒体验收：现场 Tauri 与 mpv 进程均存活、mpv CPU 和 EOF 循环继续推进，但 Rust 在启动/中性旁路阶段可能先发布 `process_id=Some + physical_paused=None`，在 stop/demote 阶段又可能发布 `process_id=None + physical_paused=Some`，被前端严格合同正确拒绝。现统一规定：GPU、CPU4、Original、同进程换源和中性旁路只有在串行 mpv IPC 回读确认真实 `pause` 与 `eof-reached` 后，才一次性记录 PID、PTS 和物理事实；stop、demote、probing、failed 同步清除 PID、PTS、物理暂停、物理 EOF 与逻辑 EOF。前端未放宽校验，并补齐两种非法配对、无进程残留 PTS/EOF、合法 spawned/active 的跨语言回归合同。该修复不改变 GPU `79/83`、CPU4 `4/83`、周期算法、shader、声音功能或 4K 边界；未运行真实媒体，需用户复验首次启动、处理开关、降级和循环场景。
- 2026-08-30 完成“长时间周期变换后画面冻结”根因整改的代码与非媒体门禁，待用户真实媒体验收：旧 runtime 在每个约 `40ms` 观察 tick 中按 PTS 重新计算并写入 `glsl-shader-opts`，长期累积的视频选项更新会让 mpv/libplacebo 渲染停止推进；现改为每个 `5–8s` 周期提交且只提交一条完整 shader 快照，18 项调度中的视觉门控、PIP 抖动、切片、高光、局部模糊、平滑和异步旋转由受控 shader 使用自动 `PTS + 周期起点 + source fps + seed + 间隔/幅度` 在 GPU 内演化，40ms 健康观察不再写 shader 属性。帧率相关调度仍只通过既有受限速度入口执行。新增物理呈现 PTS 看门狗：仅在进程存活、非暂停、非 EOF 的受管 GPU/CPU4/Original 播放中启用，停滞阈值按源帧率计算并限制在 `1.5–5s`；每次停滞只允许一次同源 seek/恢复播放，继续不前进则进入既有 GPU→CPU4→Original 失败路径，seek、循环、换源、新会话、暂停、EOF 或 PTS 恢复都会清零。EOF 同时改为“预演下一身份 → 物理 seek/loadfile 并验收成功 → 在最新 `PlaybackCore` 上恰好提交一次”，物理失败时 generation/loop/index/position 不推进，物理成功后发现身份已过期则只清理仍匹配的旧 runtime，不覆盖并发状态或误杀新会话。开发能力口径仍为 GPU `79/83`、CPU4 `4/83`；未启用具有持续性能代价的 libplacebo `dynamic_constants` 原始调试选项，未修改声音功能，未运行真实媒体、1080p60、30 分钟长稳、多厂商 GPU 或 4K，因此不得据此承诺周期边界绝对无卡顿。
- 2026-08-30 完成“mpv EOF 后端原子恢复”代码与非媒体门禁，当前状态为已实施、待真实媒体验收：Rust runtime actor 每 `40ms` 有界观察物理 `eof-reached`，通过容量 `1` 的非阻塞去重通道直接唤醒 Rust EOF 监督线程；React 不再发送原生 mpv 的 `complete_playback_item`，也不在本地 seek、恢复播放或递增 `loop_index`。监督事务严格校验 `playback_generation/backend_epoch/clock_epoch/loop_index/source_media_index`，在 clone 上预演下一身份后先完成 mpv 物理 seek/loadfile 与回读验收，再重新锁定最新 `PlaybackCore` 并恰好提交一次；物理失败时权威播放身份不推进，提交前身份过期时不得覆盖并发状态或误杀新会话。前端在 EOF 待处理期间只冻结并清理视频候选 prepare/commit/retry，展示“EOF 切换中，等待后端推进”，新后端身份建立后重建 N/N+1/N+2。一次恢复只有在回读确认 `pause=false`、`eof-reached=false`、`seeking=false`、源路径/媒体段身份正确、`source_pts_ms` 位于合法源内范围且连续观测 PTS 前进后才算成功；非 stale 恢复失败先记录 `Source/Failed` 再无条件有界释放失效 mpv，stale/superseded 不误杀更新后的会话。Rust 桌面命令 `95/95`、runtime `78/78`、实时入口合同 `6/6`、前端聚焦 `111/111`、TypeScript、Cargo check/严格 Clippy/fmt 已通过。本整改不修改普通声音 DSP、候选格式、N/N+1、PortAudio 输出、插话、固定话术或混音语义；真实 1080p 单/多文件、声音开关组合、循环压力和长稳门禁通过前不得宣称无卡顿或音画同步已保证。
- 2026-08-30 完成本轮音画稳定性核心代码与聚焦测试，真实媒体验收待用户执行：音频和视频统一使用 `playback_generation + source_path + loop_index + source_duration_ms` 的媒体段身份，并把跨循环的 `presentation_pts_ms` 与单个源文件内的 `source_pts_ms` 明确分离；PortAudio 可听时钟可携带两种位置，但 mpv `time-pos/seek` 只接收满足 `0 <= source_pts_ms < source_duration_ms` 的源内位置，禁止把第 N 轮绝对呈现时间写入单个文件。播放态循环边界固定执行 `Seek → SetPause(false)`，清除 `keep-open` 遗留暂停；用户暂停态固定执行 `SetPause(true) → Seek`，不得误解除暂停。身份不匹配、换算失败或目标达到/越过 EOF 时，只停用本轮音画纠偏并恢复基础视频速度，不把音频时钟错误归类为 GPU/CPU4 故障，也不触发视频降级。相关 Rust 时钟、同步控制器、runtime、命令合同和聚焦测试已通过；本轮未执行 1080p 真实媒体、多文件轮播、1080p60、4K 或长稳验收，不能宣称无卡顿或音画同步指标已经实机通过。
- 2026-08-30 完成视频同步请求的 latest-wins 与稳定错误语义代码接线，真实媒体验收待用户执行：React 同一时刻只允许一个 `sync_realtime_video_renderer` IPC 在途，后到请求覆盖尚未提交的 pending；`busy` 只按 `100/250/500ms` 有界退避，`result_unknown` 先读取 Rust 后端状态确认是否已经应用，`stale` 刷新最新 desired/播放快照后再提交，迟到响应不得覆盖新 revision。Rust 对 stale、superseded、busy、result_unknown、transport、invalid 提供稳定分类，并阻止同 generation 的迟到 Original 请求覆盖已有健康 PID 的 GPU/CPU4 会话。同步路径中的 mpv `IpcTimeout` 只表示“命令确认结果未知”：保留当前会话，返回 `result_unknown` 供状态确认和幂等重试，不再把该确认超时当作进程故障推动 GPU→CPU4；`IpcDisconnected`、断管、进程退出和其他已确认传输失效仍按原有单向降级处理。前端聚焦测试、类型检查及 Rust 同步合同/聚焦测试已通过；普通声音候选 `prepare` 移出 Tauri 主线程的调度代码与聚焦检查也已完成，但真实联合媒体体验仍待用户验收。
- 2026-08-30 完成视频周期循环事实源收口，代码及非媒体自动化通过、待用户实测（其中由前端触发 `complete_playback_item` 的历史实现已由同日 EOF 后端原子恢复决策覆盖）：现场只读证据同时出现 Rust `loop_index=0`、mpv runtime `loop_index=1`、最终效果窗本地时钟 `loop_index=2`，已准备的 N+1 因身份不一致永远不能提交，mpv 也因同步请求被拒绝而停留在暂停态。`playback_generation/source_media_index/loop_index` 只由 Rust `PlaybackCore` 推进；WebView 原声音轨仍可原生连续循环，完成前不发布倒退的绝对时钟，也不再由媒体位置推算或提前递增循环。视频候选按目标绝对时间计算所属循环，只有媒体时钟、Rust 快照和目标循环三者一致才允许 prepare/commit；跨循环计划保持 planned，进入权威循环后再准备。命令层同步不再接受 `snapshot.loop_index + 1`，mpv 暂停事实只取 Rust 播放状态；runtime 在权威循环边界释放旧 N+1/N+2、保留已生效 N。验证通过：前端聚焦 `112/112`、前端全量 `544` 通过且 `1` 项既有条件跳过、TypeScript、Vite 构建、Rust 同步合同 `4/4`、实时入口 `6/6`、runtime `72/72`、`cargo check --all-targets`、严格 Clippy、fmt 和 `git diff --check`；运行中的开发端锁住默认 debug EXE 后改用独立本地 Cargo target 完成验证，没有终止用户进程。普通声音 DSP、声音候选、PortAudio、插话、固定话术和混音未修改；未运行真实媒体、4K、1080p60 或长稳门禁，不能据此宣称现场周期变化或无卡顿已经实机通过。
- 2026-08-30 完成播放声音连续性与原生失败回退整改，待用户实测：现场只读证据显示 Rust 播放态为 `playing`，但原生后端已 `stopped`、PID 与 PTS 均为空；前端仍每 250ms 优先消费 2 秒状态轮询残留的 mpv PTS，并在超过 120ms 时硬 seek 原声，形成可听卡顿。当前带音轨视频在 WebView 原声可用时以其连续时间作为前端源时钟，mpv PTS 仅在无可用音轨时兜底；普通漂移不再改变原声倍速或硬对齐，只有显式 seek、真实 loop/ended 才允许对齐。恢复逻辑重载实际主时钟；原生后端停止/失败后，WebView 兼容画面立即重新加载并跟随原声位置。Rust 既有真实 Original 故障继续发布 `Source + Failed + process_id=null`，意图性 stop/suspend 保持 `Stopped`，新增合同测试防止混淆。PortAudio 活动时仍由 Rust 可听 PTS 闭环主导；普通声音 DSP、候选、插话、固定话术和混音未改变。本批未运行用户媒体、1080p60、长稳、4K 或多硬件门禁，不能据此宣称现场卡顿已验收消失。
- 2026-08-30 视频周期新配置默认区间由 `8–15 秒` 调整为 `5–8 秒`；硬范围仍为 `1–60 秒`，每轮按真实媒体时钟从闭区间重新随机，既有合法本地保存值保持不变。声音周期仍为 `3–5 秒`，本项不修改声音处理。
- 2026-08-30 完成 TS/MP4 原生时钟、EOF 与周期推进修复，并使用用户提供的 `初七chuqi服饰20260825184448.ts`、`e91e31b67264b3d24eb929b8e910717c.mp4` 做开发端实机复核（该次“前端据 EOF 推进”的历史所有权已由同日 EOF 后端原子恢复决策覆盖）：WebView2 无法解封装 TS 时，最终窗口改用同一四维身份的受管 mpv `presented_pts_ms` 作为视频时钟；首轮 N 尚未提交的 `Available + Spawned` 进程允许携带同身份 EOF，不再把真实对象丢弃为“响应必须是对象”。runtime actor 每次真实 PTS 前进都发布新状态，修复“mpv 正常播放而 DTO PTS 停滞”导致的周期冻结；Source、首轮 available、GPU active、CPU4 active 共用该发布规则。Original 启动与 GPU prepare 统一从 Tauri `resource_dir` 解析 `gpu83.hook`，开发资源不再因目录分叉被误判缺失并降到 CPU4。现场结果：TS 能进入 D3D11/D3D11VA/gpu-next，随后自动切到 MP4；`1–2s` 周期在同一 PID 内连续跨越多个 N，12 秒采样中 PID 未变、VO/解码/延迟丢帧增量均为 0，mpv IPC 回读到非中性 shader 参数。该证据只覆盖当前两份素材和本机 22/26fps；该次复核时换文件仍会建立新 mpv PID，后续同进程换源实现另有独立记录，因此本条不能构成多文件无缝切换、1080p60、跨 GPU 或长稳承诺。未修改普通声音 DSP、声音候选、插话、固定话术或混音，4K 仍不在范围。
- 2026-08-30 完成受管 Original DTO 契约漂移修复，待真实媒体验收：Rust 为保持同一 mpv PID，在视频处理关闭时会按物理启动模式发布 `Original（中性 shader）`、`Original（CPU4 中性参数）` 或 `Original（无 shader）`；前端此前只接受最后一种，导致合法 `Source + Active` 状态被误报为“受管 Original 管线信息无效”。前端现严格接受三种受管中性管线，并精确校验四档 GPU/CPU4 fallback、图形 API 和真实解码器组合；生产 IPC 入口统一使用结构化解析结果。尚未取得首个状态时显示“状态读取中”，不再把 React 内部 `null` 伪装成“响应必须是对象”；陈旧候选恢复期间也不提前清空最后一条 DTO 诊断，只有新的合法状态才清除诊断。聚焦回归测试 `27/27` 与 TypeScript 检查通过。用户提供的 TS/MP4 已完成只读预检：MP4 视频帧完整；TS 有一次 H.264 宏块解码错误和约 `61ms` 起始音画差，但不构成 DTO 故障原因。该修复未改变声音功能、GPU `79/83`、CPU4 `4/83` 或 4K 范围，真实轮播、参数回读和周期无卡顿仍需本轮实机门禁确认。
- 2026-08-30 完成“循环播放中开启视频处理画面卡住”的单会话整改，待用户实测：现场证据表明旧开关命令固定执行 `suspend_realtime_video_runtime()`，主动终止 mpv 后从保存 PTS 新建 GPU 会话，卡顿来自进程/视频表面重建而非 shader P99。当前实现已将逻辑 `Source/Original` 与 mpv 物理启动模式分离。在同一源、同一 playback generation、同一宿主 HWND、进程存活且物理模式仍匹配当前 fallback floor 时，关闭视频处理由 runtime actor 在现有 mpv 上写入 GPU 中性 shader 或 CPU4 四项中性值并清除视频周期计划；再次开启后复用同一 PID 提交后续周期快照，不再仅因开关切换执行 surface suspend、重新 seek 或重建视频表面。迟到的旧 Prepare 在开关关闭后只幂等确认中性旁路，不再记录 Source 兜底并终止进程；快速开关一次只允许一个状态转换；视频同步必须匹配 playback generation。首次建立会话、进程不存在/退出/丢失、不满足连续视频 EOF 复用门禁的换源或播放代次变化、宿主 HWND 变化、物理模式与 fallback floor 不匹配，以及导致会话失效的 GPU、设备或 IPC 故障降级、显式停止或关窗，仍允许先回收旧进程再创建或重启；普通周期更新不允许重启。GPU 自动周期仍为 `79/83`，CPU4 仍为 `4/83`。本批未运行真实视频、1080p60/长稳、4K 或多硬件门禁，未修改声音功能，因此最终效果和开关流畅性仍须用户实测确认。
- 2026-08-30 功能/测试包优先批次完成最小收口：PortAudio 可听时钟在读取时按快照年龄和实际播放速率外推，避免 25Hz 发布间隔被误判成音画漂移；音画控制器先拒绝乱序观测再切换同步身份，旧样本不能污染新 epoch。该修复只触及只读时钟与视频同步控制，不改变普通声音 DSP、声音候选、插话、固定话术或混音。GPU 周期继续按 `79/83` 执行，四项未准入字段保持 fail-closed 且不泄漏到基础或调度 shader 快照。测试版使用独立 debug target、独立应用标识和 `package-test` 输出，NSIS、portable ZIP 与隔离安装启动均已通过；固定连接测试控制面 `http://101.96.208.132:9090`。本批未播放用户媒体、未做 4K、长稳、完整多 GPU 矩阵或正式发布准入，测试包不得作为正式发布包。
- 2026-08-30 按用户收窄暂停正式 Phase 7A/7B 冷构建、法律准入和候选替换，优先恢复测试打包环境。新启动的 `cff165...` r3 断网冷构建容器已停止并保留为 Exit `137`，未删除容器、volume 或输出。测试包通道现与正式发布门禁隔离：`tauri:build:test` 只复用当前开发资源，固定测试控制面 `http://101.96.208.132:9090`，使用独立 debug target、`package-test`、产品名 `GpAutoLive Test` 和应用标识 `com.gepin.autolive.desktop.core.test`，不执行 Phase 7A 供应晋级、不替换正式资源且保持正式 `tauri:build` fail-closed。Windows NSIS 与 portable 已构建；portable 和隔离安装后的程序均创建真实测试主窗口，安装器 Exit `0`。本轮仅做无媒体构建/安装/启动冒烟，未播放媒体、未运行长稳，也不代表正式发布、法律或候选资产准入。
- 2026-08-30 GPU83 第三阶段颜色候选已完成但经代码总监审核未准入生产：候选定义了 BT.709 primaries、sRGB TRC、full range、203 nit 和 libplacebo `perceptual_strength`，但外部 `mpv.exe + JSON IPC` 需要分别写 `glsl-shader-opts` 与 `libplacebo-opts`，无法证明同一呈现帧上的原子提交；关闭强度也不等于关闭完整颜色转换，HDR/驱动兼容、暂停时目标属性就绪和完整运行事实均未验证。因此两项颜色恢复 `Unavailable`，自动周期保持 `61 shader + 18 scheduler = 79/83`，前端冻结其基线并标记“正式需求·待实现”。本轮同时修复既有 GPU 重建/跨 GPU 降级的活动 shader 恢复：新进程仍暂停时先重放最后已提交的完整 shader 快照，再恢复播放，避免 61 项短暂回到默认；新增顺序合同锁定“连接 → 重放参数 → 恢复播放 → 等待首帧”。两项历史能力仍冻结；CPU4 仍严格四项；未修改声音功能、未运行真实媒体、未实施 4K。非媒体门禁结果：Rust 库 `430/430`、桌面命令 `85/85`、GPU 准入 `33/33`、相关前端 `28/28`、TypeScript、fmt/check/严格 Clippy 均通过；前端全量 `531/534` 通过、`1` 项跳过，仍有两项与本批无关的既有静态断言失败（Phase 1 发布审核字段、App 自动参数函数旧调用形态），未借本批扩大范围修复。
- 2026-08-30 跨厂商 GPU83 第二阶段已完成 18 项 PTS 调度参数的开发生产接线：capability 新增受限 `ScheduledParameter`，只允许现有媒体 PTS 调度器、固定 `speed` 合成入口和 11 个已验证 runtime shader 选项，不开放通用 mpv property。前端自动周期从 61 项扩展为 `79/83`，周期完整快照只冻结 2 项未定义颜色管理与 2 项历史依赖；CPU4 仍严格只执行亮度、对比度、饱和度、色相四项。代码总监纠正了旧分类：PIP 像素抖动是可执行的同帧调度参数；`slice_min_length_ms` 与 `picture_in_picture_timeline_locked=false` 才需要过去源片段/独立时间轴，必须继续 fail-closed。Phase 3C 静态门禁同步为 `61 shader + 18 scheduler + 2 color + 2 history = 83`，现有外部 `mpv.exe + JSON IPC` 仍不得冒充历史纹理。Rust 库 `428/428`、命令 `85/85`、GPU effects `12/12`、帧调度 `15/15`、Phase 3C `11/11`、前端随机池 `10/10`、TypeScript、fmt/check/严格 Clippy 通过；本批未运行真实视频、未修改声音功能、未实施 4K，因此 `79/83` 表示开发代码可达，不表示 1080p 长稳或无卡顿门禁已完成。
- 2026-08-30 跨厂商 Full GPU83 整改第一阶段已完成非媒体生命周期与启动规格收口：周期 N+1 复用同一受管 mpv PID 时不再重复执行 HWND/视频表面验证；表面验证失败触发的意图性 `suspend` 以 `Stopped` 事实保留，下一次 prepare 重启当前 fallback floor，不再被误判为“会话丢失”并错误降级；真实子进程异常退出新增退出码与有界、已脱敏 stderr 尾部证据。生产启动顺序已扩展为与厂商无关的 `D3D11 零拷贝 → D3D11 copy → Vulkan/WinVK copy → GPU+软件解码 → CPU4 → Original`，Rust 与 React 同步接受最多五条降级历史及 D3D11 copy 的实际 decoder。四档 GPU 的纯能力选择合同只按设备、解码、渲染、shader、历史帧、调度和 `83/83` 门禁选取，不写 AMD/NVIDIA/Intel 特判；它尚未冒充实际硬件探测结果。19 项调度字段已补齐纯 PTS、暂停、seek/loop、切片窗口、PIP 时间轴、异步旋转和平滑语义，但尚未全部接入真实像素输出，继续保持 capability 不可用；两项颜色管理因缺少 primaries/TRC/range/输出标签仍 fail-closed，代码总监已阻止用 RGB 通道轮换冒充；一项历史字段仍等待已准入历史纹理。因此生产能力仍为 `61/83`。本阶段非媒体门禁通过：Rust 库 `428/428`、桌面命令 `85/85`、帧调度 `15/15`、前端视频状态 `20/20`、TypeScript、`cargo fmt --check`、`cargo check --all-targets`、严格 Clippy 和 `git diff --check`；本批未运行真实视频、未修改声音功能、未实施 4K。
- 2026-08-30 完成 mpv 条件帧指标误降级与 stale 候选恢复整改，待用户实测：现场存活进程为 Original，直接降级原因为 `get mistimed-frame-count: property unavailable`。mpv 官方语义允许该 display-sync 条件指标及 `vo-delayed-frame-count` 不可用；runtime 现复用已有强类型 `PropertyUnavailable` 将两者记为 `None`，不累计帧预算观察失败、不参与计数回退/增长比较，重新可用后只比较连续有效样本；`vo-passes`、两类 drop 指标及真正 IPC/协议故障仍维持既有连续门禁。React 将 `stale_realtime_video_plan`、`realtime_video_prepare_stale`、`stale_realtime_video_backend_epoch` 作为可恢复竞态，立即释放旧候选，并行刷新权威播放快照与后端状态，隔离旧响应后按最新身份重建。本批未新增依赖、未修改声音、不扩大 GPU `61/83` 或 CPU4 `4/83`、未实施 4K、未运行真实媒体；非媒体门禁完成后由用户复验。
- 2026-08-30 本批非媒体代码总监门禁通过：前端聚焦测试 `52/52`、TypeScript 类型检查、Rust `cargo fmt --check`、`cargo check --all-targets`、严格 Clippy、runtime `62/62`、IPC `9/9`、实时入口 `6/6`、降级合同 `5/5` 及 `git diff --check` 均通过。降级合同首次运行暴露一条既有静态断言仍使用 `record_original_started` 的旧两参数签名，已只同步为当前包含 `process_id` 的三参数事实后通过；没有改变生产行为。开发端保持运行，未执行真实媒体与 4K 验收。
- 2026-08-30 根据开发端现场状态完成 GPU/CPU4 首帧假失败整改，待用户复验：CPU4 进程已成功建立 D3D11 VO，`vo-passes.fresh` 也返回完整 pass 描述，但固定 `--pause=yes` 握手使全部 `count=0/samples=[]`；旧逻辑在恢复权威播放状态前等待样本，5 秒后把健康的暂停会话误判成“首帧尚未呈现”，GPU 路径同样存在该时序矛盾。现在 mpv 仍暂停启动，IPC 连通后先恢复请求状态；实际播放的 GPU/CPU4 才等待 render sample，用户暂停时只要求 VO、有效 PTS 和 decoder，恢复播放后继续使用既有 runtime `vo-passes`、P99、帧预算和连续失败降级门禁。嵌套启动错误只向外层后端保留原始原因，避免重复“mpv 进程…初始化失败”。本批不修改声音、不扩大 GPU `61/83` 或 CPU4 `4/83`、不实施 4K、不运行额外真实媒体；代码与非媒体合同完成后由用户复验。
- 2026-08-30 首帧假失败整改的非媒体门禁已通过：`cargo fmt --check`、`cargo check --all-targets`、`cargo clippy --all-targets -- -D warnings`、runtime `61/61` 和隔离 target 的实时入口合同 `6/6`。常规入口测试首次仅因开发端占用 Windows `target/debug/autolive-desktop-core.exe` 而无法替换该文件，改用独立本地 target 后完整通过；这不是代码失败，也未停止开发端或运行真实媒体。
- 2026-08-30 完成“视频后端状态不可用/候选处理中”专项整改并通过非媒体代码总监门禁，待用户实测：根因之一是完整 GPU→CPU4→Original 失败后，runtime 已形成合法 `Source + Failed + process_id=null`，但命令层仍强制要求 PID 并将真实终态覆盖为 `mpv_video_window_not_ready`；另一原因是长 stderr/降级文本可让前端整份 DTO 失效，且 prepare/commit 的无效响应没有在所有路径释放候选 owner。现已规定 GPU、CPU4、受管 Original 只有取得 PID 才可发布可用/活动状态，全失败终态跳过 HWND 验证；runtime 原因按 256 UTF-16 单位压缩、路径脱敏，suspend 保留身份、fallback floor 和三条历史。React 以稳定 `code/field/detail` 区分 IPC 与 DTO 失败，保留最后有效身份，并在无效响应或终态上对称释放候选；既有重试保持有界。验证通过：前端类型检查、视频后端 `19/19`、视频运行时 `30/30`、Rust runtime `60/60`、实时入口合同 `5/5`、`cargo fmt --check`、`cargo check --all-targets` 和 `cargo clippy --all-targets -- -D warnings`；未启动开发端、未运行真实视频、未修改声音功能、未实施 4K。生产能力仍为 GPU `61/83`、CPU4 `4/83`，周期真实变化与流畅性由用户验证。
- 2026-08-29 完成重复 `mpv IPC 请求 11/12（get time-pos）等待响应超时` 的第二轮根因整改，待用户实测：Windows 命名管道由单一 `mpv-ipc-worker` 独占，严格串行 `write + flush + read response`，删除同一同步管道句柄的克隆读写线程与 pending 表。事件可穿插，但响应必须匹配当前 `request_id`；超时后会话整体毒化并由 runtime 回收，迟到响应不得污染下一命令。精确的 `property unavailable/property not found` 单独建模为可重试状态，传输超时、断管、乱序、非事件缺少 request ID 和协议错误不再被首帧循环吞掉。启动查询共享 `5s` 总预算，不再为首次 HEVC 解码/VO 查询固定截断为 `500ms`；运行态仍保持 `250ms/350ms` 预算。`--wid` 改用 mpv Windows `uint32_t` 合同，并在 Original/GPU 准备返回前按 mpv PID 枚举专用宿主窗口树，确认存在可见、非零客户区的视频子窗口，否则先回收会话并返回窗口计数诊断。代码与非媒体测试完成，未启动开发端、未运行用户媒体、未修改声音功能、未实施 4K；真实流畅性、EOF、关窗重开和 1080p60 仍由用户验收。
- 2026-08-29 第一轮专用原生视频宿主修复保留为历史实施记录：不再把 Tauri/WebView2 顶层 HWND 直接传给 mpv，而是由职责单一的 `autolive-native-video-host` 在 Tauri 主事件线程创建、resize 和销毁 Win32 child HWND；主 crate 继续禁止 unsafe。该轮只校验宿主自身，仍使用克隆同步管道句柄和固定 `500ms` 首帧命令预算，用户实测继续出现请求 11/12 超时，因此不能视为最终修复；其不足已由上面的第二轮 IPC 单所有者、`uint32_t --wid`、PID 视频子窗口实证和共享启动总预算整改替代。
- 2026-08-29 完成异步视频开关的 backend epoch 竞态整改：React 新增视频专属开关 revision 栅栏，`prepare_realtime_video_plan` 必须等待串行开关队列返回权威 snapshot/backend status，并在调用前复核播放代次、源、开关和 revision；切换期间的提前候选与晚响应静默失效。Rust 新增 `StalePrepareBackendEpoch` 与稳定命令错误码 `stale_realtime_video_backend_epoch`，同代次旧 epoch 在任何 mpv/降级/状态操作前非破坏性拒绝，前端刷新状态后重建候选而不显示 mpv 进程故障；新播放代次仍先重置 epoch，避免破坏换源。开关 IPC 失败会回滚到最后权威视频值并解除栅栏。该批只执行非媒体自动化和本地编译检查，未启动开发端、未播放真实媒体、未修改音频功能、未实施 4K，最终结果待用户实测。
- 2026-08-29 修复开发端首次开启视频处理的 epoch 握手阻断：全新 runtime 的 surface suspend 只推进 operation revision，不再生成缺少四维会话身份、调用方又无法获知的 `backend_epoch=1`；已有受管会话仍推进 backend epoch 并保留身份。挂起状态明确发布为 `Available + Stopped`，不再形成 `Source + Active + Stopped + process_id=None` 的矛盾事实。`set_processing_switches` 现在原子返回播放快照与权威视频后端状态，React 在唤醒周期调度前接收该状态；同一播放时钟身份遇到永久 prepare 错误后建立熔断，禁止 100ms 调度器持续重建同一失败候选，只有开关/换源/seek/loop 或用户手动重试解除。临时 prepare 诊断日志已删除。本批按用户要求只完成代码修复，未运行自动化、mpv 或真实视频测试，结论保持“待用户开发端实测”；未修改音频功能，未实施 4K。
- 2026-08-29 当前执行点（Phase 7A/7B）：现行固定锁 SHA-256 为 `cff165702cadadb3bcb220c90466e46933a40f47796b4cbd0c08fe4d352a5352`。新冷构建在全新 `volume-nocopy` 卷中按断网、只读根文件系统、`JOBS=2`、`4 GiB` 内存且无额外 swap 执行，已完成 FFmpeg 与 mpv 链接并开始生成 `patch-bundle.tar.zst`，随后宿主 Docker Desktop Linux Engine 管道再次消失，runner 以 Exit `1` 结束；本轮未完成证据报告、未提升候选、未替换正式资源。Phase 7B 的 Node 十一项原子发布链已完成代码总监整改并通过聚焦测试 `21/21`；Rust/Tauri 十一项/schema 2 直构建门禁的 P0/P1 整改也已完成，源码侧清单字节锚点、精确三组件集合及同时篡改负向测试通过，聚焦测试 `6/6`。当前没有运行新的媒体测试，没有重复测试用户视频，没有修改既有声音功能，也未实施 4K。
- 2026-08-29 完成视频导入卡顿根因修复及代码总监整改：新加入、追加和替换播放池媒体只执行现有有界 FFprobe，直接保留 `playback_reference=source_path`、`compatibility_mode=direct`；删除对每个视频执行完整 H.264/AAC 转码的 `media_compatibility` 模块、六小时兼容超时、临时产物 RAII 和播放池兼容缓存生命周期。最多 `100` 项、白名单/真实流/规范路径去重、取消代次、整批原子提交、失败保留旧池及 Tauri 资源授权保持不变；同一路径资源授权由两次收口为一次。旧 `media-compatibility` 目录只保留受限遗留垃圾清理。聚焦契约 `1/1`、资源授权 `4/4`、导入边界 `2/2`、取消代次 `1/1`、`cargo fmt --all -- --check`、`cargo check --all-targets -j 1` 和 `git diff --check` 通过；本批未运行媒体、未修改声音功能、未实施 4K。
- 已接入单一受管 mpv 进程的持久 Windows 命名管道 JSON IPC：单一 worker 独占底层句柄并串行完成写、刷新、响应读取；请求 ID、deadline、队列上限、响应大小上限、事件穿插、超时毒化、stderr 尾缓冲、媒体路径脱敏、退出与强制回收均已落地。Windows Job Object 也已接入，强制退出优先关闭带 `KILL_ON_JOB_CLOSE` 的唯一 Job 句柄，再保留 `taskkill/Child::kill` 作为最后兜底。
- 2026-08-29 补齐 Phase 2 进程树所有权：`ManagedMpvProcess` 在启动前创建已配置的 Job，mpv 启动后立即绑定；Job 创建失败不启动进程，绑定失败必须执行 `taskkill + Child::kill + wait` 后 fail-closed。正常 IPC quit 仍先有界等待，超时关闭 Job 终止受管树；正常退出也关闭 Job，以清理主进程先退而遗留的后代。项目继续保持 `unsafe_code = "forbid"`，Windows-only 精确锁定 `win32job 2.0.3` 的安全封装。纯测试进程树通过唯一临时目录握手，保证父进程绑定 Job 后才创建后代；关闭 Job 后以 PID+启动时间有界确认父与后代均退出，失败路径由 RAII 回收进程并删除握手目录。`cargo fmt --check`、`cargo check --all-targets`、进程树精确测试 `1/1` 和 `realtime_video_backend` 测试 `22/22` 通过，收尾夹具目录、测试进程及 mpv/FFmpeg 进程均为 0。本批未运行媒体；真实 mpv 崩溃和安装包退出仍留给阶段实机门禁。
- 2026-08-29 补齐 Phase 2/3B 的用户 seek 与循环对齐：前端已有的源内 `position_ms` 不再作为兼容占位被丢弃，而是进入唯一 runtime actor。重复 generation/epoch/loop 的普通同步即使位置值变化也不发送 seek；只有 `clock_epoch` 前进或 `loop_index` 单步前进时发送一次 `seek absolute+exact`。播放态转暂停并同时 seek 时固定为 `pause → seek`，暂停态恢复并同时 seek 时固定为 `seek → unpause`；旧 generation、倒退 epoch/loop 和跨越多轮继续 fail-closed，已知源时长存在时越界位置在命令边界拒绝。该变更不恢复普通视频周期硬 seek，也不使用 React 定时器驱动同步。runtime 单元测试 `16/16`、独立同步契约 `4/4`、命令接线契约 `1/1` 与 `cargo check --all-targets` 通过；未运行 mpv、FFmpeg 或视频，真实画面 seek/循环呈现仍待阶段实机门禁。
- 2026-08-29 补齐 Phase 2 的受管 Original 代码入口：视频处理关闭且播放态为 Playing/Paused 时，React 通过 `ensure_original_video_renderer` 把当前视频、源内位置、generation、clock epoch、loop 与暂停态交给同一个 Rust runtime actor；actor 复用现有无 shader `MpvLaunchSpec::new`、D3D11 输出、持久 IPC、Job Object 和停止所有权，不创建第二播放器。`source` 状态现在可如实携带原生 graphics API、decoder、固定 `Original（无 shader）` 标识和非空 PID；该模式不消耗 GPU→CPU4 单向降级状态，后续在同一播放代次开启 GPU83 仍可按正式入口重启为 shader 会话。Original 与 GPU83 共用现有 seek/loop/pause 同步；Original IPC 失败只标记 source 失败并释放进程，不误降级到 CPU4。启动失败暂时保留 WebView 兼容画面且每个 generation/source/epoch/loop 只尝试一次，避免无界重试；Phase 6 移除兼容表面前仍不得宣称最终原生表面已完成。Rust Original 状态测试 `2/2`、命令测试 `1/1`、mpv 入口契约 `4/4`、前端状态/Original 契约 `11/11` 和 TypeScript 检查通过；本批未运行 mpv、FFmpeg、真实视频、音频或 4K，真实基础播放仍待一次明确授权的实机门禁。
- 2026-08-29 完成 Phase 6 的 EOF/表面仲裁最小静态切片及代码总监整改（其中 `complete_playback_item` 前端推进协议属于历史实现，已由 2026-08-30 EOF 后端原子恢复决策覆盖）：受限 IPC 新增严格布尔 `eof-reached`，mpv 固定使用 `keep-open=yes` 且继续禁止 `loop-file=inf`；runtime 将 EOF 绑定到 `playback_generation/backend_epoch/clock_epoch/loop_index`，新会话、seek 与循环边界清零。EOF 成立后不再累计 PTS/FPS/帧预算健康失败，暂停态不发布或消费 EOF；单项循环提交后等待新 loop 状态期间保持同一原生表面。`get_media_video_backend_status` 仅扩展为 `main/final-effect` 两个播放窗口只读；最终效果窗只有在 Rust 状态、PID 与播放身份全部匹配且 active 时才释放可见 WebView video，失败或未确认继续兼容回退。前端关联契约 `56/56`、3 个 Rust EOF 精确测试、`cargo check --all-targets`、格式和 TypeScript 检查通过。该项只表示当时的“EOF/表面仲裁静态闭环完成，待 1080p 实机门禁”，不表示当前 EOF 原子恢复或 Phase 6 已完成，不承诺无卡顿；本批未运行媒体、未实施 4K，声音链未修改。
- 2026-08-29 补齐 Phase 2/4 的 stderr 故障入口与非媒体状态矩阵：继续复用 `ManagedMpvProcess` 既有有界、路径脱敏 stderr 尾缓冲，以无新依赖的严格分类识别 shader/hook 失败、VO/图形 API 初始化失败和标准设备丢失码；启动确认阶段立即拒绝，运行阶段由唯一 actor 进入既有单向降级，Original 遇到同类错误只发布终态失败。分类明确排除音频错误和正常的 Vulkan error-diffusion 描述，原因最多保留 512 字符。新增启动失败、shader 失败、设备丢失、断管、CPU4 过载、Original 恢复和并发 stop/seek 的状态机矩阵；Rust 库 `389/389`、`cargo check --all-targets` 通过。该项不替代真实设备丢失、恢复位置、黑屏和 P99 门禁；未运行媒体、未修改声音、未实施 4K。
- 2026-08-29 完成 Phase 6 原生表面生命周期的非媒体收口：runtime 现在单独记录每个活动 mpv 进程实际绑定的 final-effect HWND，GPU、CPU4 和 Original 只有在请求 HWND 精确相同时才允许重用；换窗口必须回收旧进程并重建 `--wid`。关闭最终效果窗改为 surface suspend，只取消在途启动并释放当前表面，不再把仍在播放的 generation 写入 stop tombstone，因此同一播放代次可在重开窗口后绑定新 HWND。`CloseRequested/Destroyed` 共用每窗口一次原子清理门，命令层不再提前重复清理。该项只证明状态、进程所有权与 HWND 绑定契约；真实原生画面、黑屏、EOF 和窗口重建仍需 1080p 实机门禁。本批未运行媒体，未改动音频处理语义，未实施 4K。
- 2026-08-29 完成受管 Original 专项代码总监复核并整改 3 个 P1、2 个 P2：GPU 启动/提交失败不再调用会重建状态机的完整停止，同一播放代次保持 GPU→CPU4→Original 单向降级；只有 Rust 权威 loop 快照追平后才占用 Original 尝试键；runtime 会观察 Original 子进程退出，前端解析同步返回状态并只在存在受管 PID 时每 2 秒读取一次 Rust 状态遥测；重复 ensure 必须通过 generation/epoch/loop 单调校验；GPU `available/active` 缺失合法 PID 时前端 fail-closed。该轮没有扩大参数能力、没有运行媒体，也没有修改音频。
- 已将 `prepare_realtime_video_plan` 从 WebView2/CSS 编译目标改为 GPU83 快照编译，并在 React 正式视频周期中优先调用；成功候选不再进入旧 MSE/逐周期转码，失败由 runtime 终止当前进程并保留本播放代次的降级状态，前端释放候选并保持 Original 兼容画面。
- 已建立精确且无重复的 83 项映射表；Phase 3A v6 双后端真实门禁通过后，61 项 shader 与固定哈希已原子提升，第二阶段把 18 项已有真实消费链的 PTS 调度字段接入。当前开发能力为 `79/83`；2 项颜色候选因跨属性帧级原子性和完整颜色合同尚未通过审核，2 项跨帧历史缺少历史纹理，四项均继续返回 `Unavailable`，因此不得宣称 GPU83 已完整接入。
- 视频运行状态已收口为 `realtime_gpu / cpu4 / source`；旧 `ffmpeg_gpu / ffmpeg_cpu`、WebView2 CSS 传输分支、编码器字段及其仅测试可达代码已删除，前端对旧状态 fail-closed。
- 已随包登记 `resources/shaders/gpu83.hook`，周期参数通过一条受限 `set_property glsl-shader-opts` 命令原子更新；本机随包 mpv `gpu-next + D3D11 + D3D11VA` 已真实加载 hook，shaderc 结果为 `0 errors, 0 warnings`。
- 已实现纯 Rust 音画同步控制器及代次/epoch 隔离、`20/60/80ms` 分级、迟帧策略和恢复请求；2026-08-29 已把修正收紧为 `20–60ms` 最多约 `±1%`、`60–80ms` 最多约 `±2%`。PortAudio 输出线程按 25Hz 发布只读可听时钟，携带 generation/audio epoch/source/loop、实际采样率、输出延迟和实际音频播放速率；视频侧仅消费身份匹配且不超过 160ms 的快照。最终 mpv speed 由“实际音频播放速率 `0.5–2.0` × 视频调度速度 `0.5–1.5` × 同步修正 `0.98–1.02`”在唯一入口合成，并由 `0.25–4.0` 安全值类型约束。mpv 的 `pause`、`seeking`、`paused-for-cache` 通过固定受限属性和严格布尔响应进入同一控制器，暂停、seek 和缓存等待期间只 Hold；恢复 epoch 由视频同步控制器唯一创建。该接线不改变普通声音 DSP、N/N+1、插话、固定话术或混音语义；真实联合媒体门禁仍未执行。
- 旧 MSE/SourceBuffer、视频 period/分块 IPC、逐周期 FFmpeg 视频 Worker、权限和对应测试已按用户授权删除；2026-08-29 又移除“所有视频导入先整段转码”的残留链，新导入只做 FFprobe 并直接使用源路径。普通声音候选、PortAudio、插话和固定话术保持不变。四档跨厂商 GPU 候选再到 CPU4、Original 的六级生产状态机、mpv EOF 播放池静态接线及非媒体验证已完成；真实 1080p 故障恢复、EOF 时序、最终表面整机冒烟和长稳测试仍未完成。
- Phase 1 已新增可重复执行的 Windows 实机门禁 `desktop/tools/verify-mpv-phase1.mjs`、固定 CPU4 `eq+hue` 四字段命令编译和 mpv 发布资源/哈希/许可证材料门禁。最终代码总监报告为 [`artifacts/mpv-phase1-code-director-audit-20260828-v8-1080p.json`](../../../artifacts/mpv-phase1-code-director-audit-20260828-v8-1080p.json)：D3D11/Vulkan 的 1080p 24/25/30/50/60fps 与 CPU4 1080p60 全部通过；GPU 每路径 200 次参数更新、CPU4 160 次四字段快照均保持同 PID，逐帧丢帧时间线增量为零，帧预算和 shader 缓存契约通过。当前最高验收分辨率固定为 `1920×1080`，4K 已按用户要求延后。第三方法律材料、跨硬件运行时性能降级和长稳仍未完成，因此只证明当前机器和当前矩阵，不作所有硬件“绝对不卡顿”的承诺。
- 删除后本地验证已继续收口：Rust `cargo fmt --check`、`cargo check --all-targets`、桌面命令测试、mpv 入口契约和前端视频运行时专项通过；Rust 库现为 `378/378`。此前唯一失败的 StandardMp4/Vulkan 用例会手工强制生产入口不会选择的“部分 GPU83 快照 + 旧 Vulkan 离线路径”；仅测试可达的旧五字段 Vulkan helper 已删除，用例改为验证该非法组合必须 fail-closed。前端全套最近一次记录仍为 `392/393`，剩余失败只涉及 PortAudio 设备列表无效响应的既有断言；本轮未修改音频链。
- 后续按“Phase 3A 候选逐项晋级 → Phase 4 一次性 1080p 实机门禁 → Phase 5 PortAudio 闭环 → 联合长稳”推进。Phase 4 的生产状态机代码已接通，但实机门禁未通过前不得宣称无卡顿；音频改动只允许在 Phase 5 增加只读时钟快照、同步 epoch 和控制接线，不修改普通声音 DSP、N/N+1、插话、固定话术或混音语义。
- Phase 3A 候选已继续实施但尚未生产提升：`desktop/tools/shader-candidates/gpu83-baseline-candidate.hook` 修复现有 20 项基线中的 YIQ hue 矩阵列主序、饱和度旧 luma 和无几何变化时裁剪回混问题；独立 fragment 候选已覆盖图像修复两字段、抽象几何脸三字段、局部模糊的启用/区域/半径三字段、边缘填充启用/羽化两字段、通道偏移一字段，以及目标/核心频率、波形、动态均衡、空间维度、频率空间 XY 和 12 个固定频段组成的 21 字段共享空间调制 pass。抽象脸使用 UV 直接定位最多 10 个确定性几何槽位，每像素最多计算一个脸部 SDF，不做人脸检测；局部模糊按归一化 X/Y 百分比定义中心区域，使用固定九点二维 tent 核、区域内采样约束、边缘羽化并保留原始 alpha；边缘填充复用旧 FFmpeg 的明确语义，`0%` 为 2px 硬填充，非零羽化按 `2–16px` 有界宽度和百分比混合，每个边缘像素只增加一次同帧内侧采样；通道偏移把 `-10–10%` 直接归一化到浮点 RGB，正值增红减蓝、负值相反、绿色和 alpha 不变，不复制旧 8-bit code-value 抖动。空间调制只采样当前像素：可选频率以 `0Hz` 表示 `None`，`65–20000Hz` 对数映射到 `1–32` 个空间周期，12 个固定频段分别以 `weight-1` 进入组合且周期常量预计算；一维为 X、二维为 X+Y、三维使用同帧径向分量，不读取 PTS 或音频频谱。可用短表达式的候选通过动态 `WHEN` 跳过中性 pass；基线和空间调制保持固定预编译 pass，空间调制在 GLSL 内部于载波计算前执行中性早退，避免本次固定 libplacebo 对超长 `WHEN` 表达式解析失败。局部模糊间隔和任何时间调制仍留给 Phase 3B。候选不进入 Tauri bundle，生产 shader、固定哈希和 `20/83` capability 保持不变。色彩空间转换因缺少源/目标 primaries、TRC、范围和输出标签契约继续 `Unavailable`，不得用硬编码色偏矩阵冒充。
- 已新增并收紧 `verify-mpv-phase3a-candidates.mjs`、固定 9 文件候选契约和暂停同 PTS 的 FFmpeg 下采样 RGB 差异门禁。候选快照严格区分 61 项产品语义参数与 11 项非产品 runtime 门控选项，共 72 个有限值。每个后端只允许一个 mpv 进程：先以零 shader 启动并截取无 shader 参考帧，再在 `sessionSetup` 中按固定顺序通过 mpv 原生 `change-list glsl-shaders append` 一次性追加 9 个候选 shader，提交 72 键全激活快照并截取独立预热帧，使动态 `WHEN` pass 的首次编译与渲染发生在日志基线之前；随后恢复全默认快照并截取默认帧。三张 setup 截图必须保持同一 PTS，只有无 shader/全默认两帧进入中性差异分析。整个 setup 返回后才建立编译日志基线，之后的逐场景截图和 P99 更新若再次出现编译日志则失败。61 个产品字段中 6 个布尔开关使用完整 72 键 `baseline/active` 二态，55 个连续或离散值使用 `baseline/low/active` 三档；low 档允许以覆盖率证明广泛的一码值响应，active 仍执行原明显差异阈值，目标/核心频率和动态均衡阈值只要求 low/active 均有明显响应而不伪造强度单调性。11 个 runtime 独立场景和 1 个全激活组合使用两档，共 73 个逻辑场景；另有 1 个“PIP 开、随机图形关、只改变 seed”的预期相同隔离场景。逐场景共捕获 203 帧，每张截图都必须保持同一有限 `time-pos/full`。性能阶段只在完整默认与 72 键全激活组合间交替。隐藏窗口固定 `geometry`，使用无边框、未缩放视频、禁用 HiDPI 窗口缩放和自动窗口重设；不得同时使用会按工作区缩小窗口的 `autofit-larger`，也不使用会随显示器分辨率放大的 fullscreen，实际 OSD 表面仍必须精确为 `1920×1080`。依据 mpv 官方 `vo-passes` 契约，pass 数量可以逐帧变化，单个 pass 的 `count` 是该 pass 自身样本数而不是共享帧号；因此 P99 使用有序 `pass index/desc/count/samples` 组成统计快照身份，只跳过与上一份完全相同的快照，不再错误要求所有 pass count 相等或全局单调，同时仍要求至少 100 份相邻不重复的有效统计快照。PID 在 setup/probe/更新前后必须非空且一致，截图探针和性能阶段任一 VO/decoder 丢帧增长或计数重置均失败；失败 catch 同时保留有界 stdout/stderr，避免丢失 Vulkan 初始化原因。当前相关纯 Node/源码静态门禁为 `70/70`；非精确目标表面上原本通过的计时证据必须降为 `unverified` 并保留原始 timing，原本 `failed/unverified` 的证据不得被降级或抬升；Phase 1 与 Phase 3A 报告都只有在最终状态通过且全部 GPU 源/实际表面分辨率匹配时才允许填写最高 `1920×1080`，否则必须为 `null`。
- 本机隐藏实机报告 [`artifacts/mpv-phase3a-candidates-20260828-v2.json`](../../../artifacts/mpv-phase3a-candidates-20260828-v2.json) 状态为 `failed`，不得作为生产提升证据：D3D11/D3D11VA 同 PID、零新增 VO/decoder 丢帧，组合渲染 P99 `7.663ms`；抽象脸 3 项、局部模糊 3 项、边缘填充 2 项和通道偏移 1 项共 `9/32` 取得逐字段同帧差异。空间调制因超长 `WHEN` 解析失败而 21 项无帧差，图像修复 35% 场景最大仅 1 级通道差，Vulkan/WinVK 在本机未能建立 VO。报告后已把空间调制改为固定 pass 内部早退，并把图像修复明显对照改为 100%；按“不重复播放测试”约束只做静态整改，尚未重新实机验收。因此 D3D11 的 `9/32` 只能作为部分候选证据，Vulkan 和整改后 23 项仍待一次明确批准的后续隐藏验收，生产能力继续为 `20/83`。
- v2 使用旧报告口径，虽然总状态为 `failed`，其 `claims.maximumValidatedResolution` 仍错误保留了 `1920x1080`。该字段与总状态冲突，按当前 fail-closed 契约明确作废；只能引用 v2 的局部诊断数据，不能引用其最高分辨率声明。v3/v4 及当前脚本在总状态未通过时均把该字段固定为 `null`，历史报告不回写、不伪造成新证据。
- v2 报告后新增 9 字段同帧叠加候选：随机图形启用/数量/透明度/像素大小、共享挂件偏移，以及画中画启用/比例/透明度/旋转。随机图形以 8×4 网格直接定位至多 32 个确定性伪随机槽，每像素只判断一个槽；画中画只在旋转区域内从同一 `HOOKED` 当前帧增加一次采样，修正了先按未旋转矩形裁剪导致的旋转四角缺失。Phase 3B 在该文件本地声明 PIP X/Y jitter 与计划级随机图形 seed 三项 runtime 选项：PIP 只平移最多 ±4px，seed 只改变随机图形槽位的位置、形状和颜色，两者互不影响且都不读取循环、PTS、历史或音频；`picture_in_picture_timeline_locked=false` 仍保留 Phase 3C 历史时间轴。当前 61 项产品参数加 11 项 runtime 选项已执行一次隐藏 v3 门禁，报告为 [`artifacts/mpv-phase3a-candidates-20260828-v3.json`](../../../artifacts/mpv-phase3a-candidates-20260828-v3.json)，状态仍为 `failed`：D3D11 源与解码输出均为 `1920×1080`，但旧隐藏窗口被缩为 `1777×1000`，因此该表面上的组合 P99 `8.779ms`、118 份相邻不重复样本、同 PID、零新增丢帧、无重复编译和默认中性通过都不能作为 1080p 证据；74 个场景中 51 个通过、23 个失败，Vulkan/WinVK 仍未建立 VO。v3 后已静态修复窗口契约、19 项门禁假阴性，以及图像修复两字段、`wave_level`、`runtime_frame_inner_active` 四项真实弱响应：图像修复移除额外 `0.35` 衰减但仍固定五次边缘感知采样，后两项把最大响应从 1 提升到有界 4 码值；整改后未再次运行视频。候选继续不打包，颜色空间两项继续不可用，生产能力仍为 `20/83`。
- 用户单次授权的整改后 v4 门禁报告为 [`artifacts/mpv-phase3a-candidates-20260828-v4.json`](../../../artifacts/mpv-phase3a-candidates-20260828-v4.json)，状态仍为 `failed`，不得推广：D3D11 与 Vulkan/WinVK 均成功建立 `gpu-next`、保持同 PID、默认中性且更新期无重复编译，证明 Vulkan 已从“不可用”推进到可执行；但 `window-minimized=yes` 把两后端实际 OSD 表面都压成 `160×28`，所以 D3D11 的原始 P99 `3.619ms/118` 样本和 Vulkan 的 `0.089ms/117` 样本只作诊断，最高验证分辨率仍为 `null`。D3D11 丢帧计数从 1 重置为 0，继续按 fail-closed 判失败；D3D11 还剩 `al_runtime_frame_inner_active`，Vulkan 还剩 `al_overlay_offset_px` 与该 runtime 场景未通过。报告后已移除最小化启动和与固定 `geometry` 冲突的 `autofit-larger=100%x100%`，改用 `focus-on=never + show-in-taskbar=no` 的非抢焦点受控窗口并保留精确 `1920×1080` 表面硬门禁；位移参数改为只要求 low/active 均有可测响应，不再伪造差异幅度单调性；稀疏 frame-inner 增量从 4 提升为固定 16 码值。静态门禁通过，尚未再次运行视频。生产 shader、`20/83` capability、CPU4 与音频均未修改。
- v4 后 D3D11 计数重置已按根因收口：短样本会让 120 次更新跨越 `loop-file=inf` 边界，旧估算预算已从当前上下文删除。Phase 3A 现在只生成一个固定 `15s/900 帧` 的 `1920×1080@60` 专用样本；静态报告只声明样本窗口和至少两秒的运行时余量要求，不再用估算时延伪造剩余帧数。真实门禁在 setup/probe 后记录每次更新的 `time-pos/full` 与 Node 单调时钟，要求 PTS 每次严格推进、无 loop/reset、相对墙钟新增停顿不超过一个 60fps 帧，并以更新结束时实际 PTS 证明样本仍剩至少 `2s`；任一条件不满足即 fail-closed。
- 为防止未经逐次批准重复运行真实视频门禁，Phase 3A CLI 现在必须同时提供 `--confirm-real-media-gate` 与一个新的 `--report` 路径；缺任一项以退出码 2 在样本生成前拒绝。通过参数校验后，入口会在调用 `runPhase3aCandidateGate` 前以 Node `open('wx')` 原子预占新报告文件；同名路径因此会在任何样本生成或媒体启动前拒绝，测试完成后再由同一句柄写入结果。无媒体 CLI 验证确认拒绝路径不启动 mpv/FFmpeg，完整桌面工具静态测试为 `109/109`。
- 2026-08-29 Phase 3A 实机前静态门禁整改完成：61 项产品场景的 baseline/low/active 使用同一依赖上下文且只允许目标字段变化，`product_response` 同时要求 low 与 active 可区分；frame-inner 固定为 `2×2/8×8` 稀疏掩码，纯数学 `1920×1080 → 320×180` area 下采样证明 `56.25%` 像素仍跨过两码值阈值。真实门禁直接解析 Rust 83 项映射，逐项核对 20 个 `AVAILABLE` shader option 与已审计生产 shader；只保持数量不变而交换字段也必须拒绝。相关 7 文件 Node/源码静态集为 `81/81`。这只清除了实机前静态阻断：候选仍未进入生产，生产 capability 仍为 `20/83`，D3D11/Vulkan 真实 1080p60 门禁仍需一次新的明确授权；本批未运行媒体、未修改音频、未实施 4K。
- 2026-08-29 获得新授权后执行 Phase 3A v5 双后端门禁，报告为 [`artifacts/mpv-phase3a-candidates-20260829-v5-1080p.json`](../../../artifacts/mpv-phase3a-candidates-20260829-v5-1080p.json)，状态 `failed`。D3D11/Vulkan 均保持同一 PID、零新增丢帧且默认中性、无重复编译和 PTS 连续性通过；诊断计时中 D3D11 GPU P99 `6.322ms`、Vulkan `0.787ms`。但 Windows 工作区高度只有 `1032px`，普通无边框窗口按比例受限为 `1834×1032`，因此这些计时不能作为 1080p 通过证据；另有 6 项场景因反向强度、周期/离散响应或低档量化为默认而失败。随后仅完成静态整改：Phase 3A 专用门禁显式使用当前屏幕全屏客户区，普通隐藏窗口和 Phase 1 其他分辨率不变；PIP 透明度改为按视觉强度排序的 `90→50`，局部模糊低档改为 `3px`，核心频率使用可测的 `500→65Hz` 双响应，grain/空间维度改为非单调响应门，动态阈值低档改为 `2`。相关定向测试 `65/65` 通过，整改后尚未再次运行媒体；生产 capability 继续为 `20/83`，4K 未实施。
- 2026-08-29 用户授权后只执行一次整改后的 Phase 3A v6 双后端门禁，报告为 [`artifacts/mpv-phase3a-candidates-20260829-v6-1080p.json`](../../../artifacts/mpv-phase3a-candidates-20260829-v6-1080p.json)，状态 `passed`。D3D11 与 Vulkan 的源、解码输出和实际 OSD 表面均为 `1920×1080`；61 个产品参数场景、11 个 runtime 门控、默认中性、同 PID、PTS 连续、零新增丢帧和更新期无重复 shader 编译全部通过。D3D11 组合 GPU P99 为 `4.26ms`、参数更新 P99 为 `17.42ms`；Vulkan 分别为 `0.89ms` 和 `17.09ms`。这份报告只准入候选集合，报告生成时生产 capability 仍为 `20/83`；生产 shader、Rust 映射、固定哈希和 Phase 3C 剩余历史字段必须作为同一原子变更提升并再次通过静态门禁后，才可表述为生产 `61/83`。
- 2026-08-29 已完成 v6 后的原子生产提升：61 项候选实现合并为单一生产 shader，Rust capability 同步更新为 `61/83`，11 项 runtime 选项继续作为调度器内部选项而不计入 83 个产品字段；生产 shader 归一化 SHA-256 为 `3e08e25507a063d405807184f4eaf51e5da77e03df72acba322e4a44e01b6f9f`。Phase 1/3A/3C 静态契约 `76/76` 通过，Phase 3C 仍为 `not_admitted`；剩余 19 项调度、2 项颜色管理和 1 项跨帧历史没有被本次提升伪装成已实现。
- Phase 3B 已落地第一批纯 Rust 媒体 PTS 调度核心 `media_video_frame_scheduler.rs`：以 `schedule_epoch`、播放代次、参数序列和确定性 SplitMix64 种子生成帧率目标/基础视频速度、帧内/帧间/概率门控、切片、局部模糊间隔、高光脉冲、PIP 像素扰动、异步旋转与变换平滑状态。首次启动和新 epoch 必须携带边界；暂停 PTS 偷跑、旧 generation/epoch/sequence 和倒退 PTS 均 fail-closed 且不污染后续合法状态；同一源帧的 PIP 扰动不随控制器采样频率变化；同 PTS 到达的新参数序列立即应用并推进身份，旧序列不能回写。调度输入已收窄为纯视频与高级视觉参数，无效音频参数不能阻断视频调度。模块不读取墙钟、不自行执行 mpv IPC；内部测试 `9/9`、crate 外契约 `9/9` 通过。
- Phase 3B/5 的生产受限 mpv 原语只读取当前锁定二进制属性清单明确提供的 `time-pos`、`estimated-vf-fps`、`pause`、`seeking` 和 `paused-for-cache`，只向 `speed` 写入有限 `0.25–4.0` 合成结果；不开放任意 property 名、脚本或命令。`time-pos` 的双精度秒值在 Rust 边界四舍五入为毫秒。颜色候选虽已验证固定版本存在 `video-params`/`video-target-params` 与 `libplacebo-opts`，但跨属性帧级原子性尚无生产方案，不能进入周期写入口。Phase 3A 独立候选工具保留其已验证的同 PTS 探针，不作为生产运行时属性契约。开发生产入口保持 `79/83`：61 项 shader 加 18 项 PTS 调度；2 项颜色和 2 项历史继续不可激活。
- 2026-08-29 本机锁定 mpv 的 `vo=null` 命名管道探针连续读取 20 次 `time-pos`：中位数 `0.307ms`、最大值（会话首响应）`101.51ms`。生产观测仍按 `40ms` worker 节拍执行，单命令上限为 `250ms`，同一 tick 的所有 IPC 共享 `350ms` 总预算；复合 PTS/FPS 和 pause/seeking/cache 读取也共享调用方交给它们的总预算，避免遥测长期占有 actor。
- 生产运行时的启动、健康观察和普通控制路径只把 `IpcQueueFull` 与精确属性暂不可用视为可恢复观测失败；这些路径中的 `IpcTimeout` 仍会毒化管道会话，后续拒绝复用。唯一例外是 `sync_realtime_video_renderer` 的命令确认超时：该路径返回 `result_unknown`、保留当前 mpv 会话，并由调用方先读状态确认或按同一 desired 幂等重试，不能仅因确认超时推动 GPU→CPU4。`IpcDisconnected`、断管、进程退出、乱序、非事件缺少 request ID、协议错误、mpv 渲染错误与已确认失败的 seek/pause/speed 仍 fail-closed，并沿用既有单向降级；每秒五项帧健康读取保留独立连续失败计数，普通 PTS 成功不能掩盖它。
- mpv 初始化使用一个 `5s` 总预算。GPU83/CPU4 要求 `vo-configured=true`、`vo-passes` 已存在真实样本和 `hwdec-current` 可读；Original 在 `vo-configured=true` 后只要求有效视频 PTS，不再把仅供调度的源 FPS 当作就绪条件，随后最佳努力读取 `hwdec-current`。每次查询取得剩余总预算，避免首个 HEVC 解码/VO 查询被固定 `500ms` 提前截断；只有精确 `PropertyUnavailable` 或成功但尚未就绪的值可以轮询，传输/协议错误立即失败。命令返回前还要按 mpv PID 证明真实视频子窗口已挂入专用宿主。
- Phase 3B 第三批历史实现记录（其中逐条 `15ms` 与生产 `20/83` 已被上述 `250ms/350ms` 和当前 `61/83` 口径取代）：该批把 mpv 进程、持久 IPC、调度器和活动计划收口到单一 Rust actor，控制邮箱容量固定为 8，空闲 `40ms` 执行至多一次观察 tick，不累计 timer；FPS 在每个 epoch 首次有效观察后冻结，播放速度相同则不重复写入。React 只透传已有 `clock_epoch/loop_index`，Rust 将启动、seek 和单步循环转换为内部调度 epoch；旧代次、旧 epoch、倒退循环和跨越多轮循环均拒绝，循环与 seek 同时发生时循环边界优先。命令层不再用 `Arc<Mutex<RealtimeVideoRuntime>>` 包住 IPC，停止响应在共享状态发布为 `Stopped` 后才返回；播放池原有转换锁与声音停止顺序保持不变。该批次的 GPU→CPU4、动态 shader、生产提升和同步缺口已由本文后续阶段分别覆盖或保留为当前未完成项；本批未运行 mpv/视频且未修改音频。
- Phase 3B 第四批完成候选门控、纯快照编译和 actor 有序批次，但不晋级生产 capability：10 项 runtime 选项按 `7+1+2` 分布，`gpu83-scheduler-gates-candidate.hook` 只声明帧内/帧间/概率/切片/高光/异步旋转/平滑 7 项，局部模糊文件本地声明自己的 runtime gate，同帧叠加文件本地声明 PIP X/Y jitter，跨文件没有重复或未声明参数。scheduler pass 使用固定最多 3 次当前帧采样：帧内为固定空间稀疏 ±4 码值、帧间为同帧邻点混合、概率为固定同像素棋盘扰动、高光为 +1 码值，切片为平滑权重控制的固定 2px 中带位移，异步旋转与平滑只组合当前帧坐标；不使用片段随机数，不读取墙钟、PTS、历史或音频，不含循环或无界采样并保留源 alpha。局部模糊 runtime gate 只开关既有九点 pass；PIP jitter 只平移既有 PIP。Rust 在任一 shader 调度组启用时生成“基础快照 + 固定 10 键”的完整快照，关闭门显式写 0；actor 只按“变化后的完整 shader 快照 → 变化后的最终 speed”顺序各发送至多一次并去重。默认参数与当前 20 项快照逐字节一致；runtime 自身再次拒绝含未接入活动项的计划，所以当前 capability 下该动态批次不可达。生产 `resources/shaders/gpu83.hook`、Rust capability、音频和发布资源均未修改，本批没有运行 mpv/视频。随机图形候选仍使用固定槽位散列，尚未接入方案要求的计划级确定性 seed，因此也不得晋级。
- Phase 3B 第五批补齐随机图形的计划级确定性 seed，但仍不晋级生产 capability：Rust 从调用方计划 seed、`session_id`、`playback_generation`、`sequence` 和独立字段 ID 通过 SplitMix64 派生 `0..=16777215` 的 24 位整数；同一计划跨帧、seek 和循环保持不变，计划身份或输入 seed 改变时按新计划重派生。该值可被 f32/f64 精确表示，候选 shader 先拆成高低 12 位有界分量，再只参与随机图形六个固定槽位 hash；seed 不进入 PIP，PIP jitter 也不进入随机图形。runtime 契约因此扩展为 `7+1+3=11` 项，完整候选快照为 72 键；当前验收契约是 55 个产品三档场景、6 个产品开关二态场景、11 个 runtime 两档场景、1 个全激活组合和 1 个 seed/PIP 预期相同隔离场景，而不是旧的 53 场景口径。Rust 保留旧 10 键相对顺序并在 PIP Y 后追加 seed，随机图形关闭时显式写 0，默认参数仍与基础生产快照逐字节一致。生产 shader、capability、音频和发布资源未修改，runtime 的未接入活动项门禁仍使该候选路径不可达；本批只完成 Rust、Node 与源码静态验证，没有运行 mpv、FFmpeg 或视频。
- 2026-08-29 Phase 3C 已完成“是否允许进入 Render API 实施”的非媒体准入审计，但结论是当前方案**不准入跨帧历史**。`desktop/tools/verify-mpv-phase3c-admission.mjs` 固定核对随包清单、受控 Cargo/Rust/运行资源声明、外部 mpv JSON IPC、生产/候选 shader 与 83 项 capability 的真实常量/宏/身份：当前资产只有 `mpv.exe`，没有 libmpv DLL/导入库/头文件，主 crate 保持 `unsafe_code = "forbid"`，生产 shader 哈希及 20 个 AVAILABLE 选项逐项匹配，15 个 `HISTORY` 字段继续 `requires_history_or_secondary_texture`。`picture_in_picture_timeline_locked` 另有组合门禁：即使未来调度能力通过，时间轴解锁仍必须同时具备独立源时间轴与有界历史纹理证据。固定源码引用 `7b8915bc1d` 的[公开 Render API 头文件](https://github.com/mpv-player/mpv/blob/7b8915bc1d/include/mpv/render.h)经外部主源复核只声明 OpenGL 与软件后端；仓库没有本地头文件，报告明确标记该证据不是本地头文件校验。外部 mpv 可选择 D3D11/Vulkan VO 不等于公开的 D3D11/Vulkan libmpv Render API。报告顶层固定为 `status/admissionStatus=not_admitted`、`checkStatus=passed`、`phase3cImplemented=false`，避免把检查成功误读成功能准入；专项负向测试已进入桌面默认静态测试。三轮代码总监复审累计验证 47 个反例（44 个危险变体 fail-closed、3 个合法场景允许），最终 P0/P1/P2 均为 0；[审计证据](../../../artifacts/mpv-phase3c-code-director-audit-20260829.json)确认“实机前静态阻断已清零”，但不改变功能未准入结论。本批未启动媒体、未触碰音频、未实施 4K。

## 1. 决策

正式视频主链改为：

```text
只读 source_path
  → 常驻 mpv 播放器
  → Windows D3D11VA 硬件解码（软件解码后备）
  → mpv gpu-next
  → mpv 内置 libplacebo 自定义 shader / GPU 合成
  → 最终效果窗口原生视频表面
                ↑
       每周期一次原子 GPU83 参数快照

PortAudio 实际可听 PTS
  → Rust 音画同步控制器
  → mpv 视频速度、丢帧或必要时精确定位
```

GPU 主链失败时按会话单向降级：

```text
D3D11 + D3D11VA + gpu-next/libplacebo
  → Vulkan + WinVK + gpu-next/libplacebo
  → mpv 常驻播放 + libavfilter CPU4
  → Original
```

核心规则：

1. 复用仓库已有随包 `mpv`、受管子进程、Win32 `wid` 和 JSON IPC，不安装新的 Tauri mpv 插件，不首批引入 libmpv/libplacebo FFI、GStreamer、wgpu 或 WebGPU。
2. `gpu-next` 是 libplacebo 的实际接入点；不把“使用 mpv”与“libplacebo 已执行 GPU83”混为一谈，只有精确 shader 探测和真实帧证据成功才报告 GPU83。
3. 视频周期只更新参数，不重新解码、编码、落盘、切换 `<video>.src`、重建 MediaSource、重启 mpv 或重新编译 shader。
4. GPU83 更新失败时保持上一组已生效快照或 Original，不阻塞渲染线程，不因单个参数错误切换后端。
5. GPU/CPU 后端只允许会话级单向降级；正常周期不得在 GPU、CPU4 和 Original 之间反复切换。
6. CPU4 只执行亮度、对比度、饱和度和色相；其余 79 项不得进入 CPU。
7. PortAudio 的实际可听 PTS 是音频主时钟；视频跟随音频，React 不承担实时同步控制。
8. 当前版本仍不开发实时话术幻化、RTMP/OBS、直播平台接入、服务端媒体任务或永久媒体版本。

## 2. 目标与非目标

### 2.1 目标

- GPU 可用时，除水平/垂直翻转外的 83 个视频 UI 参数在同一计划快照中原子参加 GPU 实时播放主链。
- 正常周期边界不产生新增黑帧、播放器重启、源切换、时钟归零或超过一个视频帧的边界相关停顿。
- GPU 不可用或精确 GPU83 探测失败时，CPU 只处理 4 个基础参数；CPU4 仍不能维持播放时直接保留 Original。
- 音画播放偏差最大不超过 `80ms`，P99 目标不超过 `40ms`。
- 播放池最多 100 项、单窗口、顺序循环、单项自循环、暂停/恢复/seek/换源和纯音频行为保持现有产品契约。
- 运行状态如实区分播放器、解码器、GPU API、shader、CPU4 和 Original，不根据编码器名称或字段数量推断效果已经生效。

### 2.2 非目标

- 不把逐 period FFmpeg 重编码或 MSE 继续作为正式视频处理主链。
- 不在首批直接绑定 libmpv Render API；现有 mpv 子进程方案通过验收后才评估是否需要升级。
- 不自研解封装、视频解码、交换链、播放器时钟或通用 shader 框架。
- 不为运行时 GPU 故障预启动第二个隐藏播放器；故障恢复允许一次有界会话重建，但正常周期不得重建。
- 不恢复已删除的旧 MSE/FFmpeg 视频周期链或视频导入转码；FFmpeg 只在普通声音等既有边界继续保留。

## 3. 当前事实与必须先修复的问题

1. 最终效果窗口在受管 mpv 的 active PID 与四维播放身份尚未确认时保留 WebView 兼容画面；确认后移除视频源并交由原生表面显示。该兼容回退不是正式视频处理链，最终原生表面、EOF 时序和播放控制仍需 1080p 整机验收。
2. 实时视频命令已改为 GPU83 快照编译目标，并通过受管 mpv 的单一持久 IPC 更新；旧 WebView2 CSS 编译目标不再由该命令调用。
3. 精确 83 项映射已建立，当前生产开发能力为 `79/83`：61 项已有 D3D11/Vulkan 1080p60 shader 证据，18 项进入受限 PTS 调度；2 项颜色和 2 项跨帧历史继续 fail-closed。字段存在、代码可达或旧阶段的 `20/83`、`61/83` 记录都不能冒充当前 `83/83` 已完整生效。
4. 受控 mpv hook 已在本机 D3D11 与 Vulkan 路径完成 61 项精确 1080p60 候选门禁并提升生产；不同厂商 GPU、完整 `79/83` 组合、故障恢复和长稳仍待验证。
5. shader 与调度器使用 mpv 提供的 PTS，不再按固定 `30fps` 计算；18 项已验证调度字段进入受限生产入口，其余 4 项仍不得激活。
6. 旧 GPU 文件链/MSE 生产代码、IPC、权限和测试已删除；版本控制历史不得重新成为当前实现依赖。
7. 音画同步控制律、PortAudio 可听时钟与 mpv 观测/控制代码已接通，并在 2026-08-30 收口统一媒体段身份和 source-local PTS；真实 PortAudio/mpv 联合媒体、设备恢复与 30 分钟长稳尚未执行，因而仍不能声称满足 `80ms` 门槛。

## 4. GPU83 执行契约

### 4.1 83 项定义

- UI 共 85 个视频显示项：32 个普通视频项和 53 个高级视觉显示项。
- 水平翻转和垂直翻转固定排除自动周期，GPU83 为其余 83 个显示项。
- `advanced.band_weights` 在模型中是一个 Map，在 UI 和验收证据中按 12 个固定频率拆成 12 项。
- 83 项必须维护唯一映射表；每项记录字段、单位、中性值、范围、执行类别、shader/调度目标、验真方法和失败语义。

### 4.2 执行类别

GPU83 表示 83 项全部进入 GPU 实时播放主链，但不把所有参数伪装成单个 fragment shader：

| 类别 | 执行位置 | 参数示例 | 成功证据 |
|---|---|---|---|
| GPU 像素效果 | libplacebo 单帧/多 pass shader | 模糊、锐化、噪声、色彩、旋转、裁剪、暗角 | 精确 shader 探测、非中性帧差、呈现确认 |
| GPU 合成效果 | libplacebo 多 pass/纹理合成 | 画中画、局部模糊、随机图形、边缘填充 | 组合帧差、位置/数量/透明度语义验证 |
| 播放调度 | mpv 视频时钟与帧调度 | 帧率锁定、帧率扰动、丢帧/重复帧 | PTS、呈现间隔和丢帧计数验证 |

若某字段只能写入参数列表、宏或状态，但不能产生对应语义，则该字段状态必须是“正式需求·待实现/未接入”，整份快照不得报告 GPU83 完整生效。

### 4.3 shader 生命周期

- shader 文件位于受控应用资源或会话缓存目录，不接受前端传入任意 shader 路径或源代码。
- mpv 生产启动参数必须显式包含 `--glsl-shaders=<受控完整路径>`；只存在 shader 文件、只启用 `gpu-next` 或只观察到 `vo-configured` 都不证明 GPU83 已加载。
- 使用 mpv hook 的 `//!PARAM` 声明动态参数；所有常用效果 pass 在会话启动时一次性编译，周期只更新值和启用标志。
- 一份周期快照序列化为一次 `glsl-shader-opts` 属性更新；禁止逐项发送 83 条 IPC，也禁止在周期边界增删 shader 文件或重建滤镜图。
- 参数提交前完成有限数、范围、组合、字段完整性和重复字段校验；失败保持当前快照。
- mpv 命令成功且视频时钟越过目标位置后的首个呈现帧，才允许提交本轮实际参数和变化次数。

### 4.4 GPU 能力探测

能力探测必须使用与生产一致的完整 shader 和非中性短样本，不再只探测基础 libplacebo：

1. 固定随包 mpv 版本和资源校验通过；
2. 创建受管 mpv 进程和最终窗口宿主；
3. 首选 `--vo=gpu-next --gpu-api=d3d11 --gpu-context=d3d11 --hwdec=d3d11va`；
4. D3D11 失败后尝试 `--gpu-api=vulkan --gpu-context=winvk`，允许其使用明确的复制型硬解后备；
5. 加载完整 shader，确认 `vo-configured`、首帧呈现、参数被真实消费，且日志无 hook/编译/执行禁用错误；
6. 应用非中性快照并验证输出帧差及帧预算；
7. 任一步失败都不得报告 GPU83 可用。

## 5. CPU4 与 Original

### 5.1 CPU4

CPU4 使用常驻 mpv 的 libavfilter 软件视频滤镜，只接受：

- `brightness_percent`
- `contrast_percent`
- `saturation_percent`
- `hue_rotation_degrees`

首选已成熟且支持运行时命令的 `eq + hue`；实施前必须以随包 mpv 的 `vf=help` 和真实短样本确认滤镜、命令和范围可用。若随包构建缺少 `eq`，应调整固定 mpv 资源构建或使用经过同等动态命令验证的成熟 libavfilter 组合，不自研逐像素 CPU 工具。

CPU4 更新必须满足：

- 不重启 mpv；
- 不重新编码；
- 不在周期边界替换完整滤镜图；
- 一份计划只更新这 4 项；
- 运行速度低于实时帧预算时直接降级 Original。

### 5.2 Original

- 视频处理关闭时直接使用 mpv 播放 Original，不建立视频 period、MSE 或视频效果临时文件。
- GPU83 和 CPU4 启动探测均失败时，mpv 继续播放 Original。
- 新导入的 GPU83、CPU4 和 Original 都只读取 `source_path`；FFprobe 成功后固定 `playback_reference=source_path`、`compatibility_mode=direct`。历史快照中不同于源路径的 `playback_reference` 只为向后读取保留，不再生成新兼容产物。
- 播放池导入继续保持逐项有界探测、取消、规范路径去重和整批原子提交，但不再启动 FFmpeg 视频编码器，也不创建 `.partial/.ready` 或一次性兼容 MP4。

## 6. 音画同步

### 6.1 权威时钟

- 音频和视频共享不可部分匹配的媒体段身份：`playback_generation + source_path + loop_index + source_duration_ms`。`presentation_pts_ms = loop_index × source_duration_ms + source_pts_ms` 只表达跨循环诊断/计划时间，`source_pts_ms` 只表达当前单个源文件内的位置；两者必须通过受检算术互换，不允许取模、夹紧或用缺失时长猜测。
- 普通声音使用 PortAudio 时，以 PortAudio 回调实际消费的采样数、采样率和源起点换算“实际可听 PTS”；快照可同时携带 `presentation_pts_ms` 与 `source_pts_ms`，但只有完整媒体段身份一致的快照可进入同步控制。
- 视频使用 mpv `time-pos`、播放状态和实际呈现信息；React 展示状态但不参与同步决策。
- mpv 的 `time-pos`、精确 seek 和恢复目标始终使用 `source_pts_ms`，且发送前必须再次验证 `source_pts_ms < source_duration_ms`；跨循环 `presentation_pts_ms` 不得写入单文件 mpv 会话。
- 没有 PortAudio 输出的兼容路径以 mpv 媒体时钟为权威时钟；切换权威时钟必须建立新的同步 epoch。

### 6.2 闭环规则

Rust 同步控制器以 `25–50Hz` 采样，按当前代次和同步 epoch 拒绝旧数据。正常运行禁止按采样周期反复执行精确 seek；硬 seek 只允许用于用户 seek、换源、循环边界、GPU 设备丢失或明确停滞恢复：

- 播放态循环/seek 边界固定按 `Seek → SetPause(false)` 执行，确保 `keep-open` 或 EOF 遗留的 mpv 暂停被解除；暂停态固定按 `SetPause(true) → Seek` 执行，禁止误恢复用户暂停。
- 身份不匹配、时长缺失、换算溢出或 `source_pts_ms >= source_duration_ms` 时，本轮只停用音画纠偏并把视频速度恢复到基础值；不得因此结束健康 mpv、重建播放器或触发 GPU→CPU4→Original 降级。

### 5.2.1 EOF 后端原子恢复（2026-08-30 已实施、待真实媒体验收）

- Rust 命令/runtime 是自然 EOF、单项自循环和多文件顺序推进的唯一连续性所有者。runtime actor 必须以绑定身份的 EOF 事实直接通知 Rust EOF 监督事务；React/WebView 不得为受管 mpv 发送 `complete_playback_item`，只在 EOF 待处理期间冻结视频候选并展示状态。受管原生画面活动时，`ended`、`timeupdate` 和原声音轨回绕不得抢先触发完成，也不得在本地维护下一 `loop_index`、seek 或解除暂停。
- EOF 事件必须携带并精确匹配 `playback_generation + source_path + loop_index + source_duration_ms + backend_epoch + clock_epoch`；同一身份只允许一个恢复事务，重复事件幂等返回当前结果，旧身份或跨代事件直接丢弃。
- 单文件和视频→视频事务统一执行：确认旧物理 EOF → 在 clone 上只预演下一 `loop/generation/index/source` 身份 → 对当前受管 mpv 执行合法源内 seek 或受限 `loadfile replace`、恢复权威暂停态并完成物理验收 → 重新锁定最新 `PlaybackCore`、复核起始身份后恰好调用一次 `complete_item()`。物理恢复失败时权威 generation/loop/index/position 不得推进；物理已成功但提交前身份变旧时只清理仍匹配预演身份的旧 runtime，不得用 clone 覆盖并发声音/播放状态，也不得误杀新会话。两次有界物理恢复均失败时释放失效 mpv，让既有兼容画面接管；不得留下“逻辑 Playing、物理 pause/EOF”的僵死原生表面。
- 恢复成功不能只比较 generation、epoch 或 loop。必须从 mpv 回读并确认 `pause=false`、`eof-reached=false`、`seeking=false`、`0 <= source_pts_ms < source_duration_ms`，并至少取得两个时间递增的有效 PTS 观测；多文件换源还必须确认源路径/媒体身份与 VO 已配置。用户主动暂停不得被 EOF 恢复解除。
- IPC 结果未知时先读取物理状态并按同一事务身份幂等确认；若未生效，最多执行一次同会话恢复重试。断管、进程退出、媒体身份不确定或重试后仍无 PTS 前进，才允许回收并从当前单向降级下限恢复。整个事务必须有超时、取消、Join 和结构化失败原因。
- `presentation_pts_ms` 只用于跨循环计划、音画诊断和 PortAudio 可听时钟；mpv 的 `time-pos`、seek 和恢复验收只使用 `source_pts_ms`。禁止取模、夹紧、复用 EOF 末端值或把第 N 轮 presentation PTS 直接发送给单个源文件。
- 本阶段只调整播放连续性所有权、视频物理状态验收和时间轴边界，不改变普通声音效果参数、DSP、候选文件、N/N+1、PortAudio 设备/混音、插话或固定话术语义；音频准备不得长期持有阻塞 EOF 事务的播放切换锁。
- 状态门禁：代码、合同测试和非媒体自动化完成后只能标记“已实施、待真实媒体验收”。真实门禁通过前不得写“不会卡顿”“音画同步已保证”或等价结论。

| 绝对漂移 | 行为 |
|---:|---|
| `≤20ms` | 保持基础速度 |
| `20–60ms` | 小幅调整视频速度并连续复核 |
| `60–80ms` | 加快收敛，允许丢弃已经迟到的视频帧 |
| `≥80ms` | 先暂停提交新参数并快速收敛；只有持续超限或明确停滞才执行一次精确对齐 |
| seek、换源、循环边界 | 暂停提交旧控制量，按新 epoch 对齐后恢复 |

同步控制不得在前端 2 秒轮询中实现，不得用音频元素与自身比较制造“健康”状态，也不得通过任意长 sleep 掩盖竞态。

## 7. 进程、并发和资源生命周期

- 正式运行时只有一个活动 mpv 视频进程和一个普通声音 Worker；mpv 进程与声音 Worker 可并行，但各自只能有一个所有者。
- mpv 继续使用 Windows Job Object、隐藏窗口、受管命名管道、命令允许列表、取消令牌和可等待的线程 Join。
- JSON IPC 必须是单一持久连接，由独立 reader/writer 拥有；命令携带唯一 `request_id`，通过有界队列和 pending 表对应响应。禁止每条命令重开命名管道，禁止无超时 `read_line`，禁止一条慢命令阻塞后续播放控制。
- IPC 连接、每条命令、日志读取和退出都有独立 deadline；超时、断管、重复/未知 `request_id`、无界堆积都是明确失败。
- mpv stderr 不得丢弃；使用有界尾缓冲和低频结构化摘要捕获 VO、shader、解码、设备丢失和管道错误，不记录本地媒体完整路径。
- 状态不合并成一个巨型枚举，而是三个正交维度：会话生命周期 `idle/starting/ready/playing/paused/stopping/failed`、执行后端 `gpu83_d3d11/gpu83_vulkan/cpu4/original`、参数快照 `none/pending/applied/rejected`。
- 停止、换源、seek、窗口关闭和软件退出按“封住新提交 → 取消 → 发送安全退出 → 有界等待 → 终止进程树兜底 → Join IPC/日志线程 → 关闭句柄 → 清理会话 shader 文件”释放。
- GPU 设备丢失或运行时降级时，先冻结当前画面并暂停音频推进，以当前可听 PTS 建立新 epoch 后恢复 CPU4/Original；禁止让声音继续跑而视频从零重启。
- 任何同步锁不得跨 IPC 等待；事件队列和日志缓冲必须有界；掉线、超时和 mpv 非法响应必须转为稳定错误类型。
- 前端不得提交任意 mpv 命令、属性、脚本、URL、shader 或本地路径；所有命令和属性由 Rust 固定映射。

## 8. 模块职责与预计改动

| 文件/模块 | 实施职责 |
|---|---|
| `realtime_video_backend.rs` | 收口 mpv 路径、启动参数、Job Object、命名管道、JSON IPC、命令允许列表和进程退出 |
| `realtime_video_ipc.rs` | 继续拥有持久 IPC 的有界读写与属性事件；新增 mpv PTS/VO/丢帧观察，不承担同步决策 |
| `realtime_video_runtime.rs` | 拥有唯一 mpv 会话、状态机、播放控制、源切换、后端 single-flight 单向降级、同步任务和实际状态证据 |
| `media_video_gpu_effects.rs` | 改为 GPU83 映射、mpv hook shader、动态参数快照和真实准入；不再生成 FFmpeg upload/download 链 |
| `media_gpu_capabilities.rs` | 使用生产完整 shader 做 D3D11/Vulkan 精确探测并缓存当前会话结果 |
| `audio_cycle_output.rs` | 只读发布既有 PortAudio 可听时间线与 audio epoch；不改变 DSP、候选、插话或混音 |
| `media_av_sync.rs` | 只负责 PortAudio 可听 PTS、mpv 视频 PTS、漂移控制、速度合成和同步 epoch |
| `commands.rs` | 保持薄 IPC 边界，只转发经过校验的播放、周期和状态命令 |
| `App.tsx` | 停止视频 MSE/period 调度，只负责参数计划、播放控制和真实状态展示 |
| 已删除的旧视频 MSE/period 模块 | 不再保留运行时文件、IPC、权限或契约测试；版本控制历史只用于追溯 |

不得把 mpv 生命周期、83 项 shader、音画同步和页面状态继续堆进 `App.tsx`、`commands.rs` 或单个 Rust 文件。

## 9. 实施阶段

### Phase 0：文档和契约统一

- 建立本文为唯一实施入口；同步 PRD、系统架构、桌面架构、参数契约和长任务总计划。
- 删除两份 2026-08-27 FFmpeg/MSE 视频计划，并移除当前文档中的有效引用。
- 固化 GPU83 三类执行证据、CPU4 四字段和 `80ms` 音画同步验收口径。

退出标准：当前有效文档不再把 MSE、周期重编码或 WebView2 CSS 写成正式视频主链。

### Phase 1：mpv/libplacebo 与 CPU4 技术门禁

- 使用随包 mpv 完成 D3D11、Vulkan、完整 shader、动态参数和 CPU4 命令短样本。
- 验证 1080p 的 24/25/30/50/60fps 基础帧预算；4K 延后且不阻断当前阶段。
- 记录实际 mpv、FFmpeg、libplacebo 版本和许可证，不根据 `--version` 名称列表推断可用。

退出标准：连续 200 次参数更新不重编译 shader、不重启播放器；CPU4 能动态更新四字段；任一门禁失败时只保留 Original。

当前实施结果（2026-08-28）：

- 门禁脚本已接入 `desktop/ui/package.json` 默认测试；报告在每次参数更新后按源帧间隔读取 `vo-passes.fresh`，用有序 `index/desc/count/samples` 区分相邻统计快照，至少采集 100 份相邻不重复的有效快照，并同时校验 `frame-drop-count` 与 `decoder-frame-drop-count` 增量及计数重置；不再按不唯一的 `desc` 强行匹配 pass，也不拿单帧或 IPC 延迟替代 P99 帧预算。
- 精确 mpv 提交 `7b8915bc1d` 的 `gpu-next` 源码审计确认 user hook 按路径缓存、参数原位更新且播放中保留 renderer cache；实机矩阵的 mpv PID 均保持不变，更新期间未出现重复编译候选日志。
- CPU4 固定为单一受控 `lavfi=[eq,hue]` 图，只接受亮度、对比度、饱和度、色相完整快照；Rust 和真实 mpv 动态命令均通过，尚未提前接入 Phase 4 的生产会话降级。
- 当前 1080p 证据为 [`artifacts/mpv-phase1-code-director-audit-20260828-v8-1080p.json`](../../../artifacts/mpv-phase1-code-director-audit-20260828-v8-1080p.json)，状态 `passed`。该结论不覆盖 4K、其他硬件、最终 PortAudio 音画闭环和长稳；发布法律材料仍需单独补齐。

### Phase 2：受管 mpv 会话接入

- 已完成代码接线：最终效果窗口 HWND、受管进程、持久 JSON IPC、播放控制、seek、循环、暂停/恢复、退出，以及视频→视频自然 EOF 的同进程结构化 `loadfile` 换源；真实表面仍需继续完成 1080p60、多硬件和长稳验收。
- [x] mpv stderr 有界捕获已接入生产 actor：shader/hook、VO/图形 API 和设备丢失致命日志转为真实状态与降级原因；音频错误和非故障描述不触发视频降级。
- 视频处理关闭时已进入同一 actor 的无 shader Original 会话；启动失败仍保留 WebView 兼容画面，直到 Phase 6 最终表面验收后再移除。
- 所有失败、取消和资源释放路径补齐可运行测试。

退出标准：播放池基础流程、单窗口和资源释放通过，不接 GPU83 也能稳定播放 Original。

### Phase 3：GPU83 原子参数主链

- 本阶段不新增第二套参数模型。继续以 `GPU83_PARAMETER_MAPPINGS` 为唯一字段表，每项补齐中性值、低感知值、明显对照值、执行类别、实际目标、成本等级、验真 ROI 和失败码；任何字段缺少上述事实时保持 `Unavailable`。
- 所有 GPU pass 在会话启动时加载和编译。周期只提交一份完整快照；调度字段可以与 shader 快照组成一个有序 mpv 命令批次，但禁止拆成 83 次 IPC、重载 shader、重建播放器或写中间视频。
- 随机效果统一使用 `session_id + playback_generation + sequence + field_id` 派生的确定性种子；同一快照重放必须逐帧一致，禁止使用墙钟或无界随机状态。
- 每项必须同时具备纯逻辑映射测试、生产 mpv 加载证据、单字段明显对照帧差和与其他效果组合后的真实呈现证据。只验证参数字符串或 shader 编译成功不能把字段改为 `ShaderParameter`。

#### Phase 3A：单帧像素与同帧合成

- 先把现有 20 项作为不可回退基线，再补颜色空间转换、图像修复、目标/核心空间频率、波形、动态空间均衡、通道偏移、空间维度、频率空间偏移、12 个固定视觉频段、抽象几何标记、随机图形、局部模糊、边缘填充和同帧画中画。
- 这些效果继续放在受控静态 mpv user shader 中，通过少量按职责拆分的 pass 完成；复用 `HOOKED`、`SAVE/BIND` 和预声明纹理，只访问同一呈现帧内的中间纹理，不自研解码器或通用渲染框架。
- 图像修复只实现契约可验证的轻量空间修复，不宣传 AI 修复；抽象人脸只绘制确定性几何标记，不做人脸检测；12 个频段只解释为视频空间频段，不读取或修改音频频谱。
- pass 数和采样半径必须有固定上限；零值或关闭状态必须在 shader 内保持中性旁路，不能在周期边界增删 pass。
- 候选 shader 固定放在 `desktop/tools/shader-candidates/`，不得位于 Tauri resources、不得由生产 Rust `include_str!` 或启动参数引用。候选通过 D3D11/Vulkan 编译、单字段中性/低感知/明显对照帧差和完整帧预算后，才允许把生产 shader、Rust capability、精确参数数量、固定哈希、文档和证据报告作为一次原子变更提升。
- 候选验收必须使用 `desktop/tools/verify-mpv-phase3a-candidates.mjs` 的受控单会话路径；不得最小化窗口、不得为每个字段重启播放器，且实际 OSD 表面必须精确为 `1920×1080`。v6 已取得 D3D11/Vulkan 完整 1080p 候选通过证据；生产原子提升完成前仍不得把候选结果写成生产 `61/83`。

退出标准：本组每项从 `Unavailable` 变更为可用时都有独立证据；1080p60 完整 shader 的 GPU P99 仍 `≤11–12ms`，周期更新同 PID、无重复 shader 编译、无新增丢帧。

#### Phase 3B：mpv 帧调度

- 帧率锁定和帧率扰动由 Rust 生成确定性调度状态并只通过受限速度入口执行；帧内/帧间概率、切片触发、局部模糊间隔、变换平滑、高光扰动、PIP 抖动和异步旋转则由每周期唯一静态快照提供周期起点、实际 source fps、seed、间隔和幅度，shader 使用 mpv 自动 `PTS` 在周期内推导，不允许 40ms tick 重写 `glsl-shader-opts`，也不得用固定 `30fps` 或墙钟冒充媒体调度。
- 调度以权威媒体 PTS 计算，不以 React 定时器或系统墙钟计算；暂停不推进，恢复延续当前 epoch，seek、换源和循环边界建立新调度 epoch。
- `latest-wins` 只允许合并尚未提交的 N+1 计划；已经进入当前帧的 N 快照不可被旧 generation 回写。
- 帧率扰动不得直接频繁 seek。优先使用有界呈现节奏和迟帧丢弃策略；任何会改变视频基础速度的调度必须与 Phase 5 同步控制合成成一个最终 `speed`，不得由两个控制器互相覆盖。

当前实施结果（更新至 2026-08-30）：纯调度状态机、固定 mpv PTS/FPS/pause/seeking/cache 观察命令、受限速度值类型、唯一 runtime actor、计划级随机 seed、每周期唯一完整 shader 快照和最终速度有序去重已完成；不存在通用属性入口。18 个具有真实速度或 runtime shader 消费点的调度产品字段已通过 `ScheduledParameter` 接入，视觉动态值改由 shader 的自动 PTS 在周期内推导，连续 200 个观察 tick 的合同锁定为零条 shader 属性写入。PIP 像素抖动归入同帧调度；颜色两项仍待帧级原子合同，`slice_min_length_ms` 与画中画时间轴解锁仍归历史纹理门禁。开发 capability 为 `79/83`；真实 mpv shader 编译日志、周期边界 P99 与 30 分钟实机门禁完成前，本阶段性能退出标准仍未满足。

退出标准：所有调度字段均有 PTS 序列、暂停/恢复、seek、循环和旧 epoch 测试；30 分钟内无定时器积压、无周期边界黑帧或播放器重启。

#### Phase 3C：历史帧与独立画中画时间轴

- mpv user shader 的 `SAVE/BIND` 只作为同一呈现帧内的中间纹理复用，不能据此宣称拥有跨帧历史。`picture_in_picture_timeline_locked=false` 和依赖过去源片段的切片语义必须单独通过架构门禁。
- 门禁首先使用固定随包 mpv 验证能否在不增加第二个播放器、不回到 FFmpeg 周期文件链的前提下提供有界历史纹理。若不能，完整 GPU83 必须把视频表面升级为成熟的 libmpv Render API/libplacebo 渲染边界，由受管有界纹理环保存所需历史帧；不得在现有 JSON IPC 方案旁再维护一个自研播放器。
- 引入 Render API 前必须单独核对 Windows HWND/交换链、Rust `unsafe` 边界、依赖许可证、D3D11/Vulkan 互操作、移除路径和当前 1080p 门禁。若这些门禁未通过，该组继续 `Unavailable`，整机状态不得显示 GPU83 完整。
- 历史帧容量按产品允许的最大延迟与 1080p 显存预算计算并设置硬上限；不足时拒绝本组效果或降级，不允许无界缓存、回读 CPU 或临时视频落盘。

当前实施结果（更新至 2026-08-30，非媒体准入审计）：当前外部 `mpv.exe + JSON IPC + user shader` 只能控制播放属性与同帧渲染图，不向 Rust 暴露过去帧纹理，也不能把 shader 和 renderer color 两个属性证明为同帧原子提交。仓库也没有 libmpv 运行/开发资产、bindings、Render Context 或 D3D11/Vulkan 图形资源所有者，因此暂不创建 FFI crate、不并行维护第二播放器。静态命令 `pnpm --dir desktop/ui audit:mpv-phase3c-admission` 成功时仍输出 `status/admissionStatus=not_admitted`，只有检查本身是 `checkStatus=passed`；报告固定 `phase3cImplemented=false`、`historyPromotionAllowed=false`、`historyCapability.status=not_admitted`，并把开发能力拆为 `61 shader + 18 scheduler = 79/83`。同帧 `SAVE/BIND/BUFFER` 不构成跨帧证据；两项颜色与两个 HISTORY 字段保持未接入。新增 libmpv 资产、Render API/动态加载调用或历史字段晋级都必须重新专项审核。上游当前仍在讨论 gpu-next Render API；[gpu-next Render API PR](https://github.com/mpv-player/mpv/pull/16818)仍为草案，[Vulkan Render API 请求](https://github.com/mpv-player/mpv/issues/18343)也未形成当前固定版本可用的公共接口，不能把未来提案当作生产依赖。

退出标准：独立画中画/切片时间轴具有真实历史帧对照、显存上限、取消和设备丢失测试；只有 Phase 3A–3C 与 3B 全部通过后才允许报告 `83/83`。

Phase 3 总退出标准：83 项逐字段和组合证据通过；完整快照原子提交后下一真实帧呈现；正常周期边界不切源、不重启、不黑帧且无超过一个视频帧的边界相关新增停顿。

### Phase 4：CPU4、Original 与单向降级

- 把 mpv 启动规格收口为同一个枚举：`Gpu(D3d11ZeroCopy)`、`Gpu(D3d11Copy)`、`Gpu(VulkanCopy)`、`Gpu(SoftwareDecode)`、`Cpu4`、`Original`。六种模式复用同一 source、HWND、命名管道、播放控制、stderr 和退出所有权；GPU 候选只按实际能力逐级探测，不按 AMD/NVIDIA/Intel 写分支；CPU4 只在启动时安装固定 `eq+hue` 图，周期仅发送四字段完整命令。
- 启动顺序固定为 `D3D11 零拷贝 → D3D11 copy → Vulkan/WinVK copy → GPU+软件解码 → CPU4 → Original`。同一会话没有升级边，只有新播放 generation 才能重新从第一个 GPU 候选探测。
- 运行时降级由唯一 single-flight 转换拥有者执行：封住新参数提交 → 记录降级原因和当前权威 PTS → 使旧会话停止推进并有界退出 → 以相同 source、PTS、暂停态启动下一后端 → 等待 `vo-configured` 和首帧 → 应用 CPU4 四字段或 Original → 建立新同步 epoch → 恢复播放。任何旧 generation、旧 IPC 响应或并发 stop/seek 都不能复活旧后端。
- 触发条件只来自结构化事实：GPU shader/VO 初始化失败、设备丢失、mpv 退出/断管、持续命令超时、连续帧预算超限或丢帧增长。单次慢帧、单个非法参数和一次暂态 IPC 重试不触发降级；非法参数保持上一快照并返回拒绝原因。
- 运行时帧预算按连续窗口判定，避免单帧抖动误降级；建议初始门槛为连续 3 个观测窗口 P99 超预算或丢帧持续增长，最终数值以实机门禁固化。CPU4 超限后只进入 Original。
- 不预启动隐藏 CPU4 播放器。正常周期必须无切换停顿；GPU 设备丢失属于异常恢复，目标是冻结最后一帧并有界恢复，不能承诺物理设备失效时绝对零停顿。若产品要求设备丢失也无缝，需另行批准第二渲染器预热及额外显存/CPU 预算。
- 状态必须展示真实后端、graphics API、CPU4/Original、降级起因、开始/完成时间、恢复 PTS 和是否丢弃了 79 项；前端不得根据配置推断后端。

当前实施结果（2026-08-30，代码接线与非媒体门禁）：生产 runtime 已统一为 `Gpu(D3D11ZeroCopy) → Gpu(D3D11Copy) → Gpu(VulkanCopy) → Gpu(SoftwareDecode) → Cpu4 → Original` 六级单向 floor；同一 generation 不存在自动升级边。prepare/Original 启动在释放播放转换锁前签发绑定 `playback_generation + operation_revision` 的 lease；revision、最新 generation、stop tombstone 和最新权威 cursor 由同一互斥状态原子维护。stop 将当前播放 generation 写入 tombstone；新的 seek/loop 权威 cursor 会使未发布启动失效。同一循环只接受更高 `clock_epoch`；自然循环以 `loop_index + 1` 独立表达，允许 `clock_epoch` 保持不变但禁止回退，跳过多轮、旧值和混合新旧 cursor 都拒绝，因此旧请求不能在 stop 后复活。处理关闭时主动进入受管 Original 只改变当前呈现模式，不消耗失败降级 floor；后续同代次开启处理仍可回到该 floor，而真正失败到 Original 后保持终点。prepare 的生产决策统一通过 `gpu_prepare_disposition` 区分复用、重启和丢失会话恢复；意图性 `Stopped` 会话重启当前 floor，真实进程异常退出才推进下一级，并保留退出码和有界 stderr 证据。转换先读取有界权威 PTS、停止并回收唯一旧进程，再启动下一模式；状态携带四维身份、最多五条 `demotion_history`、转换时间和恢复 PTS。CPU4 在 configured/failed/available/active 任一状态都生成固定 83 项报告，其中仅亮度、对比度、饱和度、色相 4 项支持，其余 79 项明确未执行；没有计划时也不得发布空报告。前端只接受该精确契约，Original 失败后重新读取 Rust 事实状态；确定性 stale/未接入/当前不允许错误立即停止，进程或 IPC 暂态错误才按 `1/2/4/8/15s` 有界退避。

2026-08-29 候选卡住修复：最终效果窗的自然单项循环按既有契约只递增 `loop_index`，不伪造用户 seek 的 `clock_epoch`。Rust 操作租约现与 `resolve_sync_transition` 统一接受“同代次、时钟不回退、循环恰好 +1”，并继续使旧启动 lease 失效；播放项完成 IPC 未结算时不发布临时 cursor，成功或失败结算后再发布一次权威/回滚状态，避免 Rust 已单向推进后前端回退造成后续永久 stale。新增回归测试覆盖自然循环 cursor、同步边界及前端结算门禁。修复不改变音频处理链，也不修改普通声音、插话、固定话术或音频 DSP。

2026-08-30 多文件同进程换源修复：播放池的“视频→视频”自然 EOF 现在保留唯一 runtime actor 和唯一 mpv 进程，前提是旧 EOF 四维身份精确匹配、请求 generation 恰好 +1、进程仍存活、宿主 HWND 与物理启动模式不变。切源使用 mpv 官方结构化 `loadfile` 参数顺序 `path, replace, -1, {start}`，不拼接命令字符串；`keep-open` 在 EOF 后遗留的暂停态由紧随其后的固定 `set pause` 恢复。屏障要求新路径、`eof=false`、`seeking=false`、VO 已配置和播放态首帧全部成立；取消、超时或结果不确定时必须终止旧进程，并从当前单向 fallback floor 正常重启。用户提供 TS→MP4 的开发端自然轮播中，桌面 PID 8132 下 mpv PID 16712 全程不变且没有空 PID 采样，generation 1→2，GPU 周期 N=39→52，稳定 GPU pass P99 约 `3.30ms`，VO/decoder/delayed 计数均为 0，参数快照继续真实变化。该结果只完成当前开发机与指定素材的多文件基础门禁；1080p60、多 GPU、30 分钟/200 周期和异常设备恢复仍未通过，不宣称所有硬件绝对无卡顿。普通声音功能未修改，4K 不在范围。

运行时健康门禁已纠正指标含义：`vo-passes` 只记录 GPU render pass 执行时间 P99，不能充当帧呈现间隔 P99；`frame-drop-count` 与 `decoder-frame-drop-count` 是核心计数，`mistimed-frame-count` 和 `vo-delayed-frame-count` 是 display-sync 条件指标，mpv 返回 `property unavailable` 时记为不适用且不得触发降级。只有连续 3 个完整且有效的核心健康窗口异常才触发单向降级。真实帧呈现间隔 P99、恢复位置 `≤80ms`、无黑屏、单进程和 30 分钟长稳仍必须在一次明确授权的 1080p 实机门禁中取得证据。开发 GPU capability 当前为 `79/83`；不得把 Phase 4/5 代码接线写成完整 GPU83、无卡顿或音画同步已经验收。

非媒体故障入口已进一步收口：启动期间与运行期间都会读取同一个有界 stderr 尾缓冲，shader/hook、VO/图形 API 初始化和设备丢失错误分别形成可观察原因并进入当前 floor 的下一后端；普通音频错误和非故障 `error diffusion` 描述明确不匹配。七类状态矩阵覆盖启动失败、shader 失败、设备丢失、断管、CPU4 过载、Original 恢复及并发 stop/seek，证明单向顺序、完整历史和 stop tombstone；它们不冒充真实硬件故障注入证据。

退出标准：启动失败、shader 失败、设备丢失、断管、CPU4 过载、并发 stop/seek 和 Original 恢复均有自动化状态机测试；实机确认恢复后位置误差 `≤80ms`、无双进程、无黑屏常驻和无会话内自动升级。

### Phase 5：PortAudio 音画同步

- 本阶段只接时钟，不重写音频：复用 `audio_cycle_output.rs` 已有 `callback_pcm_frames_total`、实际采样率、输出延迟、DAC 时间差和 `timeline_media_position_ms`；不修改普通声音效果、候选准备/交叉淡化、插话、固定话术或 PortAudio 混音算法。
- PortAudio 回调只更新无阻塞计数/时间快照，不执行 mpv IPC、不获取业务锁、不分配内存。Rust 输出线程把 `playback_generation + audio_epoch + source_anchor_pts + audible_callback_frame + sample_rate + DAC latency` 发布为只读快照。
- mpv 持久 IPC 观察 `time-pos`、`pause`、`seeking`、`paused-for-cache`、`eof-reached`、VO 状态和丢帧计数；所有属性都由固定命令映射且严格解析响应类型。同步所有者以 `25Hz` 起步采样，只有观测或控制量变化时发送命令。React 只读展示，不以轮询驱动同步。
- PortAudio 活动且时钟健康时，音频可听 PTS 是权威时钟，mpv 继续 `--audio=no`；PortAudio 不活动、回退 WebView 或源无音轨时，权威时钟切回 mpv 并建立新 epoch，不拿缺失音频时钟计算伪漂移。
- [x] `media_av_sync.rs` 默认速度修正已从 `±16%` 收紧：`≤20ms` 保持 `1.0`，`20–60ms` 最多约 `±1%`，`60–80ms` 最多约 `±2%` 并保留视频迟帧策略，持续 `≥80ms` 才请求一次恢复 epoch；纯逻辑覆盖已通过。最终阈值仍须以 24/25/30/50/60fps 实机数据固化，禁止周期性硬 seek。
- 帧率扰动和音画同步共享一个视频速度合成点：先计算产品调度基础速度，再叠加有界同步修正并统一限幅；任何模块不得直接覆盖另一个模块设置的 `speed`。
- 启动、用户 seek、换源、循环、GPU/CPU 降级、PortAudio 恢复和权威时钟切换允许每个新 epoch 一次精确对齐；暂停、mpv seek 和缓存等待期间不纠偏、不触发恢复硬 seek。Recovery 的边界所有者是视频同步控制器，音频事实源只发布真实音频时间线边界。N/N+1 声音候选切换只更新音频 anchor，不创建视频 generation；插话 duck 不改变主节目时间轴。
- 同步任务由播放会话拥有，使用有界取消和 Join；没有两个独立定时器、没有锁跨 mpv IPC、没有后台任务脱离会话。
- UI 只显示时钟来源、当前/最大/P99 漂移、速度修正、同步 epoch 和恢复原因；不显示“已同步”直到真实 PortAudio 与 mpv 联合门禁通过。

退出标准：纯逻辑覆盖旧 generation/epoch、乱序、时钟缺失、暂停、seek、循环、候选切换、PortAudio 恢复和后端降级；30 分钟联合实机最大漂移 `≤80ms`、P99 `≤40ms`，正常播放无持续硬 seek、无可听速度突变、无双声和无回调欠载增长。

> 2026-08-30 当前状态：只读 PortAudio 可听 PTS、统一媒体段身份、presentation/source-local PTS 分离和 mpv 源内 PTS 控制接线已完成代码与聚焦测试；播放态/暂停态边界顺序及无效时钟只停用纠偏也已锁定。mpv pause/seeking/cache-pause 事实继续 fail-closed 接入，恢复 epoch 只有视频同步控制器一个所有者。同步命令确认 `IpcTimeout` 返回 result_unknown 并保留会话，断链仍按原降级。普通声音候选 prepare 移出 Tauri 主线程的调度代码与聚焦检查已完成，且未改变 DSP 或播放语义。尚未运行真实 PortAudio/mpv 联合媒体、1080p 多文件、设备恢复和 30 分钟漂移门禁，因此 Phase 5 仍未完成，不得宣称音画同步或无卡顿指标已通过。

### Phase 5.1：三链联合长稳门禁

- 按 `GPU83 → CPU4 → Original` 分别运行 720p/1080p、24/25/30/50/60fps；4K 不进入本版本。
- 每条 GPU 路径连续 30 分钟、至少 200 次完整快照；同时记录帧呈现间隔、GPU pass P99、丢帧时间线、后端转换、音画漂移、速度控制量、PortAudio xrun/underrun 和资源残留。
- 故障注入覆盖 shader 拒绝、IPC 超时、mpv 退出、GPU 设备丢失模拟、CPU4 过载、PortAudio 时钟停滞、seek/循环与换源竞态。
- 不再为普通周期重复人工播放短样本；开发期以纯逻辑和固定离屏样本为主，只有阶段退出时执行一次完整实机矩阵并保存报告。

退出标准：Phase 3–5 的各自门禁与联合矩阵全部通过后，才允许对当前硬件范围表述“1080p 周期更新无新增卡顿、GPU83 完整接入、GPU 可自动降级且 PortAudio 音画同步通过”；仍不得扩张为所有硬件绝对保证。

### Phase 5.2：子线程组织与代码总监复核

- 子线程 A 只负责 GPU83 语义清单、shader pass 和逐字段证据；子线程 B 在 A 的字段契约冻结后只负责 mpv PTS 调度；子线程 C 只做历史纹理/Render API 技术门禁。三者不得同时修改同一 shader、映射表或运行时状态机，主线程按 A → B → C 顺序合并审查。
- GPU83 退出后再启动子线程 D，专门完成四档 GPU 候选到 `Cpu4 → Original` 的启动规格和 single-flight 降级；不得修改声音模块。
- Phase 4 退出后再启动子线程 E，只接 PortAudio 可听时钟、mpv 观测和 `media_av_sync`；不得修改声音效果、候选、插话、固定话术或混音算法。
- 子线程 F 只扩展离屏门禁、故障注入和报告汇总，不改变生产策略。普通开发循环不反复打开视频窗口；每阶段只在退出门禁执行一次实机矩阵。
- 每个子线程必须提交改动边界、调用方、验证命令、未验证项和资源释放说明。主线程在每阶段完成后以代码总监方式检查重复实现、锁跨 IPC、无界队列、旧 generation 回写、状态虚报、未使用代码和音频越界；发现问题先整改再进入下一阶段。

### Phase 6：前端正式切换与事实状态

- [x] 2026-08-29 原生表面 Z 序代码整改：现场证据显示受管 mpv 的 IPC、视频 PTS、D3D11VA、视频参数与内部截图正常，但 `WRY_WEBVIEW` child HWND 覆盖同矩形的 `GpAutoLiveNativeVideoHost`，构成当前全黑直接原因。Native host 现用最小 Win32 `SetWindowPos` 在 owner thread 提升；Original/GPU 只在当前 mpv PID 的可见、非零视频子窗口验证成功后提升，提升失败回收会话，resize 时也先复核真实子窗口再重申。未修改声音功能，未自动运行真实媒体；用户复验前不得勾选无黑屏、1080p60 或长稳门禁。

- [x] EOF/表面仲裁静态闭环：`keep-open=yes`、严格布尔 EOF、四维身份、final-effect 最小只读权限，每进程精确 HWND 绑定与换窗重建，以及 active native 时释放可见 WebView video；EOF 成立后立即停止 PTS/FPS/帧预算健康采样，暂停态不发布或消费 EOF，单项循环跨轮次等待新状态时保持原生表面。关窗使用可重绑的 suspend 而不是 stop tombstone，每个窗口实例最多清理一次。指定 HEVC 文件已确认基础原生表面，EOF、关窗重开和兼容回退仍待 1080p 实机确认。
- [x] 2026-08-29 使用用户指定的 `D:\xz\8d020eb133350a74bbc4daec1f33bbc1.mp4` 完成一次桌面基础播放复验：FFprobe 事实为 HEVC Main、`720×1280@30fps`、AAC LC 单声道 `44100Hz`、时长 `72.3s`，SHA-256 为 `0E28E35F1397BC2325FAB394F2BC61B04C0C15DB32CF9A26CAA48894F0687678`。最终效果窗口观察到连续运动画面，同时只存在一个绑定目标 HWND 的受管 mpv；命令行为 `gpu-next + D3D11 + 请求 --hwdec=d3d11va + --audio=no`，实际解码器未报告，源路径与指定文件完全一致；关闭开发应用后 mpv 与 Tauri 开发进程均无残留。该结果只证明 Original 基础原生表面和退出清理，报告见 [`artifacts/mpv-phase6-native-surface-gate-20260829-v2.json`](../../../artifacts/mpv-phase6-native-surface-gate-20260829-v2.json)。本次没有开启视频处理或 PortAudio，且未覆盖 EOF、关窗重开、GPU83、GPU→CPU4、1080p60 P99 或长稳，因此 Phase 6 联合退出标准仍未完成，也不再重复播放该文件。
- [x] 2026-08-29 首轮 `1920×1080@60fps`、12 秒、无音轨门禁保留为失败证据：原 `15ms` Windows 命名管道观察 deadline 会连续超时并结束健康 Original，画面只能认定为 WebView 兼容回退。失败报告见 [`artifacts/mpv-phase6-native-surface-gate-20260829-v1.json`](../../../artifacts/mpv-phase6-native-surface-gate-20260829-v1.json)。生产观察命令现使用 `250ms` 启动/运行预算，并把运行态瞬时观察超时记为遥测缺失；上述指定文件复验证明修复后的基础 Original 原生表面，但没有把失败的 1080p60 用例改写为通过。
- [x] 2026-08-29 视频处理开关卡顿与 Original 误失败代码整改：可能等待 runtime actor 的 Tauri 视频命令使用异步响应 wrapper，不占用窗口同步 IPC 消息泵；底层 mpv IPC 改为单所有者串行传输，超时毒化会话，启动查询共享 `5s` 总预算。Original 首帧和运行态观测只依赖有效 `time-pos`，不等待仅供 GPU83/CPU4 帧预算使用的 `estimated-vf-fps`；启动还须通过 PID→专用 HWND 的真实视频子窗口检查。普通声音、插话、固定话术和混音未修改。本项仅表示代码与非媒体门禁整改，真实文件流畅性、GPU→CPU4→Original 和 1080p60 仍待用户实测，不标记无卡顿通过。
- [x] 停止旧视频直放常量旁路和视频 MSE/period 正式调用；未确认受管原生会话时只保留显式兼容回退。
- 最终效果窗口只承载 mpv 原生视频表面；React 不再传视频媒体字节。
- [x] 运行状态事实投影完成非媒体收口：GPU83/CPU4 在首帧确认后通过受限 `hwdec-current` 读取实际解码器，`no` 映射为 `software`，属性不可用、未知值或错误响应均拒绝该后端启动；Original 对未报告属性明确显示“未报告（Original）”，不能按请求的 GPU API 伪造硬解。主界面同时展示 GPU API、GPU83/CPU4/Original、降级原因、视频周期提交漂移、GPU pass P99 以及 VO/decoder 丢帧、mistimed、delayed 和连续帧预算异常窗口。`cycle_drift_ms` 不是 PortAudio 音画同步漂移，GPU pass P99 也不是真实呈现间隔 P99；Phase 5 接线和 1080p 实机门禁前不得据此宣称音画同步或无卡顿。

退出标准：唯一正式入口为 mpv/libplacebo，旧 MSE/FFmpeg 视频链没有生产调用方。当前已完成 EOF/表面仲裁静态闭环和一项指定 HEVC 文件的 Original 基础原生表面复验；EOF/重开、GPU83/CPU4 故障恢复、1080p60 P99 与联合长稳仍未通过，因此尚未达到完整退出标准。

### Phase 7：实机、发布与旧链清理

- 完成 NVIDIA、AMD、Intel 和 CPU-only 矩阵，以及 720p/1080p、24/25/30/50/60fps；4K 留待后续版本。
- 完成安装包资源、许可证、DPI、多屏、窗口焦点、睡眠恢复、设备丢失和异常退出验收。
- [x] 历史九项发布资源与许可证静态门禁基线已接通并完成直接构建旁路整改；该项仅记录迁移前基线，当前发布契约已经扩展为十一项，最终状态以本节后续“Phase 7B 供应资产替换”和“Phase 7B Rust/Tauri 静态打包门禁”两项为准。该基线曾双向核对内嵌树、大小和 SHA-256，并确认旧供应资产的完整对应源码、SBOM 与法律复核不足，因此正式发布保持 fail-closed；该批未运行媒体、未修改任何音频功能、未实施 4K。
- [x] Phase 7A 可复现供应资产：不再把 shinchiro 二进制作为正式发布候选，只保留开发与技术门禁。固定 mpv、FFmpeg、libplacebo 完整提交、全部传递输入、四项补丁和工具链归档；断网构建只消费已校验缓存。输出包含 `mpv.exe`、`spirv-cross-c-shared.dll`、`vulkan-1.dll`、构建参数、完整依赖锁、CycloneDX/SPDX SBOM、逐组件许可证/版权、补丁集合、构建日志和对应源码归档。构建规格保持 `cplayer=true`、`libmpv=false`、D3D11/D3D11VA、Vulkan、shaderc、SPIRV-Cross、libplacebo 和必要色彩链；生产固定 `--audio=no`，WASAPI/OpenAL/SDL 音频输出及网络、Lua/JavaScript、DVD/Bluray、归档、VapourSynth、gamepad、Sixel、字幕扩展未进入依赖闭包。该裁剪未修改 PortAudio 或任何声音功能。
- [x] Phase 7A-1 可复现构建输入锁与静态门禁：新增 `desktop/third_party/mpv/reproducible-build-lock.json` 和无进程、无网络校验器。代码总监复核后已删除锁内 `status/audit/unresolved` 自证模型，状态只由校验器根据声明缺口派生；SBOM、对应源码、许可证和产物哈希由独立构建报告承载。门禁区分 Git 完整提交与不可移动的 HTTPS 归档输入，锁定缓存哈希、补丁顺序、构建镜像摘要、Windows SDK/UCRT 等工具输入、确定性环境、`--wrap-mode=nodownload`、`auto_features=disabled`、`build-date=false`、静态链接、FFmpeg 禁网、视频功能启停集合、PE 动态依赖允许列表和输出角色；缓存清单与源码/补丁/工具链必须双向相等。Meson/FFmpeg 参数固定允许集合，重复、相反开关、未知项、移动引用、构建期联网、字段漂移、自动能力或遗漏 WASAPI 均失败关闭。当前固定锁已派生为 `metadata_complete`；该状态本身仍不证明输入字节或供应候选，后两项分别由 Phase 7A-1-2 与 7A-2-2 独立证明。
- [x] Phase 7A-1-2 构建输入字节匹配门禁：源码、工具链和补丁缓存条目包含 `size_bytes`，固定 `build-inputs/` 与 `cache_inventory` 双向相等且声明总量不超过 32 GiB。Dockerfile、主脚本、cross file、配置及被调用模块统一进入 `recipe_inventory`，与固定 `build/` 实际文件双向相等。门禁有界遍历，拒绝符号链接/目录联接越界、夹带和缺失，通过同一文件句柄复核大小、SHA-256 和哈希期间身份。当前 `26` 个缓存文件均已物化并通过 `input_bytes_match_lock`；该门禁仍固定 `admitted=false`，只证明输入字节与锁一致，供应候选准入由 Phase 7A-2-2 决定。
- [x] Phase 7A-2-1 构建后报告结构、目录和哈希门禁：独立报告校验器不在锁或报告中接受自证的 admitted 字段。门禁固定 `phase7a_supply_candidate` 作用域和输入锁文件 SHA-256，要求报告中的 Meson/FFmpeg 参数、缓存清单、构建环境和动态 DLL 与锁一致；`15` 个固定角色必须路径/格式唯一，`build-evidence/` 与报告双向完全相等，每个普通文件流式复算大小和 SHA-256，夹带文件、符号链接/目录联接越界、路径越界、篡改和超限锁/报告都阻断。当前真实报告的文件层为 `fileEvidenceVerified=true`；最终 `admitted=true` 仍以 Phase 7A-2-2 的独立语义 verifier 同时通过为条件。
- [x] Phase 7A-2-2 构建后语义证据：断网构建已生成真实 `reproducible-build-report.json`；独立门禁交叉验证输入锁、dependency lock、完整 Meson introspection、PE 闭包、CycloneDX/SPDX 的 build/runtime scope 与依赖边、许可证/版权清单、四项补丁双向身份和对应源码归档内容。代码总监整改后，对应源码必须逐组件完整包含“锁定原始源码 + 按序补丁”的每个文件和字节，并单独核对 shaderc/libplacebo 的七个显式 vendoring 目标；Windows 安全内部链接按构建时解引用语义物化，非 ASCII 文件名只有在同一父目录且大小与 SHA-256 唯一匹配时才接受平台编码别名。受控归档在写盘前拒绝越界链接、特殊文件和展开超限；Docker 语义门禁同时锁定构建上下文、Dockerfile、基础镜像摘要、配方镜像标签、容器名称、entrypoint/command、起止时间和三项挂载。报告只有在语义 verifier 明确返回精确 claim 后才可准入。
- 2026-08-29 本机 Phase 7A 工具链预检：已有 Rust `1.97.1`、Git `2.47.1`、Python `3.13.9` 和 Windows SDK 目录；当前 PATH 与标准 Visual Studio 位置没有 Clang/clang-cl、lld-link、llvm-rc、CMake、Meson、Ninja、NASM 或 VS 2022，WSL 也只有未运行的 `docker-desktop`，没有 Linux 发行版。因此尚未开始编译或下载新依赖。优先方案是在本机 Docker Desktop 中使用摘要固定的交叉构建镜像，避免污染宿主机；启动 Docker、拉取构建镜像和长时间本地编译必须获得用户明确允许，且仍不得在服务器构建。
- 2026-08-29 获得 Phase 7A 授权后已恢复本机 Docker Desktop，并拉取摘要固定的官方 LLVM 镜像 `ghcr.io/llvm/ci-ubuntu-24.04@sha256:224c58f5d5f3f1d4b8f36dd3873b00a5d60c28065693165d875a9454ed914233`；镜像内确认 Clang/clang-cl/lld-link `22.1.4`、CMake `3.28.3`、Ninja `1.11.1`、Python `3.12.3` 可用，Meson、NASM、pkg-config、llvm-rc 和 llvm-lib 仍需作为固定离线输入。mpv、FFmpeg、libplacebo 三个固定提交归档已下载并复算 SHA-256，但尚未写入完整主锁。进一步代码总监审计确认 Windows SDK/UCRT 不能替代 MSVC CRT/STL，主锁校验器现把 `msvc-toolset` 提升为独立强制工具输入；缺失时必须失败关闭。完整 shaderc/Vulkan 传递依赖、MSVC sysroot、FFmpeg 静态能力集合、PE 允许列表和断网实编仍未完成，不能据此宣称 Phase 7A 供应候选成立。
- 2026-08-29 Phase 7A 最终状态：容器 `autolive-phase7a-5ee096c7f4da-65761fadfff7` 在 `--network none`、只读根文件系统、`0:0` 用户下正常退出（ExitCode `0`）；`/build-inputs` 为只读 bind、`/out` 为可写 bind、`/work` 为本轮唯一且保留的 local `volume-nocopy` volume。锁 SHA-256 为 `5ee096c7f4da718cc8b74f2006d23ef39820e0ead92266bc41da1e69dad09135`，15 项证据经 E 盘受限 staging 的 report-only、`--require-admitted` 语义门禁和事务发布后得到 `admitted=true`、`semanticEvidenceVerified=true`、精确 claim `one_locked_cold_build_candidate`。这只成立一次 locked cold-build 供应候选；不代表 bit-for-bit、Phase 7B、正式发布或 1080p 实机通过。失败及成功容器、volume、镜像和原输出均保留，没有重复媒体测试。
- 2026-08-29 Phase 7B 首轮候选矩阵保留为失败证据，未替换正式资源：CPU4 的固定 `eq+hue` 图因候选 FFmpeg 未启用 GPL 而缺少 `eq`；构建日志的 `Enabled hwaccels` 为空，D3D11 请求实际回落为软件解码；Vulkan 在 AMD 驱动的扩展 surface capabilities 查询返回 `VK_ERROR_UNKNOWN`。整改配方显式启用 GPL、D3D11VA 及 AV1/H.264/HEVC/MPEG-2/VC-1/VP9/WMV3 的 D3D11VA 两套硬解组件，并增加只对该特定 Vulkan 错误使用基础 surface 查询的 libplacebo 最小回退补丁；FFmpeg 组件许可证身份同步固定为 `GPL-2.0-or-later`，校验器拒绝参数与许可证漂移。`c51c5749...` 构建因许可证身份不一致主动中止；`68ff115a...` 构建在约 37 分钟时因宿主 Docker/WSL 引擎管道消失以 Exit `255` 中断。当前锁 `cff165702...` 的新构建已完成 mpv 链接，但在证据包生成阶段因宿主 Docker Engine 管道再次消失而以 Exit `1` 中断；三次失败均未提升候选。当前配方固定并发 `2`、容器内存与 memory-swap `4 GiB`，元数据与 26 项输入字节门禁已通过；旧 `5ee096...` 报告因锁身份不同只作历史证据，不能准入当前配方。后续必须先解决宿主 Docker 稳定性并完成同一锁的完整证据报告，再执行真实全屏 `1920×1080` D3D11/Vulkan/CPU4 技术矩阵；新候选通过前不再运行媒体门禁。
- [ ] Phase 7B 供应资产替换：新资产先通过无媒体身份/哈希/许可证门禁，再执行一次授权的 1080p D3D11/Vulkan/CPU4 技术矩阵；全部通过后才原子替换当前开发资产、生成经复核的 `THIRD-PARTY-NOTICES.md`，把三项 audit 事实改为批准，并重新生成十一项运行资源树（FFmpeg、FFprobe、三项候选二进制、内部清单和五项法律文件）。任一功能、性能、SBOM、对应源码或许可证复核失败都保持当前正式发布阻断。
- [x] Phase 7B Rust/Tauri 静态打包门禁迁移与代码总监整改完成：外层 `runtime-resources.json` 保持 schema 1，Windows 内嵌树十一项双向核对及内部 `mpv-runtime-manifest.json` schema 2 的 `build/components/audit/files/supply_evidence` 校验已实现；Cargo release 通过 `build.rs` 的 `include_str!` 固定仓库源码侧候选清单字节锚点，包内清单必须先与该参考逐字节一致，禁止同时替换包内清单、十一项资源和外层哈希后自洽通过，`components` 字段集合也严格限定为 mpv/libplacebo/FFmpeg。非 Windows 目标不要求该清单参考；同时篡改和额外组件负向测试均已通过。该项不代表候选资产、法律复核或正式发布已经通过，当前未替换真实资源。
- [x] 按用户授权删除旧 MSE/周期转码代码、IPC、权限、测试和文档历史实现；普通声音链保持不变。

退出标准：发布门禁通过；旧链清理必须单独获得确认，不在本方案实施中隐式删除。

## 10. 测试与验收

### 10.1 自动测试

- 83 个字段精确、唯一且不含水平/垂直翻转；12 个 band weight 有独立证据。
- 一份 GPU83 计划只生成一次参数属性更新；非法、缺失、重复、NaN/Infinity 快照被拒绝。
- shader 元数据、参数声明、名称、类型和默认值可由生产 mpv 精确加载；编译/执行禁用日志视为失败。
- 不存在固定 `fr/30.0` 或用字段数量推断效果成功的生产判断。
- CPU4 严格拒绝其余 79 项；四参数范围与 libavfilter 命令一致。
- GPU 后端失败后只降级一次；新会话才允许重新探测升级。
- 同步控制覆盖 `20/60/80ms`、旧 epoch、暂停、seek、循环和换源边界。
- mpv IPC 覆盖超时、断管、乱序、过期 generation、非法响应、取消、退出和线程 Join。
- 前端状态只消费 Rust 事实源，不根据编码器名称、请求的 `--hwdec`、GPU API、配置值或计划值推断已生效；GPU83/CPU4 的硬解/软解必须来自首帧后的 `hwdec-current`，Original 未收到该属性时只能显示“未报告”，不得推断。

### 10.2 实机门禁

| 项目 | 门槛 |
|---|---|
| 正常周期边界 | 不切源、不重启、不黑帧，不产生超过一个视频帧的边界相关新增停顿 |
| 60fps GPU83 | 每帧总预算 `16.67ms`，GPU83 P99 目标 `≤11–12ms` |
| 长稳 | 连续 30 分钟且至少 200 次周期更新 |
| 音画同步 | 最大漂移 `≤80ms`，P99 `≤40ms` |
| GPU83 | 83 项逐字段明显对照、低感知组合、原子提交和实际呈现证据齐全 |
| CPU4 | 四参数真实变化，其他字段未应用；过载时连续回到 Original |
| 资源 | 停止、换源、关窗、退出和异常恢复后无遗留 mpv、FFmpeg、线程、管道或会话 shader 文件 |
| 硬件 | NVIDIA、AMD、Intel、CPU-only；720p、1080p；24/25/30/50/60fps；4K 延后 |

“完全不卡顿”不能只写主观结论；必须保存周期边界前后帧呈现间隔、丢帧、GPU 时间、同步漂移和后端状态证据。驱动重置或设备丢失允许一次有界恢复，但不得把异常恢复算作正常周期无卡顿验收。

## 11. 回滚与兼容

- 当前稳定 Original 直放仅作为 mpv 原生最终表面完成前的开发回滚；不得与 mpv 同时争用同一最终表面。
- 旧 MSE/逐周期 FFmpeg 视频链已从生产代码删除，审计只依赖版本控制；普通声音的 FFmpeg 文件链继续有效。
- mpv 资源缺失、校验失败或所有后端探测失败时，明确报告视频后端不可用；不得静默恢复旧周期重编码主链。
- 新导入不再生成兼容 `playback_reference`；已有字段和历史枚举值仅保持反序列化兼容。旧 `media-compatibility` 缓存目录只由受限清理入口回收，不再参与新播放池编辑生命周期。

## 12. 依赖与许可证

- 固定并随包分发 mpv、FFmpeg 和 libplacebo 版本、来源、哈希、构建选项与许可证文本。
- `mpv --version`、`--vo=help`、`--hwdec=help`、`--vf=help` 只用于发现候选能力；正式准入仍使用真实短样本。
- mpv 的 `gpu-next`、自定义 GLSL 和运行时属性以官方文档为准：<https://github.com/mpv-player/mpv>。
- libplacebo 动态参数与 shader 能力以官方仓库为准：<https://github.com/haasn/libplacebo>。
- CPU4 使用 FFmpeg 官方 libavfilter，不引入自研像素处理：<https://ffmpeg.org/ffmpeg-filters.html>。
- Windows mpv 进程树使用精确锁定的 `win32job 2.0.3` 安全封装，只在 `ManagedMpvProcess` 创建、绑定和 Drop/取消路径调用；许可证为 MIT/Apache-2.0，底层复用锁文件已有的 `windows 0.61.3` 与 `thiserror 1.0.69`。替代方案是项目内直接调用 Win32 Job API，但会破坏 `unsafe_code = "forbid"`；移除路径是在未来由成熟播放器宿主完全接管同等 Job 所有权后删除这一 Windows-only 依赖和私有封装。
- 第三方 Tauri mpv 插件和完整播放器仓库只作接线、打包与生命周期参考，不进入当前依赖树。

## 13. 交付清单

- [x] 当前有效文档全部引用本文为唯一视频实施入口。
- [x] 当前开发机对拟随包 mpv/libplacebo/CPU4 二进制执行的 Phase 1 1080p 短样本技术矩阵已通过：报告 [`artifacts/mpv-phase1-code-director-audit-20260828-v8-1080p.json`](../../../artifacts/mpv-phase1-code-director-audit-20260828-v8-1080p.json) 为 `passed`，覆盖 D3D11/Vulkan 的 24/25/30/50/60fps 与 CPU4 60fps。该勾选不表示安装包整体验收通过，也不覆盖 Phase 6 最终原生表面、生产自动降级、跨硬件、30 分钟长稳或发布法律材料；对应项目继续保持未完成。
- [x] Phase 1 门禁工具、CPU4 四字段命令、版本/哈希清单和 fail-closed 发布检查已实现。
- [x] 常驻 mpv Original 的命令、runtime actor、进程状态、seek/loop/pause 同步和 WebView 失败兼容已完成代码与非媒体自动化接线。
- [x] 常驻 mpv Original 基础播放与退出清理通过：用户指定 HEVC `720×1280@30fps` 文件在单一受管 mpv 原生表面中连续运动，关闭后无进程残留；不代表 EOF/重开、GPU83、CPU4 或 1080p60 性能通过。
- [x] 用户指定 TS→MP4 的视频播放池自然换源已通过单进程开发端门禁：受管 mpv PID 16712 在 generation 1→2 全程不变、无空 PID 窗口，GPU 周期和 PTS 在新源继续推进，稳定观察 GPU pass P99 约 `3.30ms` 且 VO/decoder/delayed 计数为 0；该项不替代 1080p60、多硬件或长稳门禁。
- [ ] 修复后的 Windows IPC 观察预算通过一次明确授权的 1080p60 原生表面/EOF/关窗重开实机复验；2026-08-29 首轮 1080p60 报告仍为失败，指定竖屏文件的基础通过不得替代该门禁。
- [ ] GPU83 逐字段映射、原子更新和呈现证据通过。
- [ ] CPU4 四字段和 Original 单向降级通过。
- [x] Phase 4 四级单向降级、启动 lease/stop tombstone、seek/loop 取消、CPU4 `83/4/79` 事实状态和非媒体状态机测试已完成；真实 1080p 故障注入、恢复误差、黑屏/单进程及长稳门禁仍归入上一项，不能据此勾选实机通过。
- [x] 统一媒体段身份、presentation/source-local PTS 分离、播放/暂停边界命令顺序，以及无效音频时钟只停用纠偏的代码与聚焦测试已完成；真实 1080p 多文件、设备恢复和长稳未包含在该勾选内。
- [x] 前端视频同步单在途/latest-wins、有界 busy 退避、result_unknown 状态确认、stale 刷新，及 Rust 稳定错误分类/健康 GPU/CPU4 Original 门禁已完成代码与聚焦测试；同步确认超时保留会话，明确断链仍按原降级。
- [x] 普通声音候选 prepare 已移出 Tauri 主线程，并通过命令聚焦测试、`cargo check --all-targets` 与严格 clippy；只改变调度位置，不改变音频 DSP、候选格式、提交或播放语义，真实联合媒体流畅性仍待用户验收。
- [ ] PortAudio 音画同步长稳通过。
- [x] 前端正式周期优先进入 mpv；处理关闭调用受管 Original，启动失败才保留 WebView 兼容画面；处理开启失败有界重试，旧视频 MSE/period 没有生产调用方。
- [ ] Windows 多硬件、1080p 多帧率和异常恢复验收通过；4K 不属于当前阶段。
- [ ] 发布资源、哈希和许可证检查通过。
- [x] 已经用户确认删除旧 MSE/周期转码代码、权限、测试和当前文档中的历史实施段落；版本控制历史不作为当前上下文恢复。
