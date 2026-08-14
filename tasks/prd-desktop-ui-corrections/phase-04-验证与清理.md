# Phase 4: 验证与清理

Parent PRD: [PRD: 桌面端主页与独立播放窗口体验修复](../prd-desktop-ui-corrections.md)
Status: In Progress
Last Updated: 2026-08-14

## Objective

完成前端、Rust、窗口截图和短视频播放回归，清理本次范围产生或暴露的无效代码，形成可交付的风险与未验证项清单。

## Context From Master PRD

- Goals covered: G-1 through G-6
- Success Criteria: SC-1 through SC-7
- Requirements covered: all FR/NFR
- Key scenarios touched: all scenarios

## Phase Discovery Gate

Before editing code, re-check:

- [ ] Phase 1–3 的 phase 文件状态和实际 diff 已同步。
- [ ] `desktop/ui/package.json`、锁文件和 CI 没有被无关改动覆盖。
- [ ] `git status --short` 中的既有用户改动已记录，不对其执行回滚/清理。
- [ ] 当前平台可用的 Tauri、Node、pnpm、Rust 和媒体资源路径。

## Scope

### In Scope

- 全量静态、测试、构建、截图、窗口生命周期和短视频回归。
- 删除未使用导入/类型/常量、无效测试入口和调试日志。
- 更新 PRD/phase 文件的勾选项、证据、风险和未验证目标。

### Out of Scope

- 不顺手清理历史 `VariantTask`、旧 Worker 或无关后端代码。
- 不修改部署、服务端镜像和管理端页面，除非验证证明本次桌面端改动直接破坏它们。

## Implementation Checklist

- [ ] 在 `desktop/ui/package.json` 保证 `pnpm test` 执行 heartbeat、运行时参数和循环纯逻辑测试；锁文件只在依赖发生真实变化时更新。
- [ ] 用 `rg` 检查 `App.tsx`、`运行时参数自动调度.ts`、`播放循环.ts` 的未使用导入、类型、常量、ref、消息字段和调试日志，删除本次改动产生的无效项。
- [ ] 运行 `cd desktop/ui && pnpm test && pnpm build`。
- [ ] 运行 `cd desktop/src-tauri && cargo fmt --all -- --check && cargo clippy --all-targets --all-features -- -D warnings && cargo test --all-features`。
- [ ] 运行 `git diff --check`，确认没有空白错误；审查 diff 不包含用户未授权的服务器构建、worktree 或无关重构。
- [ ] 以真实窗口完成导入、主页查看、打开/聚焦播放器、开关切换、参数周期变化、暂停/继续/停止、至少两轮循环、关闭/重开播放器和通信不可用降级。
- [ ] 保存并检查主窗口/播放窗口最终截图；记录窗口尺寸、操作步骤、结果和未覆盖的平台。
- [ ] 更新主 PRD、context 和 phase 文件的 Status、Exit Criteria、Validation Checklist、Discoveries 和 Change Log。

## Validation Strategy

本阶段采用最小但完整的证据组合：纯逻辑测试覆盖调度与循环边界，TypeScript/Rust 门禁覆盖构建与类型，Tauri 手工冒烟覆盖窗口和媒体事件，截图覆盖可读性与职责分离。Windows、macOS Intel 或真实 Worker 不可用时如实记录，不把未执行写成通过。

## Validation Checklist

- [ ] `cd desktop/ui && pnpm test`：通过并明确列出测试数量。
- [ ] `cd desktop/ui && pnpm build`：通过或记录确切失败原因。
- [ ] `cd desktop/src-tauri && cargo fmt --all -- --check`：通过或记录失败。
- [ ] `cd desktop/src-tauri && cargo clippy --all-targets --all-features -- -D warnings`：通过或记录失败。
- [ ] `cd desktop/src-tauri && cargo test --all-features`：通过或记录失败。
- [ ] `git diff --check`：通过。
- [ ] 视觉截图与无障碍树分别确认主页可读、播放窗口只显示媒体。
- [ ] 手工回归记录成功、空态、播放失败、暂停、停止、关闭、重开和 IPC/Worker 不可用结果。

## Exit Criteria

- [ ] 所有成功标准已验证或有明确证据缺口。
- [ ] 本次范围没有未使用代码、重复事实源和无关依赖。
- [ ] 交付报告能列出改动文件、实际命令、结果、剩余风险和后续动作。

## Phase-End Multi-Pass Review

- [ ] 1. 逐项核对全部 Success Criteria。
- [ ] 2. 检查四个阶段的接口、状态和清理是否闭合。
- [ ] 3. 复核错误、空态、权限、取消、恢复和窗口生命周期。
- [ ] 4. 删除没有必要的抽象、消息字段、依赖和配置。
- [ ] 5. 检查重复逻辑、临时文件、调试输出和未使用资源。
- [ ] 6. 检查通信、路径、日志和本地媒体信息的隐私边界。
- [ ] 7. 检查轮询、采样、动画帧和组件重渲染成本。
- [ ] 8. 核对所有命令输出与 PRD 记录一致。
- [ ] 9. 记录跨平台、真实 Worker、系统缩放等未验证项。
- [ ] 10. 完成 PRD closeout review，决定后续执行或延期。

## Discoveries / Decisions

- 基线测试通过不代表本次体验已完成；新增测试入口、真实窗口截图和短视频循环是本阶段必需证据。

## Phase Change Log

- 2026-08-14: Phase 4 创建。
- 2026-08-14: 前端/Rust 自动化门禁、debug bundle 构建、主页/纯播放器截图完成；因桌面自动化会话锁定，导入短视频、至少两轮循环、开关周期和暂停/停止手工回归保留为下一步。
