# Phase 4：React 播放与循环同步

Parent PRD: [当前文案克隆语音循环播放](../prd-voice-clone-current-playback.md)

Status: Not Started
Last Updated: 2026-08-14

## Objective

在现有单窗口播放器中实现“当前文案点击后播放一次、原音轨临时静音、克隆音频结束恢复”，并确保视频循环不会自动重新播放文案。

## Context From Master PRD

- Goals covered: G-2、G-3、G-4、G-5、G-7
- Success Criteria: SC-2、SC-3、SC-4、SC-5、SC-6
- Requirements covered: FR-2、FR-4、FR-5、FR-6、FR-8、NFR-3、NFR-4

## Phase Discovery Gate

- [ ] 阅读 `desktop/ui/src/App.tsx` 的 `syncUserAudioSettings`、`getEffectiveAudioSource`、隐藏音频元素、`restartToNextLoop`、导入后自动准备和固定话术按钮事件。
- [ ] 阅读 `desktop/ui/src/voiceCloneStatus.ts` 与 `voiceCloneStatus.test.mjs`，确认现有禁用原因和状态测试可以扩展而不改变其他开关。
- [ ] 搜索所有 `voiceCloneAudioRef`、`voiceCloneAudioUrl`、`complete_playback_loop` 和 `onEnded` 处理，避免双重恢复或循环重复触发。
- [ ] 先用浏览器/播放器事件确认现有原音轨来自 video 元素还是独立 audio 元素，再确定静音与暂停的最小操作。

## Scope

### In Scope

- 当前文本按钮/动作只提交一条文案。
- 克隆片段播放期间临时静音原音轨。
- 克隆片段结束时恢复原音轨。
- 视频循环不重新播放当前文案；下一轮用户未再次点击时继续原音轨。

### Out of Scope

- 不增加文案播放队列。
- 不改变主窗口/最终效果窗口单实例结构。
- 不重做 Ant Design 布局或无关页面。

## Implementation Checklist

- [ ] 在 `desktop/ui/src/voiceCloneStatus.ts` 增加当前文案播放的禁用原因和状态显示：无源、未准备参考人声、当前文案未生成、正在生成、正在播放、已恢复原声、失败。
- [ ] 在 `desktop/ui/src/App.tsx` 将固定话术主按钮绑定到当前文本，调用 Phase 3 的单文案准备/播放命令；不要传递预制文案数组，不要自动遍历 10 条文案。
- [ ] 生成/播放开始前将 `voiceCloneAudio` 设置到当前缓存引用，先确认元数据可用，再将原视频/原音频静音，最后以视频当前时间和克隆片段偏移启动播放。
- [ ] 增加克隆音频 `onEnded` 处理：仅当 source generation 和当前 operation ID 仍匹配时，停止克隆音频、恢复原音轨并清理临时播放标记；正常 `loop_index` 变化不能使它失效。
- [ ] 修改循环边界处理：循环事件不得停止、重置或重新启动正在播放的克隆音频；如果克隆音频已经结束，则下一轮保持原音轨，只有用户再次点击才重新播放。
- [ ] 修改暂停、继续、拖动、停止和取消处理，确保克隆音频与视频同步，并且任何提前结束路径都恢复原音轨。
- [ ] 文案输入或 preset 切换时只失效当前 track；当前正在播放的旧片段不与新片段叠加，新文案在下一次点击生效。
- [ ] 增加按钮和提示文案，明确“当前文案只播放一次”“播放结束恢复原音轨”“循环不会自动播放文案”，避免显示“批量生成”或“播放全部”。
- [ ] 在 `desktop/ui/src/voiceCloneStatus.test.mjs` 增加当前文案唯一性、播放中禁用、结束恢复、循环不触发和文案变化失效测试。

## Validation Strategy

用 UI 纯逻辑测试证明状态和按钮条件，用现有播放器窗口做人工 smoke test 证明浏览器媒体事件、静音和克隆音频的实际时钟同步。不要只依赖 React snapshot 测试。

## Validation Checklist

- [ ] `pnpm test`
- [ ] `pnpm build`
- [ ] 人工导入视频，选择一条预制文案并确认只听到这一条克隆声音。
- [ ] 人工修改为手动文案，确认不会播放旧文案或其他预制文案。
- [ ] 人工验证克隆声音结束后原声恢复。
- [ ] 人工验证克隆音频结束后让视频循环两轮，确认两轮都只播放原音轨；再次点击后才重新播放当前文案。
- [ ] 人工验证暂停、继续、拖动、停止、取消和快速重复点击无叠音。

## Exit Criteria

- [ ] 克隆播放期间原音轨始终静音，结束后可靠恢复。
- [ ] 视频循环不触发任何文案，不出现文案队列；只有用户点击才播放当前文案。
- [ ] 原有视频处理、普通声音处理和实时音频开关仍可独立使用。
- [ ] UI 对 loading、failed、cancelled、stale 和恢复状态有明确反馈。

## Phase-End Multi-Pass Review

- [ ] 1. 点击、结束、循环不触发、跨循环、暂停、拖动和停止路径都覆盖。
- [ ] 2. 媒体事件竞态不会重复恢复或重复启动克隆音频。
- [ ] 3. UI 没有为了队列而增加无调用方状态。
- [ ] 4. 音轨切换逻辑集中在已有播放协调位置。
- [ ] 5. 清理未使用 state、ref、effect 依赖和按钮文案。
- [ ] 6. 文件引用和 asset URL 仍来自 Rust 校验后的路径。
- [ ] 7. 不使用任意长延时掩盖媒体竞态。
- [ ] 8. 自动测试和人工测试分别证明逻辑与真实媒体行为。
- [ ] 9. Phase 5 的打包和手工验收清单已更新。
- [ ] 10. 主 PRD 状态和变更记录已同步。
