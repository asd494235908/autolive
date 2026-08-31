# ZLMediaKit RTMP GPU 直推实施方案

> 状态（2026-08-30）：方案已确认，尚未实施代码、IPC、权限、UI 或真实推流验收。本文件是该功能后续开发、审查和验收的唯一实施入口；没有真实运行证据前必须标记为“正式需求·待实施/未接入”，不得描述为已支持推流。

> 范围澄清：ZLMediaKit 部署在用户指定的流媒体服务器上，桌面客户端直接向用户输入的 `rtmp://` 或 `rtmps://` 地址发布媒体。Go 后端和 React 管理系统不转发、不代理、不保存媒体正文；本方案不接入 OBS、直播平台账号、多平台分发或平台审核/检测规避能力。

## 1. 决策

采用“源媒体直接处理 + 最终声音总线 + GPU 优先编码 + RTMP/FLV 发布”的路线，不捕获桌面或最终效果窗口：

- 用户在桌面端输入完整 RTMP/RTMPS 发布地址；默认同时输出画面和声音，也可以只输出其中一种，至少选择一种。
- Rust/Tauri 是推流会话、取消、重试、状态和子进程的唯一所有者；Go 控制面不进入媒体数据路径。
- 视频从当前播放池活动源直接读取，使用现有 FFmpeg、libplacebo 和 GPU83 参数映射处理，不从 HWND、桌面或显卡表面截图。
- 声音从最终 PCM 总线分流，包含普通声音处理、插话文件、后续麦克风插话的最终混音；固定话术仍由 WebView2 直接出声，在其 PCM 尚未进入 Rust 总线前必须明确标记“固定话术未进入推流声音”。
- 视频优先使用 GPU 解码、libplacebo/GPU83 处理和硬件 H.264 编码；硬件路径失败后按能力单向降级，最终使用 OpenH264 CPU 兜底。
- 音频继续使用 CPU 执行 DSP 和 AAC-LC 编码，不为 AAC 引入 GPU 路径。
- 首个可交付版本固定输出视频规格并统一转码，不启用源视频 `-c:v copy`；不同源的分辨率、帧率、像素格式和编码参数不得改变同一次 RTMP 会话的轨道合同。
- 第一阶段允许在播放池换源时受控重建直接推流进程并向同一地址重新发布；只有用户确认必须跨条目无断流时，才实施常驻发布器和帧/PCM 馈送器，不预先引入该复杂度。
- mpv 预览仍是当前本地视频事实源。FFmpeg 的 `custom_shader_bin` 不能直接复用 mpv 的动态 `glsl-shader-opts` 更新合同，因此第一阶段推流视频只使用启动或换源时取得的完整参数快照；逐周期 GPU83 动态直推属于后续门禁，未完成前不得声称与本地每一帧完全一致。

## 2. 目标与非目标

### 2.1 目标

- 用户可输入任意合法的 RTMP/RTMPS 发布地址，例如 `rtmp://192.168.1.100:1935/live/gpal`。
- 用户可选择“画面 + 声音”“仅画面”“仅声音”，默认“画面 + 声音”。
- 不打开或依赖最终效果窗口也能开始直接推流；窗口遮挡、移动或最小化不影响输出。
- 视频处理开启时使用当前已准入的 GPU83 能力；关闭时输出中性/原始画面，但仍保持稳定的视频编码规格。
- 声音处理、插话和音量变化通过最终 PCM 总线进入推流，不采集系统声卡，不混入其他应用声音。
- 推流状态、实际编码器、输出轨道、已发布时间、重试次数、最近推进时间和脱敏错误可观测。
- 停止、换源、地址变化、播放停止、会话失败和应用退出都有明确取消、Join、进程树回收和缓冲释放路径。
- 支持远端 ZLMediaKit 的发布失败、连接断开、鉴权拒绝和短暂网络故障，重试有界且可取消。

### 2.2 非目标

- 不捕获桌面、窗口、摄像头或任意第三方应用画面。
- 不集成 OBS、libobs、平台 SDK、平台账号登录或多平台一键分发。
- 不让 Go 服务接收、转发、录制或存储用户音视频。
- 不自动管理或部署 ZLMediaKit；用户负责提供可访问的发布地址和必要鉴权参数。
- 不默认调用 ZLMediaKit REST API，也不要求用户提供 ZLMediaKit API `secret`；REST 只可作为后续可选诊断。
- 不把 RTMP 地址、token、用户名或密码写入普通日志、错误正文、截图、测试快照或前端构建产物。
- 不承诺首版在播放池换源时完全无断流；常驻发布器是独立后续阶段。
- 不承诺首版推流视频跟随本地 mpv 的每一次自动 GPU83 周期变化；只在取得动态参数和真实帧证据后晋级。
- 不扩展到实时话术幻化、ASR、LLM、TTS 改写、口型同步或检测规避。

## 3. 当前仓库事实

- 当前生产 mpv 固定 `--audio=no`，只负责最终视频表面；声音由独立链路输出。
- `audio_cycle_output.rs` 已经是普通声音、插话、duck、交叉淡化、音量和 PortAudio 输出的最终混音所有者，适合作为 PCM 分流点。
- `media_video_gpu_effects.rs` 已提供 `build_gpu83_video_filter`，能为 FFmpeg/libplacebo 生成 GPU83 兼容过滤计划；它是直接推流视频处理的复用入口，不代表动态周期已经接通。
- 随包 FFmpeg 已验证具备 `rtmp`、`rtmps`、FLV mux、AAC、libplacebo，以及 `h264_nvenc`、`h264_amf`、`h264_qsv`、`h264_mf`、`h264_vulkan`、`libopenh264` 等编码能力。
- `media_engine.rs` 已有硬件 H.264 编码器探测和尝试顺序；直接推流必须复用同一能力事实，不能新建第二份硬编码显卡判断。
- 当前最终效果的 GPU83 动态周期依赖 mpv JSON IPC 的单条 `glsl-shader-opts` 原子更新；FFmpeg/libplacebo 的 `custom_shader_bin` 为静态过滤参数，首版不能伪造同等动态能力。
- 当前固定话术使用 WebView2 `localService` 系统声音直接播放并暂时静音主轨，PCM 未进入 Rust 最终混音总线。

## 4. 用户交互契约

### 4.1 推流卡片

桌面右侧“声音/输出”区域新增 Ant Design“RTMP 推流”卡片，使用现有布局和公开组件，不覆盖 `.ant-*` 内部样式。第一版提供：

- 完整推流地址输入框。
- `输出画面`、`输出声音`复选框，默认都选中；两者都未选中时禁止开始。
- 输出分辨率：第一版固定或仅提供少量白名单，例如 `1280×720`、`1920×1080`。
- 帧率：第一版白名单 `25/30/50/60`，默认 `30`；`50/60` 必须通过 GPU 和真实网络门禁后才可用。
- 视频码率、音频码率使用有界白名单或数值范围，不提供任意 FFmpeg 参数输入。
- `开始推流`、`停止推流`主操作。
- 真实状态、已发布时间、实际编码器、输出轨道、当前码率、重试次数和脱敏错误。

推流过程中锁定地址、轨道组合和编码规格。用户修改草稿不改变实际运行会话；停止并重新开始成功后才提交新配置。

### 4.2 地址与敏感信息

允许示例：

```text
rtmp://192.168.1.100:1935/live/gpal
rtmp://stream.example.com/live/user001?token=opaque-value
rtmps://stream.example.com/live/user001
```

Rust 边界必须验证：

- scheme 只允许 `rtmp`、`rtmps`。
- 完整地址 UTF-8 长度不超过 `2048` 字节，不含控制字符、CR/LF 或空白前后缀。
- host 必须存在，显式端口必须在 `1..=65535`，路径必须至少包含发布目标。
- 地址通过参数数组交给 FFmpeg，禁止经过 shell 或拼接命令字符串。
- 错误、stderr 尾缓冲和结构化日志只显示 `scheme://host:port/<redacted>`，不得显示 userinfo、path 中可能的 stream key 或 query。
- 默认不持久化完整地址。若后续提供“记住地址”，非敏感 server/app 可进入普通本地配置，用户明确标记的秘密部分必须进入系统凭据库；第一版可直接不提供持久化以缩小风险。

### 4.3 轨道语义

| 选择 | FLV/RTMP 轨道 | 无源数据时的行为 |
| --- | --- | --- |
| 画面 + 声音 | H.264 + AAC | 视频暂缺发送黑帧/保持策略；音频暂缺发送静音或受控重连 |
| 仅画面 | H.264 | 禁止生成音轨；纯音频播放项输出黑色视频 |
| 仅声音 | AAC | 不启动视频解码、GPU 处理或视频编码 |

同一次发布过程中禁止热增删轨道。轨道选择变化必须停止旧会话并重新发布。

## 5. 媒体数据路径

### 5.1 第一阶段：直接进程

```text
当前活动源 ─┬─ 视频解码 → libplacebo/GPU83 快照 → H.264 ─┐
            │                                             ├→ FLV → RTMP/RTMPS → ZLMediaKit
最终 PCM ───┴──────────────────────────────→ AAC ─────────┘
```

- Rust 在开始推流时取得权威 `playback_generation + source_path + loop_index + source_duration_ms + source_pts_ms`。
- FFmpeg 视频输入从当前源的合法 source-local PTS 开始，并使用 `-re`/等价受控读速率，禁止把跨循环 presentation PTS 传给源文件 seek。
- 音视频模式的 AAC 输入来自 Rust PCM 管道；视频 FFmpeg 不自行读取源音轨，避免与最终混音双轨或不同步。
- 仅视频模式由 FFmpeg 视频时钟驱动；仅声音模式不启动视频 Worker。
- 换源或改变静态视频参数快照时，第一阶段停止旧进程、回收线程和管道，然后按最新权威身份重新发布到同一地址。
- ZLMediaKit 可能在换源时观察到短暂注销/重新注册；UI 必须显示“正在重新发布”，不能保持虚假的“推流中”。

### 5.2 后续阶段：常驻发布器

只有确认需要播放池跨条目无断流时实施：

```text
视频生产器 → 固定规格 NV12/硬件帧 ─┐
                                     ├→ 常驻编码/FLV/RTMP 发布器
最终 PCM 总线 ───────────────────────┘
```

- 发布器固定拥有 RTMP 连接、H.264/AAC 编码器、FLV muxer 和单调输出时间戳。
- 源切换只替换生产器，不关闭发布器；准备期间发送上一帧或黑帧和静音。
- 常驻发布器必须定义视频帧/PCM 有界缓冲、背压、丢弃策略、最大延迟和时间戳重建。
- 不允许用无界 rawvideo/PCM 队列换取“不断流”；当编码器跟不上时优先丢弃过期视频帧，声音不得无限积压。
- 该阶段需要单独性能基线，避免 `1080p30 NV12` 的大吞吐复制拖慢 mpv/PortAudio。

## 6. GPU 路线

### 6.1 推荐顺序

推流视频优先：

```text
硬件解码 → Vulkan/libplacebo GPU83 → 硬件 H.264 编码 → RTMP
```

编码候选按现有能力探测复用，目标顺序为：

1. NVIDIA `h264_nvenc`
2. AMD `h264_amf`
3. Intel `h264_qsv`
4. Windows Media Foundation `h264_mf`
5. CPU `libopenh264`

`h264_vulkan` 虽然随包可见，但在完成目标显卡、驱动、ZLMediaKit 和播放器兼容门禁前不加入默认生产顺序。

### 6.2 零拷贝边界

- “GPU 处理 + GPU 编码”不自动等于零拷贝。当前兼容过滤计划可能在 libplacebo 后执行 `hwdownload`，硬件编码器随后再上传。
- 第一阶段允许一次有界下载/上传，但必须记录 CPU/GPU 使用率、帧处理 P99、编码队列和 dropped frame；若 `1080p30` 不达标再做厂商专用零拷贝优化。
- 不在没有真实门禁时为 NVENC、AMF、QSV 各复制一整套业务状态机；差异只收口到硬件帧转换和编码器参数适配层。
- GPU 初始化或运行失败后按会话单向降级，禁止在同一次推流中来回震荡；重试新硬件路径需要新会话。

### 6.3 第一版编码基线

- 视频：H.264、`yuv420p`、默认 `1920×1080@30fps`、默认 `6000kbps`、GOP `2s`、低延迟参数、B 帧关闭或最小化。
- 音频：AAC-LC、`48kHz`、双声道、默认 `160kbps`。
- 输出：FLV muxer，目标为用户输入的 RTMP/RTMPS 地址。
- 分辨率、帧率、码率、GOP 和声道布局在一次会话内固定。
- 编码器专用参数必须由固定枚举生成；UI 不接收自由格式 FFmpeg option。

## 7. 音频分流

### 7.1 最终 PCM sink

在 `audio_cycle_output` 最终完成主轨增益、交叉淡化、插话 duck/叠加和限幅后增加可选 `RtmpAudioSink`：

```text
最终混音 PCM ─┬→ PortAudio ring → 本地设备
               └→ 有界 SPSC/管道 → FFmpeg AAC → RTMP
```

- 分流写入必须非阻塞，不能在 PortAudio callback 或音频生产关键路径等待 FFmpeg。
- 缓冲以毫秒和字节双上限约束；达到上限时返回结构化落后状态并触发推流重建，禁止静默积压。
- 暂停或媒体暂无声音时的静音策略必须与 RTMP 连接保活策略一起测试。
- 仅声音模式不要求最终效果窗口或 mpv 视频会话存在。
- 后续麦克风插话完成后，其清理后 PCM 进入同一最终总线即可自然进入推流。

### 7.2 固定话术限制

当前固定话术由 WebView2 系统声音直接播放，不在 Rust PCM 中。第一阶段必须二选一：

1. UI 明确显示“固定话术不进入当前推流音轨”，并将此作为未完成能力；或
2. 另立任务使用成熟 Windows 系统语音 API 将固定话术渲染为 PCM，再按既有优先级混入最终总线。

不得使用系统全局回环录音作为默认补丁，因为它会采集其他应用声音、引入权限和设备差异，并破坏单一最终音频事实源。

## 8. Rust 状态和资源所有权

### 8.1 状态模型

```rust
pub enum RtmpOutputState {
    Idle,
    Validating,
    Starting,
    Publishing,
    Reconnecting,
    Stopping,
    Failed,
}
```

运行快照至少包含：

- session generation；
- 脱敏目标；
- `video_enabled`、`audio_enabled`；
- 当前播放身份；
- 实际编码器、分辨率、帧率、目标码率；
- FFmpeg PID；
- 已发布时间、最近进度时间、输出字节和重试次数；
- 有界错误码与脱敏摘要。

### 8.2 模块放置

建议新增：

```text
desktop/src-tauri/src/rtmp_output/
├─ mod.rs          公共入口、状态机和所有权
├─ config.rs       地址、轨道和编码参数校验
├─ process.rs      FFmpeg 参数、进程监督、stderr/progress
└─ audio_sink.rs   最终 PCM 有界分流
```

第一阶段不创建抽象工厂或多协议通用框架；只有 RTMP/RTMPS 一种发布能力。常驻发布器确认实施后再新增 `publisher.rs`、`video_producer.rs`，避免为未批准能力预留空层。

### 8.3 生命周期

- `start`：校验地址和轨道 → 解析受信 FFmpeg → 取得权威媒体身份/参数快照 → 探测编码器 → 建立音频管道 → 启动受管进程 → 等待输出进度 → 发布 `Publishing`。
- `stop`：推进 generation → 取消重试/读写线程 → 尝试有界优雅退出 → Job Object 终止兜底 → Join → 关闭管道 → 清空秘密和状态。
- 换源：第一阶段按最新身份受控停止并重新发布；迟到的旧进程退出或进度不得覆盖新状态。
- 地址或轨道草稿变化：只更新前端草稿，不触碰运行会话。
- 应用退出：先停止 RTMP，再停止最终音频和 mpv；不得留下 FFmpeg 或后台线程。

## 9. 重试和错误合同

- 地址解析失败、轨道为空、当前无可用媒体、资源缺失和编码器全部失败属于确定性错误，不自动重试。
- DNS、连接拒绝、TLS 握手、短暂断管和发送超时属于可重试错误。
- 建议退避：`1s / 2s / 4s / 8s / 15s`，最多 5 次；用户停止、换地址或应用退出立即取消。
- 每次重试复用同一用户意图，但必须创建新 session attempt；旧 attempt 的进度和 stderr 按 generation 丢弃。
- stderr 尾缓冲继续有行数、字节和单行上限，保存前执行 URL/路径脱敏。
- ZLMediaKit 鉴权拒绝必须显示可操作原因，但不得回显秘密参数。
- FFmpeg 进程存活不等于推流成功；只有输出时间/字节持续推进并通过启动宽限，才可显示 `Publishing`。

## 10. Tauri IPC

建议最小命令：

- `validate_rtmp_output_config`
- `start_rtmp_output`
- `stop_rtmp_output`
- `get_rtmp_output_status`

第一阶段采用主窗口有界轮询或既有低频状态同步，不建立高频事件流。所有命令只允许 `main` 窗口调用；最终效果窗口无权读取或修改完整目标地址。

请求 DTO 使用明确字段，不接受原始 FFmpeg 参数：

```text
target_url
video_enabled
audio_enabled
width
height
fps
video_bitrate_kbps
audio_bitrate_kbps
```

服务端 DTO 返回脱敏目标，不返回完整 URL。

## 11. 实施阶段

### Phase 0：文档与失败测试

- 同步 PRD、系统架构、桌面架构、参数契约、长计划和项目 AI 约束。
- 增加地址校验、轨道组合、脱敏、状态机、命令权限和退出清理的失败测试。
- 固定第一阶段换源会短暂重新发布、视频使用静态快照、固定话术不在 PCM 的限制。

### Phase 1：FFmpeg 直接发布技术门禁

- 在本地开发机使用随包 FFmpeg 向真实 ZLMediaKit 验证三种轨道组合。
- 分别验证 NVENC、AMF、QSV、Media Foundation 和 OpenH264；没有对应硬件时记录未验证，不伪造通过。
- 验证用户地址中的 query/token 能发布且日志完全脱敏。
- 验证 H.264/AAC/FLV 轨道在 ZLMediaKit 与至少一个实际播放器中可读。

### Phase 2：Rust 会话和 GPU 视频

- 建立 `rtmp_output` 状态、配置、进程监督和生命周期。
- 复用 `build_gpu83_video_filter` 与硬件编码器探测。
- 按当前 source-local PTS 开始，完成暂停、停止、循环和换源受控重建。
- 失败保持本地播放，不得让 RTMP 故障改变 mpv/PortAudio 权威状态。

### Phase 3：最终声音总线

- 增加有界非阻塞 PCM sink，接入 AAC 输入。
- 覆盖普通声音处理、插话、音量、静音、暂停、无音轨和纯音频项。
- 固定话术在 PCM 接入前保持明确限制；若用户要求完整进入推流，再实施系统 TTS PCM 子任务。

### Phase 4：桌面 UI

- 新增 RTMP 卡片、草稿/运行配置分离、三种轨道选择和真实状态。
- 覆盖 loading、starting、publishing、reconnecting、failed、stopping 和权限错误。
- 地址错误只展示脱敏摘要，复制/保存行为不得泄漏秘密。

### Phase 5：长稳和发布门禁

- 完成远端 ZLMediaKit、真实网络断开/恢复、鉴权拒绝和地址变化测试。
- 覆盖至少 30 分钟、多个播放池条目、纯视频、纯音频、视频无音轨和混排。
- 验证 CPU/GPU/RSS、句柄、线程、FFmpeg PID、缓冲和重试无持续增长。
- 完成 Windows 10/11 x64 与可用的 NVIDIA、AMD、Intel、CPU-only 门禁。

### Phase 6（可选）：常驻无断流与动态 GPU83

- 只有首版换源重发布或静态快照不能满足用户验收时进入。
- 先建立常驻发布器和固定轨道时间戳，再接视频生产器。
- 动态 GPU83 必须提供与 mpv 等价的原子参数入口和逐帧证据；不能通过频繁重启 FFmpeg 冒充平滑周期更新。

## 12. 验收标准

- 默认配置实际发布 H.264 + AAC；取消声音后只有 H.264，取消画面后只有 AAC。
- 用户填写的合法 RTMP/RTMPS 地址可用；非法 scheme、端口、空 host、控制字符和超长地址在 Rust 边界拒绝。
- 推流不依赖最终效果窗口，不读取桌面或 HWND，不受窗口遮挡、移动和最小化影响。
- GPU 可用时状态显示实际硬件编码器；失败按单向顺序降级到 OpenH264，并保留脱敏原因。
- RTMP 故障不停止、不暂停、不换代本地播放；本地播放失败也不能留下无人管理推流进程。
- 声音处理和插话的实际最终 PCM 可在推流端听到；固定话术未接入前 UI 明确限制。
- 远端断开后按有界退避重试，用户停止能立即取消；超过上限进入 `Failed`，不得无限重连。
- 所有日志、错误和状态不包含完整目标路径、query、token、用户名或密码。
- 应用退出、停止、换源和失败后无 FFmpeg、管道线程、Job Object 子进程或无界缓冲残留。
- 第一阶段只声明“启动/换源静态参数快照已应用”；没有动态证据时不得声明推流跟随全部 GPU83 周期。

## 13. 验证命令与人工门禁

开发完成后至少执行：

```powershell
cd desktop\src-tauri
cargo fmt --all -- --check
cargo check --all-targets
cargo clippy --all-targets -- -D warnings
cargo test --all-targets

cd ..\ui
pnpm test
pnpm exec tsc --noEmit
pnpm run build
```

人工门禁：

1. 分别向真实 ZLMediaKit 发布音视频、仅视频、仅音频。
2. 使用 ZLMediaKit 播放地址或实际播放器核对轨道、分辨率、FPS、码率、声音和时间推进。
3. 验证 GPU 编码器选择、驱动失败、编码会话占满和 CPU 回退。
4. 播放池跨视频、纯音频和无音轨视频，观察第一阶段重新发布时长和可理解状态。
5. 断开网络、停止 ZLMediaKit、返回鉴权拒绝、恢复网络，核对有界重试和取消。
6. 运行 30 分钟后核对 CPU/GPU/RSS、句柄、线程、PID 和缓冲无持续增长。

## 14. 依赖和参考

首版不新增 OBS、libobs、窗口捕获或第二套媒体服务器依赖。优先复用仓库随包 FFmpeg、libplacebo、现有 GPU83 映射、PortAudio 和 Rust 进程监督能力。

- FFmpeg RTMP 协议：<https://ffmpeg.org/ffmpeg-protocols.html#rtmp>
- FFmpeg FLV mux/编码能力以随包二进制 `-muxers/-encoders/-protocols/-filters` 实测为准
- ZLMediaKit 官方仓库 RTMP 推流示例（H.264 + AAC + FLV）：<https://github.com/ZLMediaKit/ZLMediaKit/wiki/ZLMediaKit推流测试>
- ZLMediaKit 播放 URL 规则：<https://docs.zlmediakit.com/zh/guide/media_server/play_url_rules.html>
- ZLMediaKit REST API（可选诊断，不作为首版依赖）：<https://docs.zlmediakit.com/zh/guide/media_server/restful_api.html>
