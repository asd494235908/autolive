# PRD: 桌面端主页与独立播放窗口体验修复

## Document Status
- Status: In Progress
- File Mode: Split
- Current Phase: Phase 4 — 验证与清理
- Active Phase File: [Phase 1](./prd-desktop-ui-corrections/phase-01-主页与主题.md)
- Context File: [context.md](./prd-desktop-ui-corrections/context.md)
- Last Updated: 2026-08-14
- PRD File: `tasks/prd-desktop-ui-corrections.md`
- Purpose: 作为本次桌面端体验修复的唯一执行依据，后续实现过程中按事实更新。

## Problem

当前 Tauri 桌面端把大量状态、控制和诊断信息同时渲染在主窗口与独立播放窗口中。主窗口和播放窗口的主题没有统一，播放窗口在深色背景上使用了低对比度文字；自动参数调度与循环兜底已有部分代码和测试，但尚未接入实际播放流程。用户因此看不清页面、无法在主页集中管理状态，并且视频结束后可能停在结束帧。

## Goals

- G-1：主窗口成为唯一的信息与控制主页，展示素材、播放状态、三个独立开关、运行时参数、波形/频谱、Worker 和研究状态。
- G-2：独立窗口只承担视频播放和当前音频，不再展示状态卡片、参数表、Worker、研究面板或诊断图。
- G-3：主窗口和播放窗口使用同一套高对比度 Ant Design 主题，正文和状态文字在实际窗口截图中可读。
- G-4：在音频/视频对应开关开启时，按 `audio.random_change_period_ms`（默认 5000ms，范围 500–60000ms）更新受支持的本地预览参数，并在主页显示周期、当前值和倒计时。
- G-5：视频结束后先在本地立即从 0 秒继续播放，再异步同步 Rust 轮次；IPC 失败不能阻塞循环。
- G-6：始终只存在一个 label 为 `final-effect` 的播放窗口，重复打开只显示并聚焦现有窗口。

## Non-Goals

- NG-1：不新增 Go API、数据库表、媒体任务队列或服务端视频代理。
- NG-2：不生成 N 个离线 MP4，不恢复 RTMP/OBS、平台接入或自动发布。
- NG-3：不把尚未接入运行时 DSP 的 MFCC、SNR、共振峰、滤波 Q、相位等参数伪装成已生效；首轮实时声音预览只承诺已有增益/响度路径。
- NG-4：不创建第二个播放窗口，不用重建窗口实现换源。
- NG-5：不引入新的 UI 组件库或重复实现 Ant Design 已有控件。

## Success Criteria

- SC-1：在主窗口 1100×820 和播放窗口 1280×760 的截图中，标题、标签、值、按钮、错误和禁用状态均能清楚阅读；不再出现深色底黑字。
- SC-2：播放窗口可见内容只有视频和必要的原生播放控件；暂停、继续、停止、状态、轮次、参数、波形/频谱和 Worker 信息在主页可见。
- SC-3：导入一个受支持视频格式后主窗口能够显示当前源素材、播放状态、轮次和“打开/聚焦播放器”操作；重复操作不创建第二个 `final-effect` 窗口。
- SC-4：开启视频或声音处理后，主页按配置周期显示新一轮运行时预览值；关闭对应开关后该模块停止变化，另一模块不受影响。
- SC-5：视频结束或 `timeupdate` 进入末尾阈值时，同一窗口从 0 秒继续播放；`complete_playback_loop` 失败、慢或重复触发都不能让媒体停住或重复递增轮次。
- SC-6：播放窗口关闭后主窗口仍可工作，重新打开只恢复同一窗口 label；通信关闭、音频分析不可用或 Worker 不可用时，源视频仍可播放。
- SC-7：前端测试、TypeScript 构建、Rust 格式化/Clippy/测试和手工桌面冒烟均有实际命令结果，未执行项明确记录。

## Key Scenarios

### Scenario 1: 主窗口集中管理
- Actor: 普通桌面用户
- Trigger: 导入一个本地受支持视频格式
- Expected outcome: 主窗口展示素材信息、播放状态、轮次、三个开关、运行时参数和诊断信息；播放窗口只展示视频。

### Scenario 2: 周期参数预览
- Actor: 普通桌面用户
- Trigger: 打开视频处理或声音处理开关
- Expected outcome: 主页倒计时到期后更新对应模块的运行时预览值；另一模块保持原值；关闭开关后恢复基线并停止调度。

### Scenario 3: 单源连续循环
- Actor: 普通桌面用户
- Trigger: 视频播放到结束
- Expected outcome: 播放窗口不重建、不黑屏、不停住，同一源视频立即从 0 秒继续；主页轮次最终递增一次。

## Discovery Summary

- Reviewed: `产品需求文档.md`、`系统架构总览.md`、`桌面客户端架构.md`、`媒体参数范围与默认值.md`、`长任务开发总计划.md`、`开发约束/前端开发约束.md`、`开发约束/Rust开发约束.md`、现有主页/播放窗口设计文档、`desktop/ui/src/App.tsx`、`desktop/ui/src/main.tsx`、`desktop/ui/src/播放循环.ts`、`desktop/ui/src/运行时参数自动调度.ts`、`desktop/src-tauri/src/commands.rs`、`desktop/src-tauri/src/lib.rs`、Tauri 配置和现有测试。
- Current system: React/Tauri 使用固定 `final-effect` label 创建独立播放窗口；Rust `PlaybackCore` 是播放状态事实源；主页和播放窗口各自轮询 `get_snapshot`；当前 `FinalEffectWindow` 仍渲染状态、控制、描述信息和波形/频谱。
- Evidence: [主窗口截图](./prd-desktop-ui-corrections/evidence/01-main-home.jpeg)、[播放窗口截图](./prd-desktop-ui-corrections/evidence/02-player-window.jpeg)、[取证上下文](./prd-desktop-ui-corrections/context.md)。
- Validation surface: `pnpm test` 已纳入 heartbeat、运行时参数和播放循环测试并通过 12 项；`pnpm build`、`cargo fmt -- --check`、`cargo clippy --all-targets --all-features -- -D warnings`、`cargo test --all-features` 和本地 debug bundle 构建均通过。已用打包桌面应用确认主页信息集中、文字可读和独立窗口仅保留媒体；短视频导入、至少两轮循环、开关周期变化和暂停/停止的完整手工回归因桌面自动化会话锁定暂未完成。
- Design implications: 主页必须成为唯一信息所有者；运行时参数由主页调度，播放窗口只接收受限预览消息；本地媒体播放不能等待 IPC；现有 Ant Design 主题配置需要用真实截图验证并补齐 Descriptions 的公开 token。
- Confidence / gaps: 已确认主窗口和播放窗口的视觉问题；尚未让一个 6 秒测试文件完整走完导入流程并实测结束事件，循环方案仍需在实现阶段做短视频手工冒烟。

## Requirements

### Functional Requirements

- FR-1：主页显示播放状态、轮次、暂停/继续/停止和打开/聚焦播放器；这些功能不再依赖播放窗口中的自定义状态按钮。
- FR-2：播放窗口只渲染一个视频元素、一个隐藏候选音频元素和必要的原生播放控件；卸载时释放定时器、音频上下文、对象 URL 和通信通道。
- FR-3：主页接收播放窗口的限量波形/频谱采样并展示；采样通信失败不影响播放。
- FR-4：运行时参数调度必须读取并校验周期，默认为 5000ms；周期未到不更新，周期到达只更新已开启模块，并且生成值稳定、有限幅、可测试。
- FR-5：支持的运行时效果必须实际作用于当前预览：声音为现有 Web Audio 增益/响度路径，视频为预览层亮度、对比度、饱和度、色相、模糊、缩放/空间偏移等已实现路径；其他参数显示“仅配置/研究”状态。
- FR-6：循环逻辑必须同时处理 `ended` 和接近 duration 的 `timeupdate`，同一结束事件只能重启一次；新源、停止、暂停和处理后换源必须重新布置门禁。
- FR-7：打开播放器命令继续使用固定 `final-effect` label；已有窗口只执行 show/focus，不创建新窗口。

### Non-Functional Requirements

- NFR-1：React 继续使用 Ant Design；禁止覆盖 `.ant-*` 选择器，主题只使用 `ConfigProvider` 公开 token、组件 Props 和页面布局。
- NFR-2：所有定时器、`requestAnimationFrame`、`BroadcastChannel`、AudioContext 和媒体事件监听都必须有对称清理，并兼容 React StrictMode 重复挂载。
- NFR-3：播放窗口与主页之间只传输有限诊断采样和运行时预览参数，不传输模型密钥、完整视频或完整音频文件。
- NFR-4：保留当前 Rust 播放状态、权限边界和本地媒体链路；除非循环契约证明需要，否则不扩展后端和数据库。
- NFR-5：实现过程中删除本次改动暴露的未使用导入、类型、常量、测试入口和调试日志，但不删除工作区中与本任务无关的用户改动。

## Assumptions

- A-1：首轮“每几秒自动变换”指本地连续预览的运行时参数变化，不代表每几秒重新编码或生成新的 MP4。
- A-2：实时音频幻化仍受现有 Worker/租约能力限制；没有 Worker 时保持原音轨并显示不可用原因。
- A-3：`BroadcastChannel` 在当前 Tauri WebView 目标平台可用；不可用时降级为“只保留播放，不显示实时诊断/预览参数”。
- A-4：当前已存在的未提交改动属于用户工作，不在本 PRD 中覆盖或回滚。

## Dependencies / Constraints

- 使用现有 `antd`、React、Tauri API 和 Rust `PlaybackCore`，不新增依赖。
- 参数范围和默认值以 `媒体参数范围与默认值.md`、`desktop/src-tauri/src/research_params.rs` 为准。
- 构建在本地或 CI 完成，不把源码放到服务器构建；不使用 Git worktree。
- 任何新处理参数必须通过既有 Rust 校验和能力状态，不得仅在前端声称生效。

## Risks / Edge Cases

- 播放窗口打开时主页刷新或主页关闭：播放器仍不能变成第二个状态事实源，且必须能安全继续/停止。
- `ended` 与 `timeupdate` 同时到达：必须由结束事件 token 或等价门禁去重，不能只用播放代际，因为同一源会有多个轮次。
- `complete_playback_loop` 返回慢/失败：视频先本地重启，状态同步失败只影响轮次显示和下一次同步，不影响媒体播放。
- 处理后视频引用变化：换源时等待媒体加载到可播放状态，再清理旧候选音频并重置循环门禁。
- 用户快速开关或修改参数：旧定时器和旧消息不得覆盖新状态；关闭模块后应立即停止对应运行时效果。
- 窗口缩放、系统字号和长文件名：主页必须在最小窗口和文本扩展下保持可读，播放窗口视频不能被状态面板挤压。

## Execution Rules

- 按 Phase 1 → Phase 4 顺序执行；阶段发现改变后先更新本 PRD 和对应 phase 文件。
- 先写能失败的纯逻辑测试，再实现运行时行为；视觉问题必须通过实际窗口截图复核。
- 使用本 PRD 和 phase 文件作为唯一活动计划，不另建相互冲突的清单。
- 每个阶段结束执行该阶段的多轮复核，重点检查循环竞态、通信清理、对比度、未使用代码和能力边界。

## Phase Index

| Phase | Status | Objective | Validation Focus | File |
|---|---|---|---|---|
| Phase 1: 主页与主题 | Implemented | 主页集中承载信息和控制，两个窗口统一可读主题 | 打包应用无障碍树/截图、类型检查 | [phase-01-主页与主题.md](./prd-desktop-ui-corrections/phase-01-主页与主题.md) |
| Phase 2: 运行时参数与通信 | Implemented | 接入周期参数调度、预览效果和诊断通信 | 纯逻辑测试、构建；手工周期切换待补 | [phase-02-运行时参数与通信.md](./prd-desktop-ui-corrections/phase-02-运行时参数与通信.md) |
| Phase 3: 单窗口循环 | Implemented | 修复同一源视频的连续循环和单窗口聚焦 | 循环单测、Rust 测试；短视频手工冒烟待补 | [phase-03-单窗口循环.md](./prd-desktop-ui-corrections/phase-03-单窗口循环.md) |
| Phase 4: 验证与清理 | In Progress | 完成全量检查、删除无效代码并交付风险清单 | 前端/Rust 门禁、截图已完成，媒体回归待补 | [phase-04-验证与清理.md](./prd-desktop-ui-corrections/phase-04-验证与清理.md) |

## Final Multi-Pass Review After All Phases

- [ ] 1. Requirements coverage review: FR、NFR 和 SC 均已满足或有明确延期记录。
- [ ] 2. Cross-phase integration review: 主页状态、播放窗口媒体和 Rust 状态没有重复事实源。
- [ ] 3. Correctness review: 成功、空态、错误、暂停、停止、取消、换源和重复事件均有路径。
- [ ] 4. Simplicity/refactor review: 没有为了跨窗口通信引入多余 Store、第三方库或大范围重构。
- [ ] 5. Duplication/cleanup review: 未使用导入、类型、常量、测试和调试输出已删除。
- [ ] 6. Security/privacy review: 通信不携带 Secret、完整媒体或不必要的本地路径信息。
- [ ] 7. Performance/load review: 轮询、定时器、动画帧和采样消息有频率与数量上限。
- [ ] 8. Validation review: 自动化、截图和手工播放验证组合足以覆盖本次风险。
- [ ] 9. Documentation/operability review: 参数能力边界、回退语义和验收步骤已同步。
- [ ] 10. PRD closeout review: 状态、变更记录和遗留风险已更新。

## Open Questions

- 当前没有阻塞实现的问题；若产品希望“自动变换”包含未接入的高级音频 DSP 或离线重编码，需要另开范围评审，不能在本任务中默认为已支持。

## Change Log

- 2026-08-14: 基于源码、现有设计文档、桌面端运行态和测试结果创建初版 PRD。
- 2026-08-14: 完成主页/播放器职责拆分、主题统一、运行时参数通信和本地优先循环接线；补充自动化验证结果与手工媒体回归缺口。
