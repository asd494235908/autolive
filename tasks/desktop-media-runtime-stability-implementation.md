# 桌面媒体运行时稳定性实施方案

> 日期：2026-08-25  
> 状态：代码修复、自动化门禁与本地测试包构建已完成；待目标 Windows 实机长稳验收  
> 范围：Rust/Tauri 桌面执行层、PortAudio/FFmpeg 驱动边界、最终效果 WebView 资源生命周期  
> 非范围：Go 控制面、管理端、实时话术幻化、推流、服务器构建、无关代码清理

## 1. 问题与目标

当前桌面端的主要风险不是某一个 `BUFFERSIZE` 常量，而是视频重编码、音频生产、硬件输出和 WebView 诊断各自按高频节拍工作，叠加后形成持续 CPU 占用、重复解码、缓冲积压和资源不能及时释放。已确认的高风险点包括：

1. 视频周期会反复触发完整 FFmpeg 重编码；同一素材可能长期处于“播一份、转一份”的状态。
2. 普通声音同时保留 current/N+1 两个 FFmpeg 生产者，输出侧还存在 `1ms` 轮询、临时 `Vec` 拷贝和高频重分析。
3. PortAudio 停止/关闭失败时的原始流指针所有权不清晰，存在失败后继续访问无效指针的风险；单声道路径还混用了 sample/frame 单位。
4. 最终效果窗口即使没有进入画中画，也维护隐藏视频解码；PortAudio 已接管时 Web Audio 图仍可能保持活动。
5. FFmpeg 编码器失败会遍历多个编码器，每次沿用过大的独立超时；音频合法空输出可能形成重启风暴。
6. IPC/WebView 数据虽然已有局部校验，但嵌套数组、状态组合和数值边界仍有缺口。

本方案先恢复资源上界和生命周期正确性，再以真实基线决定是否需要更深的流水线改造。目标是：

- 任意时刻最多一个活动视频渲染任务、一个 current 音频生产者和一个 N+1 音频生产者。
- 稳态播放期间不因 8～15 秒视频周期反复启动整段视频重编码；视频周期只预选下一快照，实际渲染在受控应用边界执行。
- PortAudio 环缓、候选预热和 FFmpeg channel 全部保持现有有界容量，生产者在下游背压时等待，不以忙轮询扩大 CPU。
- 诊断计算只在新 PCM 到达且达到发布节拍时执行，不能反向阻塞 PCM 生产和硬件输出。
- 画中画、Web Audio、对象 URL、媒体监听器、定时器和 FFmpeg/PortAudio 资源都有对称创建/释放。
- 最终效果窗口继续是客户观看的纯媒体表面，所有结构化错误只进入主操作窗口诊断通道。

## 2. 功能边界

### 2.1 必须保持

- 当前进程内 `1～100` 项本地有序播放池、单一最终效果窗口和一个活动媒体源。
- 视频处理与普通声音处理开关独立；处理失败保持当前可用媒体并回退，不把错误文字显示在最终效果窗口。
- 普通声音 `current + N+1 + 单 PortAudio` 上界、30ms 交叉淡化、候选严格位于当前绝对媒体时间之后 `60000ms` 内。
- FFmpeg/FFprobe 继续使用随包成熟二进制和统一后台进程边界，不自行实现编解码器。
- 现有正式媒体参数、范围、默认值和三态能力契约。

### 2.2 本批次不做

- 不引入新的媒体框架、内存池库、异步运行时或自定义编解码器。
- 不重写完整播放引擎，不恢复实时话术候选、RTMP/OBS 或离线版本队列。
- 不删除与本次修复无直接关系的存量未使用代码；是否执行扩展清理等待用户确认。
- 不在服务器构建，不使用 Git worktree。

## 3. 实施分组

### A. PortAudio、缓冲和诊断

责任文件：

- `desktop/crates/autolive-portaudio-output/src/lib.rs`
- `desktop/src-tauri/src/audio_cycle_output.rs`
- `desktop/src-tauri/src/audio_mixer.rs`
- `desktop/src-tauri/src/audio_output_diagnostic.rs`
- `desktop/src-tauri/src/audio_feature_analysis.rs`

实施内容：

1. 先增加失败测试，覆盖 PortAudio stop/close 失败、流指针所有权、mono/stereo frame 换算和缓冲上限。
2. 将硬件流生命周期收口为明确状态机；只有成功 close 后才释放句柄，失败状态不再把悬空指针暴露给后续操作。
3. 所有容量、健康和水位统一以 frame 为跨层单位，在 API 边界显式换算 interleaved sample，使用 checked/saturating 算术拒绝溢出。
4. 把 `1ms` 忙轮询改为有界 channel/阻塞等待或不低于一个音频块节拍的退避；避免每轮复制完整 `Vec`，只移动所有权或复用固定工作缓冲。
5. 重分析改为“新 PCM + 最短 250ms 发布节拍”门禁；波形轻量采样与 FFT/MFCC/LPC 重分析分开，分析失败只标记诊断不可用。
6. 合法短时空输出不触发重启；只有有效 PCM 连续 `1500ms` 不推进或 FFmpeg 明确 EOF/失败时进入既有恢复链。

### B. FFmpeg 与视频渲染

责任文件：

- `desktop/src-tauri/src/media_engine.rs`
- `desktop/src-tauri/src/commands.rs`
- `desktop/ui/src/App.tsx` 中视频周期的受控应用入口

实施内容：

1. 先增加编码器失败分类和共享 deadline 测试；输入/滤镜/磁盘/取消/超时错误不得继续遍历编码器，只有明确的编码器不可用才尝试下一项。
2. 一个媒体处理请求只持有一个总 deadline；后续编码器尝试消费剩余预算，不能每次重新获得最长 `6h` 超时。
3. 保留单飞和 latest-wins 合并，但停止视频周期到点后立即触发整段重编码。周期仍预选下一快照，实际渲染只在首次开启、用户人工应用、换源或下一条受控媒体边界执行。
4. 视频缓存继续采用 `.partial`、探测、哈希和原子提交；换源、关闭处理、窗口退出和 generation 变化时取消旧任务并释放旧缓存引用。
5. 为视频处理任务、活动缓存项和最近错误增加低基数诊断字段，便于实机确认没有周期性 FFmpeg 重启。

### C. WebView、画中画和类型边界

责任文件：

- `desktop/ui/src/App.tsx`
- 相关 `desktop/ui/src/*.test.mjs`
- 必要时新增单一职责的解析/状态模块，不把逻辑继续堆入 `App.tsx`

实施内容：

1. 画中画媒体元素改为用户触发时才装载和同步；未进入画中画时不设置源、不 seek、不持续解码，退出后立即 pause、移除 src 并 `load()` 释放解码器。
2. Web Audio 只服务 WebView 回退/插话诊断；PortAudio 健康接管后 suspend/disconnect，回退时按单飞恢复，卸载时 close。
3. 页面隐藏、停止、暂停和窗口退出时停止不必要轮询；恢复后读取一次最新快照再重启有界节拍。
4. 补齐 IPC DTO 的安全整数、有限数、数组长度、枚举、嵌套对象和跨字段组合校验；非法数据拒绝进入状态，不用默认值掩盖结构错误。
5. 保持最终效果窗口错误不可见；错误通过现有诊断 channel 进入主窗口。
6. 当前版本不注册、不轮询也不启动 `speech-to-speech` 能力与 Worker；历史兼容 IPC 只作后续版本审计，不进入最终效果页运行链。

## 4. 实施顺序

1. 冻结目标文件基线，记录已有工作区改动，避免覆盖用户代码。
2. 三组先分别提交能复现风险的测试，再做最小实现。
3. 子线程完成后，主线程在用户确认后按“代码总监复核”检查 diff：所有权、数组边界、任务取消、错误分类、文档一致性和未使用代码。
4. 只修复审核发现的本次范围问题；扩展删除存量未使用代码需另行确认。
5. 完成自动化门禁后再做本地 Windows 测试包构建，不将源码上传服务器。

## 5. 验收标准

### 5.1 自动化

- `cd desktop/src-tauri && cargo fmt --check`
- `cd desktop/src-tauri && cargo test --workspace`
- `cd desktop/src-tauri && cargo clippy --workspace --all-targets -- -D warnings`
- `cd desktop/ui && pnpm test`
- `cd desktop/ui && pnpm exec tsc --noEmit`
- `cd desktop/ui && pnpm run build`

### 5.2 运行时

- 1080p 素材播放 30 分钟：稳定阶段不再每 8～15 秒产生视频 FFmpeg 重编码进程。
- 运行时 FFmpeg 上界：视频渲染 `≤1`；普通声音 `current≤1`、`N+1≤1`；N+2 无进程。
- PortAudio ring 不超过配置容量，frame/sample 换算在 mono/stereo 测试均正确，无越界、悬空指针和重复 close。
- 诊断重分析频率 `≤4Hz`，没有新 PCM 时不重复执行 FFT/MFCC/LPC。
- 从预热完成后第 5 分钟到第 35 分钟，桌面进程 RSS 不呈持续线性增长；目标斜率 `≤10MiB/10min`，对象 URL、AudioContext、画中画解码器和媒体监听器数量稳定。
- PortAudio 健康时 Web Audio 图处于 suspend/释放状态；退出 PortAudio 后 WebView 原声能恢复。
- 注入 stop/close/FFmpeg 失败、合法空输出、候选 stale、窗口关闭和换源：不中断当前可用媒体，不在最终效果窗口显示错误。

## 6. 回滚与剩余风险

- 三组改动保持文件边界，可按组回滚；不迁移数据库、不改变 OpenAPI、不修改历史迁移。
- 若受控视频应用边界不能满足真实画面变化需求，后续应采用单次解码的实时 GPU/视频滤镜流水线；不得恢复“每周期整段离线重编码”作为长期方案。
- CPU/RSS 阈值必须在目标 Windows 10/11 x64 新机和至少一台低配机器复测；自动化只能证明资源上界与生命周期，不能代替声卡、驱动和真实 FFmpeg 的长稳测试。

## 7. 交付记录

- [x] 方案与范围已写入项目文档。
- [x] A 组：PortAudio、缓冲和诊断。
- [x] B 组：FFmpeg 与视频渲染。
- [x] C 组：WebView、画中画和类型边界。
- [x] 用户确认后的代码总监复核与本次范围无用代码/导入/调试项清理。
- [x] Rust、桌面 UI 自动化门禁和本地 Windows 测试包构建。
- [ ] 目标 Windows 10/11 x64 新机、真实声卡/驱动和 30 分钟 CPU/RSS 长稳验证。

代码总监复核补充修正：

- 音频输出直接借用 `VecDeque` 的连续立体声帧，仅在环尾不足一个完整帧时整理缓冲；驱动/健康状态刷新统一降到 `250ms`。
- FFmpeg 的单请求 deadline 覆盖 FFprobe、编码器候选、音频产物校验以及提交前后哈希检查；只有明确的编码器初始化/不可用错误才尝试下一编码器。
- 最终效果页删除本版禁止启用的实时话术能力查询与 `250ms` Worker 调度；无媒体源时不启动 Web Audio 重试，页面隐藏/非播放状态停止非必要轮询。
- 画中画资源生命周期不再随播放状态变化反复释放；PortAudio 状态通过运行时 DTO 校验进入 UI，移除未使用的响应类型与测试外诊断计数。
- 媒体缓存 asset scope 改为受控缓存目录授权，静态契约同步到实际安全边界；未删除任何无关存量代码。

当前自动化基线：PortAudio crate `27/27`、Rust workspace `374/374`、桌面 UI `282/282`；TypeScript 类型检查、Rust fmt、全目标 Clippy、UI production build、Rust workspace build 和 `tauri:build:test` 均通过。已生成含 FFmpeg/FFprobe、PortAudio、环境声资源和 WebView2 离线安装器的 NSIS 测试安装包及内部 portable ZIP。尚未在另一台全新 Windows 电脑安装运行，也未完成真实驱动 30 分钟 CPU/RSS 长稳采样，因此不得把本地构建结果描述为跨机实测通过。
