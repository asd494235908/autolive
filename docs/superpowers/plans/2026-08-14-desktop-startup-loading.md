# 桌面端启动 Loading 与首屏白屏优化 Implementation Plan

> 当前版本范围声明（2026-08-19）：本版本不开发实时话术幻化。本文中的 speech-to-speech 能力探测属于历史/兼容代码的延后探测，不得作为当前版本功能、入口或验收项；当前启动流程不得准备或启动实时话术链路。

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 Tauri 开发版和生产包在 React 加载前显示启动反馈，并通过入口懒加载、错误恢复和延后非必要能力探测减少首屏白屏时间。

**Architecture:** `index.html`提供不依赖运行时代码的静态启动壳；`main.tsx`只加载 React、启动反馈组件和懒加载 App；`App.tsx`拥有 Ant Design 外壳与业务状态。首屏快照保持立即读取，研究/媒体/实时 Worker 的能力探测在首屏提交后通过可取消的空闲调度执行。

**Tech Stack:** React 18、TypeScript 5、Ant Design 5、Vite 5、Tauri 2、Node.js 内置 `node:test`。

## Global Constraints

- 直接在当前分支和工作区实施，不创建 Git worktree。
- 保留当前所有与语音克隆、Worker、模型和文档相关的未提交改动，不回滚、不覆盖、不批量格式化无关文件。
- 不新增运行时依赖，不把代码放到服务器构建；构建在本地执行。
- React 启动反馈不依赖 Ant Design、Tauri IPC、Worker 或网络。
- 只延后首屏非必要能力探测；`get_snapshot`、导入视频、播放和固定话术自动准备语义保持不变。
- 每个新行为先写失败测试，确认失败后再写生产代码；交付前删除本次产生的未使用导入、类型、导出和样式。

## 文件结构

- Create: `desktop/ui/src/startup-loader.tsx` — 轻量 Loading 和顶层 Error Boundary。
- Create: `desktop/ui/src/startup-loader.test.mjs` — Loading/Error Boundary 的源契约和行为测试。
- Create: `desktop/ui/src/startup-scheduler.ts` — 可取消的首屏后空闲调度封装。
- Create: `desktop/ui/src/startup-scheduler.test.mjs` — 空闲 API 和定时器回退契约测试。
- Modify: `desktop/ui/index.html` — 静态启动壳及首屏样式。
- Modify: `desktop/ui/src/main.tsx` — App 懒加载、Suspense 和 Error Boundary。
- Modify: `desktop/ui/src/App.tsx` — 移动 Ant Design 根外壳，并将非必要能力探测改为空闲调度。
- Modify: `desktop/ui/package.json` — 纳入新增 Node 测试文件。

### Task 1: 启动反馈组件

**Files:**
- Create: `desktop/ui/src/startup-loader.tsx`
- Test: `desktop/ui/src/startup-loader.test.mjs`

- [ ] 先写测试：断言 Loading 默认文案、错误边界错误提示和重载按钮契约。
- [ ] 运行 `pnpm exec node --test src/startup-loader.test.mjs`，确认在组件不存在或导出缺失时失败。
- [ ] 实现无 Ant Design 依赖的 `StartupLoading` 与 `StartupErrorBoundary`。
- [ ] 重跑聚焦测试，确认通过。

### Task 2: 静态启动壳与 React 入口

**Files:**
- Modify: `desktop/ui/index.html`
- Modify: `desktop/ui/src/main.tsx`
- Create: `desktop/ui/src/startup-entry.test.mjs`

- [ ] 先写 HTML/入口源码契约测试，锁定静态壳、`lazy(() => import('./App'))`、`Suspense` 和 Error Boundary。
- [ ] 运行 `pnpm exec node --test src/startup-entry.test.mjs`，确认当前空壳和同步入口使测试失败。
- [ ] 把静态壳加入 `#root`，保持文案、对比度和 spinner 不依赖外部 CSS。
- [ ] 让入口只同步加载轻量启动组件，懒加载 App，并用 Suspense/Error Boundary 包裹。
- [ ] 将 `ConfigProvider`/`AntApp`移动到 `App.tsx`默认导出内部，删除入口中的未使用 Ant Design 导入。
- [ ] 重跑聚焦测试和 `pnpm build`，确认生成生产 dist。

### Task 3: 首屏后空闲调度

**Files:**
- Create: `desktop/ui/src/startup-scheduler.ts`
- Test: `desktop/ui/src/startup-scheduler.test.mjs`
- Modify: `desktop/ui/src/App.tsx`

- [ ] 先写测试：`requestIdleCallback`存在时使用它并返回取消函数；不存在时使用可清理的 `setTimeout`回退。
- [ ] 运行聚焦测试，确认调度模块缺失时失败。
- [ ] 实现小型调度函数，不保存全局状态，不改变业务 IPC 返回值。
- [ ] 将媒体引擎能力探测放入可取消的首屏后调度；研究 Worker/状态/默认参数和 speech-to-speech 能力属于历史/后续版本，本版本不调度、不探测，研究状态轮询不进入当前页面。
- [ ] 保留 `get_snapshot` 的立即轮询和固定话术按需探测；检查 `prepareVoiceCloneAfterImport` 的现有调用不被延后调度误伤。
- [ ] 运行聚焦调度测试和前端全量测试。

### Task 4: 统一验证与主线程代码审查

**Files:**
- Inspect all changed files; modify only if verification发现本轮引入的错误或未使用代码。

- [ ] 检查 `git diff` 和 `git status`，确认没有覆盖用户未提交改动。
- [ ] 执行 `cd desktop/ui && pnpm test`。
- [ ] 执行 `cd desktop/ui && pnpm build`，检查 `dist/index.html`仍包含静态壳且存在独立 App chunk。
- [ ] 执行 `cd desktop/src-tauri && cargo fmt --all -- --check && cargo clippy --all-targets --all-features -- -D warnings && cargo test --workspace --all-features`。
- [ ] 本地启动 `pnpm tauri:dev` 做桌面启动冒烟；记录当前 macOS 环境无法替代 Windows 原生 GUI 验证的风险。
- [ ] 以代码总监视角复核：首屏壳无 IPC 依赖、错误可恢复、调度清理对称、现有语音克隆逻辑未被破坏、无未使用代码。

