# Phase 3：Rust 调度与导入准备

Parent PRD: [当前文案克隆语音循环播放](../prd-voice-clone-current-playback.md)

Status: Not Started
Last Updated: 2026-08-14

## Objective

把导入后的自动人声准备、当前文案音频生成、缓存命中、取消、超时和旧结果拒绝接入 Tauri 命令，同时保留旧局部替换和其他播放能力。

## Context From Master PRD

- Goals covered: G-1、G-2、G-6、G-7
- Success Criteria: SC-1、SC-2、SC-6、SC-7
- Requirements covered: FR-1、FR-2、FR-3、FR-7、FR-8、NFR-1、NFR-3、NFR-5

## Phase Discovery Gate

- [ ] 阅读 `desktop/src-tauri/src/commands.rs` 中 `prepare_voice_clone_source`、`start_voice_clone_replacement`、Worker 安装/回收和缓存读写函数。
- [ ] 搜索 `prepare_voice_clone_source`、`start_voice_clone_replacement`、`cancel_voice_clone_operation` 的 UI 和测试调用方。
- [ ] 确认常驻 `VoiceCloneWorkerServer` 的互斥规则，不能让新生成任务和旧 Worker 任务同时占用同一进程。
- [ ] 核对 `voice_clone_model_root`、`voice_clone_cache_root`、正式资源目录和临时产物清理策略。

## Scope

### In Scope

- 导入后自动触发已有 `prepare_voice_clone_source`。
- 新增当前文案音频准备/启动命令及缓存命中路径。
- 结果完整校验、过期拒绝、取消、超时和资源清理。

### Out of Scope

- 不在 Rust 中实现 TTS。
- 不修改普通声音处理和实时音频候选优先级。
- 不让新命令生成完整源音轨或 MP4。

## Implementation Checklist

- [ ] 在 `desktop/src-tauri/src/commands.rs` 增加当前文案准备请求 DTO、Worker 请求 JSON、结果校验和缓存 manifest 读写函数；manifest 必须记录源/参考/文案/模型身份、音频 SHA、时长、格式和创建时间。
- [ ] 增加 `prepare_voice_clone_playback` 命令：验证当前源、已准备参考人声和单条文本；缓存命中时直接写入 `track_ready`，不得启动 Worker。
- [ ] 缓存未命中时复用常驻 Worker，写入 `.partial` 请求/结果/进度文件，设置超时和 CancellationToken；成功后先完成 FFprobe、完整 SHA-256 和源/文本身份校验，再原子提交。
- [ ] 增加 `start_voice_clone_playback` 或等价命令：只接受当前已 ready 的 track 和当前位置，进入 `playing`；不接受文案数组、下一个 preset 参数或“按循环自动播放”参数。
- [ ] 增加结束/取消/停止命令，使 Rust 状态回到 `ready` 或 `idle`，并返回前端恢复原音轨所需的 snapshot。
- [ ] 源视频代际改变、文案 SHA 改变或窗口解绑时拒绝旧结果，并清理对应临时目录；正常 `loop_index` 变化不得使正在播放的克隆音频失效。
- [ ] 在 `desktop/src-tauri/tests/voice_clone_runtime_contract.rs` 增加缓存命中不启动 Worker、文案变化失效、循环不触发新播放、旧 generation/text 结果拒绝和取消清理测试。

## Validation Strategy

通过 Rust 集成测试验证调度和状态，不依赖真实 XTTS 模型；Worker 调用使用受控临时文件和 fake 输出。重点验证失败时 snapshot 不会误报 `playing`。

## Validation Checklist

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo test --all-targets --test voice_clone_runtime_contract`
- [ ] `cargo clippy --all-targets -- -D warnings`
- [ ] 验证相同缓存键不会启动第二个 Worker。
- [ ] 验证失败、取消、超时和 stale 都删除临时文件但保留旧可用音频。

## Exit Criteria

- [ ] 导入后可自动准备参考人声，已有源缓存不重复分离。
- [ ] 当前文案可生成/命中一条独立克隆音频。
- [ ] Rust 不会提交旧视频、旧文案的 Worker 结果；正常视频循环不会误判当前克隆播放过期。
- [ ] 新命令不破坏旧替换、实时音频和普通声音处理命令。

## Phase-End Multi-Pass Review

- [ ] 1. 导入、生成、播放、结束、循环不触发和更换视频的状态连接完整。
- [ ] 2. 所有异步边界都有 generation、loop、text SHA 和 operation ID 防护。
- [ ] 3. 缓存读取路径比启动 Worker 更简单且可解释。
- [ ] 4. 命令职责清晰，没有把 React 业务条件复制进 Rust。
- [ ] 5. 清理未使用 DTO、辅助函数、导入和旧调试日志。
- [ ] 6. 文件路径和模型资源来源均受控。
- [ ] 7. Worker 并发和超时有上限，JoinHandle 有所有者。
- [ ] 8. 集成测试覆盖错误恢复和资源清理。
- [ ] 9. UI 所需 snapshot 字段已经稳定。
- [ ] 10. 主 PRD 和 Phase 4 的播放事件方案已同步。
