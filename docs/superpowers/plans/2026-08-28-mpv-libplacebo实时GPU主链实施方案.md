# mpv/libplacebo 实时 GPU 主链实施方案

> 日期：2026-08-28
>
> 状态：实施中（里程碑 1 已完成，当前 1080p Phase 1 实机门禁已通过；下一版本 Phase 3–5 的实施顺序、模块边界和退出门禁已冻结）。本文是当前视频播放、GPU83、CPU4、Original 降级和音画同步的唯一实施入口；4K 延后，不属于当前开发与验收范围；完整 GPU83、生产 CPU4 降级、真实 PortAudio 闭环、Windows 多硬件和长稳验收尚未完成，不得标记为已交付。
>
> 覆盖范围：本文覆盖此前“逐 period FFmpeg 重编码 → `.partial/.ready` → Rust 分块 → WebView2 MSE”、视频 A/B、WebView2 CSS 视频效果以及“mpv/libplacebo 仅作预留”的视频主链描述。两份旧视频实施方案已按用户要求删除，后续不得从提交历史恢复为当前上下文。普通声音 N/N+1、PortAudio、插话、固定话术、播放池和导入原子性继续有效。

## 0. 当前实施进度（2026-08-28）

- 已接入单一受管 mpv 进程的持久 Windows 命名管道 JSON IPC：请求 ID、请求 deadline、有界 pending/响应、响应大小上限、stderr 尾缓冲、媒体路径脱敏、退出与强制回收均已落地；Windows Job Object 仍待补齐，当前强制回收使用进程树兜底。
- 已将 `prepare_realtime_video_plan` 从 WebView2/CSS 编译目标改为 GPU83 快照编译，并在 React 正式视频周期中优先调用；成功候选不再进入旧 MSE/逐周期转码，失败则停止实时 renderer、释放候选并保持 Original。
- 已建立精确且无重复的 83 项映射表；当前只有 `20/83` 项具备已验证 shader 参数语义，剩余 `63/83` 明确返回 `Unavailable`，因此含未验证激活项的快照拒绝宣称 GPU83 完整生效。
- 视频运行状态已收口为 `realtime_gpu / cpu4 / source`；旧 `ffmpeg_gpu / ffmpeg_cpu`、WebView2 CSS 传输分支、编码器字段及其仅测试可达代码已删除，前端对旧状态 fail-closed。
- 已随包登记 `resources/shaders/gpu83.hook`，周期参数通过一条受限 `set_property glsl-shader-opts` 命令原子更新；本机随包 mpv `gpu-next + D3D11 + D3D11VA` 已真实加载 hook，shaderc 结果为 `0 errors, 0 warnings`。
- 已实现纯 Rust 音画同步控制器及代次/epoch 隔离、`20/60/80ms` 分级、有限速度修正、迟帧策略和恢复请求；尚未连接 PortAudio 实际可听 PTS 与 mpv 已呈现 PTS，普通周期硬 seek 已从同步命令中移除。
- 旧 MSE/SourceBuffer、视频 period/分块 IPC、逐周期 FFmpeg 视频 Worker、权限和对应测试已按用户授权删除；普通声音候选、PortAudio、插话、固定话术和必要的一次性视频导入兼容保持不变。CPU4、D3D11→Vulkan→CPU4→Original 完整会话降级、mpv EOF 驱动播放池、最终表面整机冒烟和长稳测试仍未完成。
- Phase 1 已新增可重复执行的 Windows 实机门禁 `desktop/tools/verify-mpv-phase1.mjs`、固定 CPU4 `eq+hue` 四字段命令编译和 mpv 发布资源/哈希/许可证材料门禁。最终代码总监报告为 [`artifacts/mpv-phase1-code-director-audit-20260828-v8-1080p.json`](../../../artifacts/mpv-phase1-code-director-audit-20260828-v8-1080p.json)：D3D11/Vulkan 的 1080p 24/25/30/50/60fps 与 CPU4 1080p60 全部通过；GPU 每路径 200 次参数更新、CPU4 160 次四字段快照均保持同 PID，逐帧丢帧时间线增量为零，帧预算和 shader 缓存契约通过。当前最高验收分辨率固定为 `1920×1080`，4K 已按用户要求延后。第三方法律材料、跨硬件运行时性能降级和长稳仍未完成，因此只证明当前机器和当前矩阵，不作所有硬件“绝对不卡顿”的承诺。
- 删除后本地验证：Rust `cargo fmt --check`、`cargo check --all-targets`、桌面命令测试 `73/73`、mpv 入口契约 `3/3` 和前端视频运行时测试 `28/28` 通过。Rust 库测试为 `312/313`，剩余失败是既有 StandardMp4/Vulkan GPU83 测试夹具；前端全套为 `392/393`，剩余失败只涉及 PortAudio 设备列表无效响应的既有断言。两项均未通过改动音频链或扩大本轮旧视频删除范围来掩盖。
- 下一版本只按 `Phase 3A → 3B → 3C → Phase 4 → Phase 5 → 联合长稳` 串行推进：先冻结并逐项证明 GPU83 语义，再接生产降级，最后把已有 PortAudio 可听时间线与已有音画控制器接入 mpv。音频改动只允许增加只读时钟快照、同步 epoch 和控制接线，不修改普通声音 DSP、N/N+1、插话、固定话术或混音语义。
- Phase 3A 首批候选已开始实施但尚未生产提升：`desktop/tools/shader-candidates/gpu83-baseline-candidate.hook` 修复现有 20 项基线中的 YIQ hue 矩阵列主序、饱和度旧 luma 和无几何变化时裁剪回混问题；`gpu83-image-repair-candidate.hook` 以独立最终帧 fragment pass 实现固定 1px 四邻域边缘感知修复。两份候选不进入 Tauri bundle，生产 shader、固定哈希和 `20/83` capability 保持不变。色彩空间转换因缺少源/目标 primaries、TRC、范围和输出标签契约继续 `Unavailable`，不得用硬编码色偏矩阵冒充。静态 Node 门禁 `25/25` 与 GPU83 Rust 目标测试 `6/6` 通过，未运行视频窗口；D3D11/Vulkan 编译、逐字段帧差和 P99 仍是生产提升前门禁。

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
- 不恢复已删除的旧 MSE/FFmpeg 视频周期链；必要视频导入兼容和普通声音 FFmpeg 能力继续保留。

## 3. 当前事实与必须先修复的问题

1. 最终效果窗口迁移期仍由 `DIRECT_VIDEO_SRC_PLAYBACK = true` 提供 Original 时钟和兜底；mpv 已进入正式周期准备入口，但最终原生表面、EOF 和播放控制尚需整机接线验收。
2. 实时视频命令已改为 GPU83 快照编译目标，并通过受管 mpv 的单一持久 IPC 更新；旧 WebView2 CSS 编译目标不再由该命令调用。
3. 精确 83 项映射已建立，但目前只有 20 项拥有已验证 shader 语义；其余 63 项必须继续补算法与逐字段证据，不能用字段存在冒充生效。
4. 受控 mpv hook 已修复结构和标识符问题，并在本机 D3D11 路径真实编译通过；Vulkan、不同厂商 GPU 和组合效果仍待验证。
5. shader 使用 mpv 提供的 PTS，不再按固定 `30fps` 计算；真实帧率调度类字段仍未接入。
6. 旧 GPU 文件链/MSE 生产代码、IPC、权限和测试已删除；版本控制历史不得重新成为当前实现依赖。
7. 音画同步控制律已实现但尚未接入 PortAudio/mpv 真实观测，因而当前不能声称满足 `80ms` 门槛。

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
- `source_path` 是 GPU83/CPU4 的首选输入；源格式或解码失败时可使用导入阶段已有的 `playback_reference` 作为兼容播放引用。
- `playback_reference` 不再要求满足任意 MSE period 边界；后续可在独立清理任务中评估是否收窄导入转码契约，本批不借机修改播放池导入原子性。

## 6. 音画同步

### 6.1 权威时钟

- 普通声音使用 PortAudio 时，以 PortAudio 回调实际消费的采样数、采样率和源起点换算“实际可听 PTS”。
- 视频使用 mpv `time-pos`、播放状态和实际呈现信息；React 展示状态但不参与同步决策。
- 没有 PortAudio 输出的兼容路径以 mpv 媒体时钟为权威时钟；切换权威时钟必须建立新的同步 epoch。

### 6.2 闭环规则

Rust 同步控制器以 `25–50Hz` 采样，按当前代次和同步 epoch 拒绝旧数据。正常运行禁止按采样周期反复执行精确 seek；硬 seek 只允许用于用户 seek、换源、循环边界、GPU 设备丢失或明确停滞恢复：

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

- 门禁脚本与 16 项纯逻辑测试已接入 `desktop/ui/package.json` 默认测试；报告按 `vo-passes.fresh[].desc` 匹配更新前后 pass，至少采集 100 帧，并同时校验 `frame-drop-count` 与 `decoder-frame-drop-count` 增量，不再拿单帧或 IPC 延迟替代逐帧预算。
- 精确 mpv 提交 `7b8915bc1d` 的 `gpu-next` 源码审计确认 user hook 按路径缓存、参数原位更新且播放中保留 renderer cache；实机矩阵的 mpv PID 均保持不变，更新期间未出现重复编译候选日志。
- CPU4 固定为单一受控 `lavfi=[eq,hue]` 图，只接受亮度、对比度、饱和度、色相完整快照；Rust 和真实 mpv 动态命令均通过，尚未提前接入 Phase 4 的生产会话降级。
- 当前 1080p 证据为 [`artifacts/mpv-phase1-code-director-audit-20260828-v8-1080p.json`](../../../artifacts/mpv-phase1-code-director-audit-20260828-v8-1080p.json)，状态 `passed`。该结论不覆盖 4K、其他硬件、最终 PortAudio 音画闭环和长稳；发布法律材料仍需单独补齐。

### Phase 2：受管 mpv 会话接入

- 接通最终效果窗口 HWND、受管进程、持久 JSON IPC、播放控制、seek、循环、换源、暂停/恢复和退出。
- 接入 mpv stderr 有界捕获，将 shader/VO/设备错误转为真实状态和降级原因。
- 当前仍以 Original 播放，先证明一个常驻播放器完成基础流程。
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

退出标准：本组每项从 `Unavailable` 变更为可用时都有独立证据；1080p60 完整 shader 的 GPU P99 仍 `≤11–12ms`，周期更新同 PID、无重复 shader 编译、无新增丢帧。

#### Phase 3B：mpv 帧调度

- 帧率锁定、帧率扰动、帧内/帧间概率、切片触发、局部模糊间隔、变换平滑、高光扰动和异步旋转由 Rust 生成确定性调度状态，再映射为受限 mpv 属性与同一 shader 快照中的门控值；不得用 GLSL 固定 `30fps` 时间或片段内随机数冒充调度。
- 调度以权威媒体 PTS 计算，不以 React 定时器或系统墙钟计算；暂停不推进，恢复延续当前 epoch，seek、换源和循环边界建立新调度 epoch。
- `latest-wins` 只允许合并尚未提交的 N+1 计划；已经进入当前帧的 N 快照不可被旧 generation 回写。
- 帧率扰动不得直接频繁 seek。优先使用有界呈现节奏和迟帧丢弃策略；任何会改变视频基础速度的调度必须与 Phase 5 同步控制合成成一个最终 `speed`，不得由两个控制器互相覆盖。

退出标准：所有调度字段均有 PTS 序列、暂停/恢复、seek、循环和旧 epoch 测试；30 分钟内无定时器积压、无周期边界黑帧或播放器重启。

#### Phase 3C：历史帧与独立画中画时间轴

- mpv user shader 的 `SAVE/BIND` 只作为同一呈现帧内的中间纹理复用，不能据此宣称拥有跨帧历史。`picture_in_picture_timeline_locked=false` 和依赖过去源片段的切片语义必须单独通过架构门禁。
- 门禁首先使用固定随包 mpv 验证能否在不增加第二个播放器、不回到 FFmpeg 周期文件链的前提下提供有界历史纹理。若不能，完整 GPU83 必须把视频表面升级为成熟的 libmpv Render API/libplacebo 渲染边界，由受管有界纹理环保存所需历史帧；不得在现有 JSON IPC 方案旁再维护一个自研播放器。
- 引入 Render API 前必须单独核对 Windows HWND/交换链、Rust `unsafe` 边界、依赖许可证、D3D11/Vulkan 互操作、移除路径和当前 1080p 门禁。若这些门禁未通过，该组继续 `Unavailable`，整机状态不得显示 GPU83 完整。
- 历史帧容量按产品允许的最大延迟与 1080p 显存预算计算并设置硬上限；不足时拒绝本组效果或降级，不允许无界缓存、回读 CPU 或临时视频落盘。

退出标准：独立画中画/切片时间轴具有真实历史帧对照、显存上限、取消和设备丢失测试；只有 Phase 3A–3C 与 3B 全部通过后才允许报告 `83/83`。

Phase 3 总退出标准：83 项逐字段和组合证据通过；完整快照原子提交后下一真实帧呈现；正常周期边界不切源、不重启、不黑帧且无超过一个视频帧的边界相关新增停顿。

### Phase 4：CPU4、Original 与单向降级

- 把 mpv 启动规格收口为同一个枚举：`Gpu(D3d11)`、`Gpu(Vulkan)`、`Cpu4`、`Original`。四种模式复用同一 source、HWND、命名管道、播放控制、stderr 和退出所有权；CPU4 只在启动时安装固定 `eq+hue` 图，周期仅发送四字段完整命令。
- 启动顺序固定为 `D3D11 → Vulkan → CPU4 → Original`。同一会话没有升级边，只有新播放 generation 才能重新从 D3D11 探测。
- 运行时降级由唯一 single-flight 转换拥有者执行：封住新参数提交 → 记录降级原因和当前权威 PTS → 使旧会话停止推进并有界退出 → 以相同 source、PTS、暂停态启动下一后端 → 等待 `vo-configured` 和首帧 → 应用 CPU4 四字段或 Original → 建立新同步 epoch → 恢复播放。任何旧 generation、旧 IPC 响应或并发 stop/seek 都不能复活旧后端。
- 触发条件只来自结构化事实：GPU shader/VO 初始化失败、设备丢失、mpv 退出/断管、持续命令超时、连续帧预算超限或丢帧增长。单次慢帧、单个非法参数和一次暂态 IPC 重试不触发降级；非法参数保持上一快照并返回拒绝原因。
- 运行时帧预算按连续窗口判定，避免单帧抖动误降级；建议初始门槛为连续 3 个观测窗口 P99 超预算或丢帧持续增长，最终数值以实机门禁固化。CPU4 超限后只进入 Original。
- 不预启动隐藏 CPU4 播放器。正常周期必须无切换停顿；GPU 设备丢失属于异常恢复，目标是冻结最后一帧并有界恢复，不能承诺物理设备失效时绝对零停顿。若产品要求设备丢失也无缝，需另行批准第二渲染器预热及额外显存/CPU 预算。
- 状态必须展示真实后端、graphics API、CPU4/Original、降级起因、开始/完成时间、恢复 PTS 和是否丢弃了 79 项；前端不得根据配置推断后端。

退出标准：启动失败、shader 失败、设备丢失、断管、CPU4 过载、并发 stop/seek 和 Original 恢复均有自动化状态机测试；实机确认恢复后位置误差 `≤80ms`、无双进程、无黑屏常驻和无会话内自动升级。

### Phase 5：PortAudio 音画同步

- 本阶段只接时钟，不重写音频：复用 `audio_cycle_output.rs` 已有 `callback_pcm_frames_total`、实际采样率、输出延迟、DAC 时间差和 `timeline_media_position_ms`；不修改普通声音效果、候选准备/交叉淡化、插话、固定话术或 PortAudio 混音算法。
- PortAudio 回调只更新无阻塞计数/时间快照，不执行 mpv IPC、不获取业务锁、不分配内存。Rust 输出线程把 `playback_generation + audio_epoch + source_anchor_pts + audible_callback_frame + sample_rate + DAC latency` 发布为只读快照。
- mpv 持久 IPC 观察 `time-pos`、`pause`、`seeking`、`eof-reached`、VO 状态和丢帧计数；同步所有者以 `25Hz` 起步采样，只有观测或控制量变化时发送命令。React 只读展示，不以轮询驱动同步。
- PortAudio 活动且时钟健康时，音频可听 PTS 是权威时钟，mpv 继续 `--audio=no`；PortAudio 不活动、回退 WebView 或源无音轨时，权威时钟切回 mpv 并建立新 epoch，不拿缺失音频时钟计算伪漂移。
- 复用 `media_av_sync.rs`，但在接线前把默认速度修正从当前过大的 `±16%` 收紧并通过听感/帧预算门禁：`≤20ms` 保持 `1.0`，`20–60ms` 使用不超过约 `±1%` 的比例修正，`60–80ms` 不超过约 `±2%` 并允许视频迟帧丢弃，持续 `≥80ms` 才请求一次恢复 epoch。最终阈值以 24/25/30/50/60fps 实机数据固化，禁止周期性硬 seek。
- 帧率扰动和音画同步共享一个视频速度合成点：先计算产品调度基础速度，再叠加有界同步修正并统一限幅；任何模块不得直接覆盖另一个模块设置的 `speed`。
- 启动、用户 seek、换源、循环、GPU/CPU 降级、PortAudio 恢复和权威时钟切换允许每个新 epoch 一次精确对齐；暂停期间两个时钟都不推进。N/N+1 声音候选切换只更新音频 anchor，不创建视频 generation；插话 duck 不改变主节目时间轴。
- 同步任务由播放会话拥有，使用有界取消和 Join；没有两个独立定时器、没有锁跨 mpv IPC、没有后台任务脱离会话。
- UI 只显示时钟来源、当前/最大/P99 漂移、速度修正、同步 epoch 和恢复原因；不显示“已同步”直到真实 PortAudio 与 mpv 联合门禁通过。

退出标准：纯逻辑覆盖旧 generation/epoch、乱序、时钟缺失、暂停、seek、循环、候选切换、PortAudio 恢复和后端降级；30 分钟联合实机最大漂移 `≤80ms`、P99 `≤40ms`，正常播放无持续硬 seek、无可听速度突变、无双声和无回调欠载增长。

### Phase 5.1：三链联合长稳门禁

- 按 `GPU83 → CPU4 → Original` 分别运行 720p/1080p、24/25/30/50/60fps；4K 不进入本版本。
- 每条 GPU 路径连续 30 分钟、至少 200 次完整快照；同时记录帧呈现间隔、GPU pass P99、丢帧时间线、后端转换、音画漂移、速度控制量、PortAudio xrun/underrun 和资源残留。
- 故障注入覆盖 shader 拒绝、IPC 超时、mpv 退出、GPU 设备丢失模拟、CPU4 过载、PortAudio 时钟停滞、seek/循环与换源竞态。
- 不再为普通周期重复人工播放短样本；开发期以纯逻辑和固定离屏样本为主，只有阶段退出时执行一次完整实机矩阵并保存报告。

退出标准：Phase 3–5 的各自门禁与联合矩阵全部通过后，才允许对当前硬件范围表述“1080p 周期更新无新增卡顿、GPU83 完整接入、GPU 可自动降级且 PortAudio 音画同步通过”；仍不得扩张为所有硬件绝对保证。

### Phase 5.2：子线程组织与代码总监复核

- 子线程 A 只负责 GPU83 语义清单、shader pass 和逐字段证据；子线程 B 在 A 的字段契约冻结后只负责 mpv PTS 调度；子线程 C 只做历史纹理/Render API 技术门禁。三者不得同时修改同一 shader、映射表或运行时状态机，主线程按 A → B → C 顺序合并审查。
- GPU83 退出后再启动子线程 D，专门完成 `Gpu(D3D11/Vulkan) → Cpu4 → Original` 启动规格和 single-flight 降级；不得修改声音模块。
- Phase 4 退出后再启动子线程 E，只接 PortAudio 可听时钟、mpv 观测和 `media_av_sync`；不得修改声音效果、候选、插话、固定话术或混音算法。
- 子线程 F 只扩展离屏门禁、故障注入和报告汇总，不改变生产策略。普通开发循环不反复打开视频窗口；每阶段只在退出门禁执行一次实机矩阵。
- 每个子线程必须提交改动边界、调用方、验证命令、未验证项和资源释放说明。主线程在每阶段完成后以代码总监方式检查重复实现、锁跨 IPC、无界队列、旧 generation 回写、状态虚报、未使用代码和音频越界；发现问题先整改再进入下一阶段。

### Phase 6：前端正式切换与事实状态

- 停止 `DIRECT_VIDEO_SRC_PLAYBACK` 临时旁路和视频 MSE/period 正式调用。
- 最终效果窗口只承载 mpv 原生视频表面；React 不再传视频媒体字节。
- 状态明确展示 GPU API、硬解/软解、GPU83/CPU4/Original、降级原因和最近同步漂移。

退出标准：唯一正式入口为 mpv/libplacebo，旧 MSE/FFmpeg 视频链没有生产调用方。

### Phase 7：实机、发布与旧链清理

- 完成 NVIDIA、AMD、Intel 和 CPU-only 矩阵，以及 720p/1080p、24/25/30/50/60fps；4K 留待后续版本。
- 完成安装包资源、许可证、DPI、多屏、窗口焦点、睡眠恢复、设备丢失和异常退出验收。
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
- 前端状态只消费 Rust 事实源，不根据编码器名称、配置值或计划值推断已生效。

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
- 导入阶段已有 `playback_reference` 继续作为兼容播放引用，直到后续独立任务证明可安全收窄；本批不破坏已有播放池缓存生命周期。

## 12. 依赖与许可证

- 固定并随包分发 mpv、FFmpeg 和 libplacebo 版本、来源、哈希、构建选项与许可证文本。
- `mpv --version`、`--vo=help`、`--hwdec=help`、`--vf=help` 只用于发现候选能力；正式准入仍使用真实短样本。
- mpv 的 `gpu-next`、自定义 GLSL 和运行时属性以官方文档为准：<https://github.com/mpv-player/mpv>。
- libplacebo 动态参数与 shader 能力以官方仓库为准：<https://github.com/haasn/libplacebo>。
- CPU4 使用 FFmpeg 官方 libavfilter，不引入自研像素处理：<https://ffmpeg.org/ffmpeg-filters.html>。
- 第三方 Tauri mpv 插件和完整播放器仓库只作接线、打包与生命周期参考，不进入当前依赖树。

## 13. 交付清单

- [x] 当前有效文档全部引用本文为唯一视频实施入口。
- [ ] 随包 mpv/libplacebo/CPU4 技术门禁通过。
- [x] Phase 1 门禁工具、CPU4 四字段命令、版本/哈希清单和 fail-closed 发布检查已实现。
- [ ] 常驻 mpv Original 播放基础流程通过。
- [ ] GPU83 逐字段映射、原子更新和呈现证据通过。
- [ ] CPU4 四字段和 Original 单向降级通过。
- [ ] PortAudio 音画同步长稳通过。
- [x] 前端正式周期优先进入 mpv；新主链启用时处理关闭直接保持 Original、处理开启失败有界重试，旧视频 MSE/period 没有生产调用方。
- [ ] Windows 多硬件、1080p 多帧率和异常恢复验收通过；4K 不属于当前阶段。
- [ ] 发布资源、哈希和许可证检查通过。
- [x] 已经用户确认删除旧 MSE/周期转码代码、权限、测试和当前文档中的历史实施段落；版本控制历史不作为当前上下文恢复。
