# 桌面端启动 Loading 与首屏白屏优化设计

## 背景与目标

当前 Tauri 主窗口在 HTML 只有空 `#root` 时就可见，React 入口又同步加载 Ant Design 和约 3200 行的 `App.tsx`。因此在生产包中，用户可能先看到空白窗口；如果入口模块加载失败，也没有顶层错误回退。目标是让开发版和生产包都在 React 完成加载前显示稳定的启动反馈，并减少首屏必须执行的工作。

## 范围

### 本轮包含

- 在 `desktop/ui/index.html` 提供不依赖 React、Ant Design 或 Tauri IPC 的静态启动壳。
- 将主应用通过 `React.lazy` 延迟加载，并使用轻量 `Suspense` Loading。
- 增加顶层 Error Boundary，显示可操作的失败信息和重新加载按钮。
- 将 Ant Design `ConfigProvider` 与 `AntApp` 放到延迟加载的应用模块中，避免阻塞入口。
- 将媒体引擎、研究 Worker、研究默认参数和 speech-to-speech Worker 的非必要能力探测安排到首屏渲染后的空闲阶段；用户实际使用研究/实时音频能力时仍保留必要的显式刷新路径。
- 保留固定话术自动准备的现有业务语义；语音克隆能力在导入等待或手动操作时仍可按需探测。
- 用 Node 内置测试锁定 HTML 启动壳、入口懒加载、Suspense 和 Error Boundary 契约。

### 本轮不包含

- 不修改 Tauri 原生窗口隐藏/显示生命周期，不新增第二个原生 Splash 窗口。
- 不修改 Rust 播放状态机、媒体处理、模型处理协议或业务页面布局。
- 不把启动 Loading 做成服务端页面，不新增前端依赖。
- 不把所有 IPC 探测改成并行预加载；只延后首屏非必要工作，避免改变业务状态来源。

## 方案与架构

启动链路变为：

```text
Tauri 可见窗口
  -> index.html 静态启动壳（立即可见）
  -> main.tsx 加载轻量 React 启动组件
  -> Suspense 异步加载 App.tsx
  -> App 内部初始化 Ant Design 与业务状态
  -> 首屏渲染后空闲探测媒体/研究能力
```

静态启动壳使用深色背景、旋转指示器、应用名和“正在启动桌面端…”文案，确保即使 JavaScript 入口尚未执行也不会出现纯白空窗。React 挂载成功后，正常应用树会替换该壳，不依赖额外的 DOM 清理脚本。

`启动加载.tsx`只依赖 React：`StartupLoading`负责 Suspense 回退，`StartupErrorBoundary`负责捕获懒加载或渲染异常，并提供 `window.location.reload()` 恢复入口。它不调用 IPC，不引入 Ant Design，避免错误回退再次依赖可能失败的重模块。

`App.tsx`继续作为业务单一所有者，只把 `ConfigProvider` 和 `AntApp`移动到其默认导出内部。首屏必须的 `get_snapshot`保持现有轮询；研究、媒体引擎和 speech-to-speech 能力探测在首屏交互稳定后执行，固定话术自动准备继续使用当前源视频代际、哈希和 Worker 可用性门禁。

## 错误、恢复与兼容性

- 静态壳本身不显示错误，因为 HTML 阶段没有可可靠获取的错误上下文；React 入口或应用树失败时由 Error Boundary 接管并显示错误摘要。
- 用户点击“重新加载”刷新当前 Tauri WebView，重新执行静态壳和入口加载。
- 能力探测失败继续落到现有 `null`/不可用状态，不阻塞主页面和视频原声播放。
- `requestIdleCallback`不可用时使用带超时的 `setTimeout`回退，兼容 Windows WebView、macOS 和开发浏览器。
- 生产包继续通过 Tauri `frontendDist: ../ui/dist`加载同一份 `index.html`，开发版通过 Vite 加载同一入口，因此两条路径共享启动反馈。

## 测试与验收

- 启动契约测试断言 `index.html`含静态启动壳、可见文案和 spinner。
- 入口契约测试断言使用 `lazy(() => import('./App'))`、`Suspense`和顶层 Error Boundary，且不在 `main.tsx`同步导入 Ant Design 或 `App`。
- 启动组件测试覆盖 Loading 文案、错误边界错误提示和重新加载操作存在。
- 执行 `pnpm test`、`pnpm build`，确认生产 dist 中保留启动壳并生成分离的应用 chunk。
- 执行 Tauri Rust 格式/Clippy/测试；本机无法模拟 Windows 原生 GUI 时，明确记录未验证项，并用源码契约和本地产物验证生产路径。

