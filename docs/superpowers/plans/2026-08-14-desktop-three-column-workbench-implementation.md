# Desktop Three-Column Workbench Implementation Plan

> 当前版本范围声明（2026-08-19）：本版本不开发实时话术幻化。本计划只调整当前桌面页面布局和主题；不新增实时话术设置、speech-to-speech Worker 状态、候选音轨或模型租约入口。

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 将桌面端主页面改为 28% / 38% / 34% 的三列工作台，并恢复 Ant Design 默认亮色主题，同时保持现有媒体行为不变。

**Architecture:** `DesktopApp` 继续持有现有 Tauri 状态和事件处理，只调整 JSX 的三列职责分区；页面布局使用一个专用 CSS 文件表达精确比例、列内滚动和窄屏堆叠。`ConfigProvider` 删除自定义 Token，回到 Ant Design 默认主题。

**Tech Stack:** React 18、TypeScript、Ant Design 5、Vite、Tauri 2、Node.js 内置 `node:test`。

## Global Constraints

- 宽屏列比例固定为 28% / 38% / 34%，左列素材与视频状态，中列声音设置，右列视频处理与实时参数；主页不展示实际视频画面。
- 使用 Ant Design 默认亮色主题，移除当前自定义深色 Token 和自定义青色主色。
- 不修改 Rust/Tauri 命令、播放状态机、媒体处理流程、API、数据库或媒体任务模型。
- 不新增组件库、状态管理库或测试框架；继续复用现有 Ant Design 组件。
- 不覆盖 `.ant-*` 内部选择器，不把服务端状态复制到新的 Store，不引入未授权的持久化。
- 三个处理开关保持独立；导入成功后保持 `Ready`，用户点击“播放”才在同一窗口开始单源循环播放。
- 直接在当前分支和工作区开发，不使用 Git worktree，不把代码放到服务器构建。
- 交付前删除本次改动产生的未使用导入、类型、样式、依赖、调试日志和重复实现。

---

### Task 1: 建立三列布局契约测试

**Files:**
- Create: `desktop/ui/src/desktop-three-column-layout.test.mjs`
- Modify: `desktop/ui/package.json`

**Interfaces:**
- Test reads `desktop/ui/src/App.tsx` and `desktop/ui/src/desktop-layout.css` as text; it does not add runtime code or a new test dependency.
- Later layout work must expose the class names and CSS contract asserted below.

- [ ] **Step 1: Write the failing test**

Create `desktop/ui/src/desktop-three-column-layout.test.mjs`:

```js
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

const appPath = new URL('./App.tsx', import.meta.url);
const layoutPath = new URL('./desktop-layout.css', import.meta.url);

test('desktop page exposes the approved three-column layout contract', async () => {
  const app = await readFile(appPath, 'utf8');
  let css = '';
  try {
    css = await readFile(layoutPath, 'utf8');
  } catch {
    // Keep the failure an assertion failure until the layout stylesheet exists.
  }

  assert.match(app, /desktop-workspace/);
  assert.match(app, /desktop-column-source/);
  assert.match(app, /desktop-column-audio/);
  assert.match(app, /desktop-column-video/);
  assert.match(css, /grid-template-columns:\s*minmax\(0,\s*28fr\)\s+minmax\(0,\s*38fr\)\s+minmax\(0,\s*34fr\)/);
  assert.match(css, /@media\s*\(max-width:\s*800px\)/);
  assert.match(css, /grid-template-columns:\s*1fr/);
});
```

- [ ] **Step 2: Run the focused test and verify it fails for the missing layout contract**

Run:

```bash
cd desktop/ui && node --test src/desktop-three-column-layout.test.mjs
```

Expected: one assertion failure because the current `App.tsx` has no three-column markers and the stylesheet does not exist.

- [ ] **Step 3: Register the test in the existing test script**

In `desktop/ui/package.json`, append `src/desktop-three-column-layout.test.mjs` to the existing `node --test` command without changing the other test files or adding a dependency.

- [ ] **Step 4: Commit the red test and test-script registration**

```bash
git add desktop/ui/src/desktop-three-column-layout.test.mjs desktop/ui/package.json
git commit -m "test: define desktop three-column layout contract"
```

---

### Task 2: Implement the three-column desktop workbench

**Files:**
- Create: `desktop/ui/src/desktop-layout.css`
- Modify: `desktop/ui/src/App.tsx`
- Test: `desktop/ui/src/desktop-three-column-layout.test.mjs`

**Interfaces:**
- `DesktopApp` continues to use the existing state variables and callbacks; no new state owner or Tauri command is introduced.
- `desktop-layout.css` exports no symbols; it is imported for side effects by `App.tsx`.

- [ ] **Step 1: Add the minimum layout stylesheet**

Create `desktop/ui/src/desktop-layout.css` with only page-owned selectors:

```css
.desktop-page {
  min-height: 100vh;
}

.desktop-page-content {
  width: 100%;
  max-width: 1440px;
  margin: 0 auto;
  padding: 24px;
  box-sizing: border-box;
}

.desktop-workspace {
  display: grid;
  grid-template-columns: minmax(0, 28fr) minmax(0, 38fr) minmax(0, 34fr);
  gap: 16px;
  align-items: start;
}

.desktop-column {
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 16px;
}

@media (max-width: 1100px) {
  .desktop-page-content {
    padding: 16px;
  }

  .desktop-workspace {
    gap: 12px;
  }

  .desktop-column {
    max-height: calc(100vh - 32px);
    overflow-y: auto;
    padding-right: 4px;
  }
}

@media (max-width: 800px) {
  .desktop-workspace {
    grid-template-columns: 1fr;
  }

  .desktop-column {
    max-height: none;
    overflow: visible;
    padding-right: 0;
  }
}
```

The preview surface does not introduce a color token; all interactive colors come from Ant Design defaults.

- [ ] **Step 2: Import the stylesheet and replace the single vertical content wrapper**

At the top of `desktop/ui/src/App.tsx`, add:

```ts
import './desktop-layout.css';
```

In `DesktopApp` replace the current `Layout`/`Layout.Content` sizing and outer vertical `Space` with:

```tsx
<Layout className="desktop-page">
  <Layout.Content className="desktop-page-content">
    {/* existing hidden media element for Picture-in-Picture synchronization */}
    <div className="desktop-workspace">
      <section className="desktop-column desktop-column-source" aria-label="视频素材与状态">
        {/* title, single-source alert, import/open buttons, error, playback status, source media */}
      </section>
      <section className="desktop-column desktop-column-audio" aria-label="音频设置">
        {/* audio/realtime switches, voice clone, runtime audio values, diagnostics, audio research fields */}
      </section>
      <section className="desktop-column desktop-column-video" aria-label="视频处理与实时参数">
        {/* video switch/engine/apply state, runtime video values, video research fields */}
      </section>
    </div>
  </Layout.Content>
</Layout>
```

Keep the existing Ant Design `Card`, `Alert`, `Descriptions`, `Space`, `Button`, `Switch`, `Slider`, `InputNumber`, `Select`, `Progress` and `Tag` components inside their new sections; do not replace them with custom controls.

- [ ] **Step 3: Place the current content by responsibility**

Move existing JSX blocks without changing their handlers or labels:

1. Source column: page title, single-source alert, import/open actions, page error, “播放状态与控制”, and “当前源素材”.
2. Audio column: audio switch and realtime-audio switch from “处理开关”; audio-side worker alerts; fixed speech/interlude controls; audio-only runtime values; “实时诊断”; current ordinary audio fields only.
3. Video column: video switch and media-engine/apply controls from “处理开关”; video-only runtime values; current mapped video fields only. Do not add a “本地研究分析 Worker” card. The homepage does not show a video frame.

The combined `applyMediaProcessing()` action remains in the video column but keeps its existing guard, request order, loading state, and error handling. The research Worker card remains one card and keeps its existing commands; only its position changes.

- [ ] **Step 4: Keep the homepage media element hidden and preserve Picture-in-Picture synchronization**

Keep the existing `pictureInPictureVideoRef` as an invisible, muted media element with `preload="auto"`, `aria-hidden="true"`, and `onLoadedMetadata={syncPictureInPictureVideo}`. Do not add a visible video surface or an autoplay effect. The element only supplies the existing Picture-in-Picture button with a media source and synchronized playback position; it must not create a Tauri window or alter the final-effect window lifecycle.

- [ ] **Step 5: Verify the layout contract and build**

Run:

```bash
cd desktop/ui && node --test src/desktop-three-column-layout.test.mjs
cd desktop/ui && pnpm build
```

Expected: the focused layout test passes and the TypeScript/Vite production build exits with code 0.

- [ ] **Step 6: Commit the layout implementation**

```bash
git add desktop/ui/src/App.tsx desktop/ui/src/desktop-layout.css desktop/ui/src/desktop-three-column-layout.test.mjs desktop/ui/package.json
git commit -m "feat: arrange desktop UI as three-column workbench"
```

---

### Task 3: Restore Ant Design default theme

**Files:**
- Modify: `desktop/ui/src/main.tsx`
- Test: `desktop/ui/package.json` existing build command

**Interfaces:**
- `App` remains mounted under Ant Design `ConfigProvider` and `AntApp`.
- No theme token or color is consumed by `App.tsx` after this task.

- [ ] **Step 1: Remove only the custom theme configuration**

Change the import to:

```ts
import { App as AntApp, ConfigProvider } from 'antd';
```

Replace the configured provider with:

```tsx
<ConfigProvider>
  <AntApp>
    <App />
  </AntApp>
</ConfigProvider>
```

Do not change `React.StrictMode`, the root element lookup, or the `App` import.

- [ ] **Step 2: Verify the theme configuration and build**

Run:

```bash
cd desktop/ui && pnpm build
```

Expected: TypeScript and Vite both exit with code 0; `main.tsx` no longer contains `darkAlgorithm`, `colorPrimary`, `colorBgBase`, or the custom `components` token block.

- [ ] **Step 3: Commit the theme change**

```bash
git add desktop/ui/src/main.tsx
git commit -m "style: restore default Ant Design theme"
```

---

### Task 4: Whole-page cleanup and verification

**Files:**
- Modify only files touched by Tasks 1-3 if cleanup is required.

- [ ] **Step 1: Inspect the final diff and search for stale layout/theme code**

Run:

```bash
git diff --check HEAD~3..HEAD
rg -n "darkAlgorithm|colorBgBase|colorPrimary|position: 'fixed'|width: 1|height: 1|desktop-workspace|desktop-column-(source|audio|video)" desktop/ui/src
```

Remove only stale imports, dead JSX wrappers, old hidden-preview styles, and debug output introduced or exposed by this work. Do not remove existing Tauri behavior that still has a caller.

- [ ] **Step 2: Run the complete desktop UI checks**

Run:

```bash
cd desktop/ui && pnpm test
cd desktop/ui && pnpm build
```

Expected: all existing Node tests plus `桌面三列布局.test.mjs` pass, and the production build exits with code 0.

- [ ] **Step 3: Review the working tree for unrelated changes**

Run:

```bash
git status --short
git diff --stat
```

Leave pre-existing user changes untouched. The final report must list this task's files, exact commands and results, unverified visual/manual checks, and remaining risks.
