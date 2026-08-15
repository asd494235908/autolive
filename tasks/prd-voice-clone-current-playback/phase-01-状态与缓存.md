# Phase 1：状态、契约与缓存

Parent PRD: [当前文案克隆语音循环播放](../prd-voice-clone-current-playback.md)

Status: Not Started
Last Updated: 2026-08-14

## Objective

建立一套独立于旧局部替换的“当前文案克隆播放”状态、请求/结果契约和缓存键，使后续 Worker 与 UI 能判断：当前文案是否已生成、是否正在播放、是否需要恢复原音轨。

## Context From Master PRD

- Goals covered: G-2、G-3、G-4、G-5、G-6、G-7
- Success Criteria: SC-2、SC-3、SC-4、SC-5、SC-6、SC-7
- Requirements covered: FR-2、FR-3、FR-5、FR-6、FR-7、FR-8、NFR-2、NFR-5

## Phase Discovery Gate

- [ ] 重新阅读 `desktop/src-tauri/src/lib.rs` 中 `PlaybackSnapshot`、`PlaybackCore::complete_loop` 和当前音频来源解析。
- [ ] 重新阅读 `desktop/src-tauri/src/voice_clone.rs` 的文本校验、源代际校验和现有替换结果校验。
- [ ] 搜索 `voice_clone_replacement` 全部调用方，确认新字段不会覆盖旧兼容状态。
- [ ] 核对 `desktop/src-tauri/tests/voice_clone_runtime_contract.rs` 的源变更、循环和过期结果测试。
- [ ] 如果现有状态命名或循环边界与本阶段方案冲突，先更新主 PRD 和后续阶段文件。

## Scope

### In Scope

- 新增当前文案克隆播放状态和音频片段结果类型。
- 定义单条当前文案的缓存身份和旧结果拒绝条件。
- 定义克隆音频结束后恢复原音轨所需的播放上下文。

### Out of Scope

- 不实现 Python TTS 调用。
- 不修改 React 播放事件。
- 不删除旧局部替换状态。

## Implementation Checklist

- [ ] 在 `desktop/src-tauri/src/voice_clone.rs` 增加 `VoiceClonePlaybackTrack`、`VoiceClonePlaybackState`、`VoiceClonePlaybackPlan` 和 `VoiceClonePlaybackResult`，字段至少包括 source generation/path/SHA、loop index、text SHA/text、reference SHA、audio reference/SHA/duration/sample rate/channel count/model/operation ID。
- [ ] 在 `desktop/src-tauri/src/lib.rs` 的 `PlaybackSnapshot` 和 `PlaybackCore` 增加独立的当前文案播放状态，默认状态为 `idle`，源变更时清理当前播放产物但保留可重新准备的源索引。
- [ ] 增加只接受单个 `text` 的启动计划函数；函数不得接受数组、队列或多个 preset ID。
- [ ] 增加 `mark_voice_clone_playback_ready`、`start_voice_clone_playback`、`finish_voice_clone_playback`、`cancel_voice_clone_playback` 等状态转换，并在不匹配 source generation/text SHA/loop index 时返回结构化 stale 错误。
- [ ] 增加缓存键函数：`source_sha256/reference_audio_sha256/text_sha256/model/sample_rate/channel_count` 规范化后以固定顺序拼接并哈希，不把用户可变路径作为唯一身份。
- [ ] 为状态转换、文案唯一性、源变更清理、循环不触发克隆播放和旧结果拒绝编写 Rust 测试。

## Validation Strategy

先用纯 Rust 单元测试证明状态机和缓存键，再用现有 runtime contract 测试证明源变更和循环不会影响视频/普通音频开关。此阶段不需要启动模型。

## Validation Checklist

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo test --all-targets voice_clone`
- [ ] `cargo test --all-targets --test voice_clone_runtime_contract`
- [ ] `cargo clippy --all-targets -- -D warnings`
- [ ] 验证旧 `VoiceCloneReplacementState` 测试仍通过。

## Exit Criteria

- [ ] 新状态能表达 idle/preparing/ready/playing/finished/failed/cancelled/stale。
- [ ] 一次状态只关联一条当前文案，不能注册文案队列。
- [ ] 视频循环不会改变当前克隆播放状态；源或文案变化会使缓存失效。
- [ ] 旧局部替换行为的测试不被破坏。

## Phase-End Multi-Pass Review

- [ ] 1. 状态覆盖播放、恢复、循环不触发、跨循环和取消路径。
- [ ] 2. source generation、loop index、text SHA 校验没有遗漏。
- [ ] 3. 没有为批量文案或未来队列增加无调用方抽象。
- [ ] 4. 新类型职责清晰，未把 Worker I/O 放进 `PlaybackCore`。
- [ ] 5. 删除新增未使用导入、类型和导出。
- [ ] 6. 缓存身份不包含明文 secret 或不受控路径依赖。
- [ ] 7. 状态更新不引入无界缓存或后台任务。
- [ ] 8. 测试能证明旧状态仍保持兼容。
- [ ] 9. 后续 Worker/Rust/UI 阶段的字段名已统一。
- [ ] 10. 主 PRD 状态和变更记录已同步。
