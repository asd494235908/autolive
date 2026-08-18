# 桌面端主页与独立播放窗口体验修复：取证上下文

Parent PRD: [PRD](../prd-desktop-ui-corrections.md)
Last Updated: 2026-08-14

## Reviewed Inputs

- 产品和架构：`产品需求文档.md`、`系统架构总览.md`、`桌面客户端架构.md`、`媒体参数范围与默认值.md`、`长任务开发总计划.md`。
- 约束：`开发约束/前端开发约束.md`、`开发约束/Rust开发约束.md`、仓库根目录 `AGENTS.md`。
- 现有设计：`docs/superpowers/specs/2026-08-14-桌面端主页与独立播放器窗口设计.md`、`docs/superpowers/plans/2026-08-14-桌面端主页与独立播放器窗口实施计划.md`。
- 前端：`desktop/ui/src/App.tsx`、`desktop/ui/src/main.tsx`、`desktop/ui/src/播放循环.ts`、`desktop/ui/src/运行时参数自动调度.ts`、`desktop/ui/src/运行时参数自动调度.test.mjs`、`desktop/ui/package.json`。
- Rust/Tauri：`desktop/src-tauri/src/commands.rs`、`desktop/src-tauri/src/lib.rs`、`desktop/src-tauri/src/main.rs`、`desktop/src-tauri/tauri.conf.json`、`desktop/src-tauri/capabilities/default.json`。
- Evidence screenshots: `evidence/01-main-home.jpeg`、`evidence/02-player-window.jpeg`。

## Current Behavior

1. `App.tsx` 通过 URL 查询参数在 `DesktopApp` 和 `FinalEffectWindow` 间分支。
2. `DesktopApp` 当前有导入、素材信息、三个开关、研究参数、Worker/研究状态和缓存清理，但没有完整的播放状态/轮次/暂停/继续/停止/诊断主页面板。
3. `FinalEffectWindow` 当前包含标题、轮次、Descriptions 状态、暂停/继续/停止按钮、视频、隐藏音频和实时波形/频谱卡片。
4. `commands.rs:1332-1364` 已使用固定 `final-effect` label；已有窗口执行 `show` 和 `set_focus`，创建逻辑本身已具备单窗口入口。
5. `PlaybackCore::complete_loop` 已递增轮次、保持播放状态、清理候选音频并提交待处理视频；问题主要在前端结束事件先等待 IPC，且未接入接近结束兜底。
6. `运行时参数自动调度.ts` 和 `播放循环.ts` 已有纯逻辑函数，但 `App.tsx` 中当前只导入未调用；`PLAYBACK_CHANNEL_NAME`、消息类型和运行时类型也没有实际通信链路。
7. `desktop/ui/package.json` 的 `test` 当前只执行 `src/heartbeatOutbox.test.mjs`，不会执行 `运行时参数自动调度.test.mjs`。

## Runtime Evidence

- 主窗口截图呈浅色页面，卡片/研究参数连续纵向堆叠；当前可见内容没有运行时参数周期、倒计时和主页播放控制。[主窗口](./evidence/01-main-home.jpeg)
- 播放窗口截图显示深色背景上的黑色状态文字，并同时显示“当前音轨、幻化 Worker、音频决策、回退原因、声音处理、视频处理、暂停/继续/停止、实时音频诊断”。[播放窗口](./evidence/02-player-window.jpeg)
- 运行态无障碍树确认窗口标题为 `autoLive 最终效果`，URL 为 `index.html?view=final-effect`，视频正在播放，播放器 label 没有第二个实例证据。

## Baseline Commands

- `cd desktop/ui && pnpm test`：通过，3 个 heartbeat 测试；未覆盖运行时调度测试。
- `cd desktop/ui && node --test src/运行时参数自动调度.test.mjs`：通过，3 个测试。
- `cd desktop/ui && pnpm build`：通过；Vite 报告 bundle 超过 500 kB 的已有警告。
- `cd desktop/src-tauri && cargo test --all-features`：通过现有 Rust 单元/集成测试。
- `git diff --check`：通过。

## Verified Gaps

- 主题配置虽已在 `main.tsx` 中尝试加入 dark algorithm，但实际截图仍表现为浅色主窗口；播放窗口的 `Descriptions` 使用了页面级 `style.color`，没有覆盖内部 token，产生深色底黑字。
- `FinalEffectWindow` 的可见状态与诊断没有移回主页，违反窗口职责设计。
- 自动参数函数没有调用方，因而不会每 5 秒产生或应用新的运行时预览参数。
- 循环辅助函数没有调用方；当前 `onEnded` 先调用 `complete_playback_loop`，只在 Promise 成功后重置并 `play()`，IPC 失败会停在结束状态；没有 `timeupdate` 兜底。
- 当前循环辅助函数以 `mediaGeneration === lastRestartGeneration` 去重，若直接把播放代际当作去重键，同一个源的第二轮会被误判为已处理；实现阶段必须使用每次结束事件 token 或等价语义。

## Validation Limits

- 本轮运行态已成功打开一个已有本地受支持格式视频（当时使用 MP4）的独立窗口并取得截图；没有等待其完整 72.3 秒自然结束，也没有完成一个 6 秒临时视频的导入选择流程，因此循环修复仍需实现后用受支持格式短视频手工验证。
- 未在 Windows 或 macOS Intel 上运行桌面窗口；跨平台媒体资源已有 Rust/脚本测试，但窗口主题和媒体事件仍需至少在目标开发机复核。
- 截图不能证明键盘焦点、读屏语义、系统缩放和实际 AudioContext 资源释放，必须在阶段验证中补做适用检查。
