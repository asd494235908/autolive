# Phase 1: 主页与主题

Parent PRD: [PRD: 桌面端主页与独立播放窗口体验修复](../prd-desktop-ui-corrections.md)
Status: Implemented; media interaction regression pending
Last Updated: 2026-08-14

## Objective

让主页成为桌面端唯一的信息与控制中心，并让主页、独立播放窗口和状态文字使用一致且可读的主题。

## Context From Master PRD

- Goals covered: G-1, G-2, G-3
- Success Criteria: SC-1, SC-2, SC-3
- Requirements covered: FR-1, FR-2, NFR-1
- Key scenarios touched: Scenario 1

## Phase Discovery Gate

Before editing code, re-check:

- [ ] `desktop/ui/src/App.tsx` 的 `DesktopApp`、`FinalEffectWindow` 和 `App` 分支仍与上下文一致。
- [ ] `desktop/ui/src/main.tsx` 的 Ant Design token 和现有工作区改动，避免覆盖用户修改。
- [ ] `desktop/src-tauri/src/commands.rs:1332-1364` 的固定窗口 label 和 show/focus 语义未改变。
- [ ] `媒体参数范围与默认值.md` 的窗口、循环和可读性口径未发生冲突。
- [ ] 现有截图仍能复现浅色主窗口和深色播放窗口低对比度文字。

## Scope

### In Scope

- 主窗口显示源素材、播放状态/轮次、暂停/继续/停止、打开/聚焦播放器、三个独立开关、参数与能力状态。
- 播放窗口只显示视频和必要的原生播放控件；移除可见状态 Descriptions、自定义播放按钮、波形/频谱和 Worker 信息。
- 统一 Ant Design 主题、页面背景、Card、Descriptions label/content、标题、状态 Tag、按钮和禁用状态。

### Out of Scope

- 不改变 Rust 播放状态模型、不新增后台接口、不重做研究参数算法。
- 不用自定义按钮、表单或 `.ant-*` CSS 替代 Ant Design 组件。

## Implementation Checklist

- [ ] 在 `desktop/ui/src/main.tsx` 整理唯一主题配置，使用 `theme.darkAlgorithm` 和已有 token；补齐 Descriptions 的 `labelColor`/`contentColor` 等公开组件 token，确认 `Typography`、Card、Alert、Tag、InputNumber、Switch、Select 的文字、边框和禁用态对比度。
- [ ] 在 `desktop/ui/src/App.tsx` 将主页内容按“素材与播放”“处理开关”“运行时预览”“实时诊断”“本地研究”分组；把播放 snapshot 的状态、轮次和控制动作归属 `DesktopApp`。
- [ ] 在 `desktop/ui/src/App.tsx` 删除 `FinalEffectWindow` 可见的状态 Descriptions、Worker 信息、暂停/继续/停止自定义按钮和实时诊断 Card，只保留 `<video>`、隐藏 `<audio>` 和必要的原生 controls/错误空态。
- [ ] 在 `desktop/ui/src/App.tsx` 增加“打开/聚焦播放器”主页按钮，复用 `open_final_effect_window`；导入成功后仍自动打开，但重复点击只聚焦已有窗口。
- [ ] 在 `desktop/ui/src/App.tsx` 为主页的 `get_snapshot` 轮询增加 loading、无源素材、播放中、暂停、停止和错误可读状态；不要在两个窗口维护可见的重复事实源。
- [ ] 若当前单文件继续超过可审查范围，只把真实边界抽成 `PlaybackStatusPanel`、`RuntimePreviewPanel`、`DiagnosticsPanel` 等直接调用组件；不创建无调用方的 `utils/common/shared`。

## Validation Strategy

本阶段风险主要是视觉层级、窗口职责和状态归属。使用 `pnpm build` 做静态/类型验证，使用 Tauri 开发窗口在 1100×820 与 1280×760 截图检查主题和布局，并用无障碍树确认播放窗口不再包含状态卡片和诊断信息。

## Validation Checklist

- [ ] `cd desktop/ui && pnpm build` 通过。
- [ ] 主窗口截图确认正文、标签、状态值、错误、禁用控件可读，且长页面在最小尺寸可滚动。
- [ ] 播放窗口截图确认只剩视频和必要原生控件，没有 `当前音轨`、`Worker`、`音频决策`、`实时音频诊断` 等主页信息。
- [ ] 关闭播放窗口后主页仍可读取 snapshot；重新点击“打开/聚焦播放器”不产生第二个 `final-effect` 窗口。
- [ ] 键盘 Tab 顺序覆盖导入、播放控制、开关和参数；图标按钮有可访问名称。

## Exit Criteria

- [ ] 主页是所有信息和控制的唯一可见承载面。
- [ ] 独立窗口只承担媒体播放，主题与主页一致且截图无低对比度文字。
- [ ] 本阶段未引入新的 UI 依赖、无用抽象或未清理导入。

## Phase-End Multi-Pass Review

- [ ] 1. 逐项对照 G-1/G-2/G-3、SC-1/SC-2/SC-3。
- [ ] 2. 复核播放窗口关闭、空素材、错误和禁用状态。
- [ ] 3. 复核主题只用公开 token/Props，没有 `.ant-*` 覆盖。
- [ ] 4. 复核组件职责和状态所有权，没有主页/播放窗口双写。
- [ ] 5. 删除未使用导入、类型、常量和调试日志。
- [ ] 6. 检查敏感路径和媒体信息没有被新增到通信消息或日志。
- [ ] 7. 检查大卡片、视频尺寸和滚动性能。
- [ ] 8. 复核截图和无障碍树证据是否足够。
- [ ] 9. 根据发现更新 Phase 2 的通信边界。
- [ ] 10. 更新主 PRD 的状态、风险和变更记录。

## Discoveries / Decisions

- 当前主窗口和播放窗口的主题表现与代码配置不一致，因此必须以实际截图作为阶段退出条件。

## Phase Change Log

- 2026-08-14: Phase 1 创建。
- 2026-08-14: 主页承载状态、控制、运行时预览和诊断；独立窗口收敛为媒体播放页；debug bundle 截图确认可读性和职责分离。
