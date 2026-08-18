# Phase 2: 运行时参数与通信

Parent PRD: [PRD: 桌面端主页与独立播放窗口体验修复](../prd-desktop-ui-corrections.md)
Status: Implemented; manual toggle timing pending
Last Updated: 2026-08-14

## Objective

把现有运行时参数调度接入主页，按独立开关周期更新受支持的预览值，并用受限的跨窗口消息把参数和诊断同步到独立播放器。

## Context From Master PRD

- Goals covered: G-4
- Success Criteria: SC-4, SC-6
- Requirements covered: FR-3, FR-4, FR-5, NFR-2, NFR-3
- Key scenarios touched: Scenario 2

## Phase Discovery Gate

Before editing code, re-check:

- [ ] `desktop/src-tauri/src/research_params.rs` 中 `random_change_period_ms` 的 500–60000ms 约束和默认 5000ms。
- [ ] `desktop/ui/src/App.tsx` 的 `ResearchParams` 类型与 Rust DTO 字段，尤其是 `pixel_scale_percent` 是否已进入前端模型。
- [ ] `desktop/ui/src/运行时参数自动调度.ts` 当前函数签名、范围裁剪和已有独立测试。
- [ ] Phase 1 是否已将主页作为可见状态所有者。
- [ ] Tauri WebView 对 `BroadcastChannel` 和 AudioContext 的能力探测/降级路径。

## Scope

### In Scope

- 周期、周期号、倒计时、基线参数和运行时预览参数的纯逻辑模型。
- 主页 → 播放窗口的运行时参数消息，播放窗口 → 主页的限量诊断采样消息。
- 当前已实现 Web Audio 增益/响度和视频 CSS 预览路径；未实现 DSP 保持不可用。

### Out of Scope

- 不把运行时参数写回 Rust 研究参数或覆盖源视频。
- 不把每次周期变化变成 FFmpeg 重编码任务，不生成离线版本。
- 不新增消息队列、全局 Store 或第三方跨窗口库。

## Implementation Checklist

- [ ] 在 `desktop/ui/src/运行时参数自动调度.ts` 导出与实际 `ResearchParams.video` 对齐的基线/预览类型，移除当前未接入的字段依赖；保留周期默认值、有限幅波形和边界裁剪。
- [ ] 在 `desktop/ui/src/运行时参数自动调度.test.mjs` 先补失败测试：周期 500/5000/60000 边界、周期未到、相同 cycle 稳定、音频/视频独立开关、范围裁剪和关闭后回到基线。
- [ ] 在 `desktop/ui/src/App.tsx` 让 `DesktopApp` 持有 scheduler ref/state；使用 100ms 以内的 UI 定时器显示倒计时，到期只递增一次 cycle 并生成新 preview 参数，卸载/严格模式重复执行时清理定时器。
- [ ] 在 `desktop/ui/src/App.tsx` 增加主页“配置值/运行时预览值/下一次变化”信息；用户编辑参数只更新基线，不能被运行时预览反向写入表单。
- [ ] 在 `desktop/ui/src/App.tsx` 创建 `BroadcastChannel('autolive-playback-ui-v1')`；主页只发送版本、运行时参数和两个开关，不发送完整媒体、完整音频或密钥；通道创建失败时记录可见但不阻塞播放的状态。
- [ ] 在 `FinalEffectWindow` 中接收并校验消息结构，应用受支持的视频滤镜/transform 和 Web Audio 增益；未支持参数不执行伪造效果，保留 Rust snapshot 的能力状态。
- [ ] 在播放窗口把波形/频谱采样裁剪到固定数量并按有限频率发送给主页；主页渲染采样，播放窗口没有接收方或通道关闭时停止发送但继续播放。
- [ ] 对 Channel、AudioContext、动画帧、定时器、媒体事件和对象 URL 补齐对称清理；旧消息不得覆盖新播放代际或已关闭模块。
- [ ] 在 `desktop/ui/package.json` 将现有 `test` 脚本纳入 `src/运行时参数自动调度.test.mjs`，保留 heartbeat 测试。

## Validation Strategy

先用 Node 纯逻辑测试证明调度边界，再用 TypeScript 构建证明消息类型和前端模型一致。最后启动 Tauri，开启/关闭音频和视频开关，观察主页周期与倒计时，并确认播放窗口仅应用预览效果且通信失败不阻塞视频。

## Validation Checklist

- [ ] `cd desktop/ui && node --test src/运行时参数自动调度.test.mjs` 通过。
- [ ] `cd desktop/ui && pnpm test` 同时通过 heartbeat 和调度测试。
- [ ] `cd desktop/ui && pnpm build` 通过。
- [ ] 开启视频处理后，只视频运行时值变化；关闭后停止变化并恢复基线。
- [ ] 开启声音处理后，只音频增益/响度运行时值变化；高级 DSP 未实现时明确显示不可用。
- [ ] 播放窗口关闭或 Channel 不可用时，视频仍能继续播放；主页显示诊断不可用而不是卡死。
- [ ] React StrictMode 下没有重复定时器、重复 Channel 或重复动画帧。

## Exit Criteria

- [ ] 主页能观察到周期、周期号、倒计时和受支持运行时参数。
- [ ] 两个独立开关互不影响，运行时变化不覆盖用户配置和源文件。
- [ ] 跨窗口通信消息有版本/形状校验、大小上限和清理路径。

## Phase-End Multi-Pass Review

- [ ] 1. 对照 G-4、SC-4、SC-6 和 FR-3/FR-5。
- [ ] 2. 复核周期边界、切换、暂停、停止和快速修改参数。
- [ ] 3. 复核消息乱序、过期代际和通信不可用。
- [ ] 4. 复核没有用 Effect 代替用户事件或复制服务端事实源。
- [ ] 5. 删除未使用消息类型、导入、类型和 debug 输出。
- [ ] 6. 确认消息没有 Secret、完整音视频或不必要的绝对路径。
- [ ] 7. 复核采样频率、数组长度和动画帧资源。
- [ ] 8. 复核测试脚本确实执行新增测试。
- [ ] 9. 更新 Phase 3 对播放代际和换源的假设。
- [ ] 10. 同步主 PRD 和阶段变更记录。

## Discoveries / Decisions

- 自动变化只作用于本地连续预览；FFmpeg 结果仍按用户点击应用参数后的既有链路处理。

## Phase Change Log

- 2026-08-14: Phase 2 创建。
- 2026-08-14: 完成稳定周期调度、主页倒计时、版本化 BroadcastChannel、播放器滤镜/增益应用和限量诊断采样；纯逻辑测试及构建通过。
