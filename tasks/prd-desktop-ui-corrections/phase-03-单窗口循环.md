# Phase 3: 单窗口循环

Parent PRD: [PRD: 桌面端主页与独立播放窗口体验修复](../prd-desktop-ui-corrections.md)
Status: Implemented; short-video regression pending
Last Updated: 2026-08-14

## Objective

修复视频结束、接近结束、换源和 IPC 失败时的连续播放行为，确保同一个源视频在同一个 `final-effect` 窗口内持续循环。

## Context From Master PRD

- Goals covered: G-5, G-6
- Success Criteria: SC-5, SC-6
- Requirements covered: FR-6, FR-7, NFR-4
- Key scenarios touched: Scenario 3

## Phase Discovery Gate

Before editing code, re-check:

- [ ] `desktop/ui/src/App.tsx` 的 video `onEnded`、`src` 更新、音频候选切换和暂停/停止处理。
- [ ] `desktop/ui/src/播放循环.ts` 的去重算法；确认不能用同一个 playback generation 阻塞第二轮。
- [ ] `desktop/src-tauri/src/lib.rs` 的 `start/pause/resume/stop/complete_loop` 状态转移。
- [ ] `desktop/src-tauri/src/commands.rs` 的 `complete_playback_loop`、窗口权限和固定 label。
- [ ] Phase 1/2 是否已把可见状态和运行时消息移出播放器。

## Scope

### In Scope

- `ended` + `timeupdate` 双路径、单事件去重、换源重置和本地优先重启。
- 音频候选在循环边界的清理/恢复、处理后视频切换和播放失败可见错误。
- 单窗口 show/focus 与关闭后重开。

### Out of Scope

- 不改变 Rust 播放状态模型的产品语义，不引入双视频队列或第二个隐藏视频窗口。
- 不把轮次同步失败伪装成成功；只保证媒体本地播放不中断。

## Implementation Checklist

- [ ] 在 `desktop/ui/src/播放循环.ts` 将去重输入改成表达“当前源 + 当前结束事件”的 token/门禁，不使用固定 playback generation 阻止同源后续轮次；补充 `ended`、接近末尾、同一事件重复、同源第二轮、新源和暂停/停止测试。
- [ ] 在 `desktop/ui/src/App.tsx` 增加 `restartToNextLoop`：先设置媒体 currentTime=0、同步重置候选音频位置并调用 `play()`，再异步调用 `complete_playback_loop`；Promise 失败只更新主页错误/同步状态，不撤销本地播放。
- [ ] 在 `<video>` 上同时接入 `onEnded` 和 `onTimeUpdate`，两者共享同一个结束事件 token；禁止 `onEnded` 与末尾 `timeupdate` 双增轮次。
- [ ] 在源视频引用、`playback_generation`、停止、暂停和处理后视频引用变化时重置循环门禁；旧 `play()` Promise 或旧 snapshot 不得覆盖新源。
- [ ] 在循环边界确认 `audio` 候选的时间偏移、`pending_audio_candidate` 和原声回退不阻塞视频；候选不完整或 IPC 失败时保持上一条可用音轨。
- [ ] 在主页的打开/聚焦播放器按钮和 Tauri 命令处保留固定 `final-effect` label；已有窗口只 show/focus，关闭后主窗口和心跳不受影响。
- [ ] 若 Rust 逻辑无需改动，只补 `desktop/src-tauri/src/lib.rs`/`commands.rs` 现有状态转移测试或静态契约测试，避免为了前端事件修复扩大领域层。

## Validation Strategy

循环是竞态敏感的行为，先用无时间等待的纯逻辑测试证明门禁，再用现有 Rust playback tests 验证状态转移，最后用 3–6 秒带音视频的本地受支持格式视频手工验证至少两轮、暂停/继续、停止、关闭/重开和 IPC 失败降级；至少抽样覆盖非 MP4 格式。

## Validation Checklist

- [ ] `cd desktop/ui && node --test src/运行时参数自动调度.test.mjs` 或新增循环测试通过。
- [ ] `cd desktop/src-tauri && cargo test --all-features` 通过。
- [ ] 短视频自然结束两次仍只有一个播放窗口，主页轮次按结束事件递增。
- [ ] 让 `complete_playback_loop` 同步失败时，视频仍从 0 秒继续；主页显示状态同步错误但不黑屏。
- [ ] `ended` 和 `timeupdate` 同时到达只触发一次本地重启和一次轮次同步。
- [ ] 处理后视频换源时不播放旧源，不重复创建窗口，候选音频不跨代际泄漏。

## Exit Criteria

- [ ] 同一个源视频连续两轮以上自动播放，不停在结束帧、不重新弹窗、不生成队列版本。
- [ ] 单窗口约束和错误/取消/停止路径可观察、可恢复。
- [ ] 循环测试不依赖任意长 sleep。

## Phase-End Multi-Pass Review

- [ ] 1. 对照 G-5/G-6、SC-5/SC-6 和 FR-6/FR-7。
- [ ] 2. 复核 ended、timeupdate、换源、暂停、停止和播放失败。
- [ ] 3. 复核异步 snapshot/Promise 乱序和跨代际候选。
- [ ] 4. 复核没有引入第二视频队列、窗口或全局可变状态。
- [ ] 5. 删除旧的不可达循环辅助逻辑和未使用 ref。
- [ ] 6. 检查 IPC 错误没有泄露路径、Secret 或完整媒体内容。
- [ ] 7. 检查本地循环没有高频无界请求或媒体资源泄漏。
- [ ] 8. 复核 Rust 测试与手工短视频证据。
- [ ] 9. 根据结果更新 Phase 4 的回归清单。
- [ ] 10. 同步主 PRD 和阶段变更记录。

## Discoveries / Decisions

- 当前 `complete_playback_loop` 的 Rust 语义已经符合单源循环；主要缺陷是前端等待 IPC 后才重启媒体，以及已有循环 helper 的 token 语义不适合多轮同源播放。

## Phase Change Log

- 2026-08-14: Phase 3 创建。
- 2026-08-14: 完成 ended/timeupdate 门禁、本地先回零播放、异步轮次同步和主页控制消息；循环纯逻辑测试及 Rust 测试通过。
