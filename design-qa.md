# 桌面主界面设计 QA

## 对比基线

- source visual truth path: `C:/Users/asd49/xwechat_files/wxid_rud357apqrci12_129a/temp/RWTemp/2026-08/615eea485d41d697d0197cc66da82d52/5674d45c92993262e575e8f2a419d76c.png`
- implementation screenshot path: `E:/aotlve/artifacts/design-qa/implementation-final-1728x1044.jpg`
- full-view comparison evidence: `E:/aotlve/artifacts/design-qa/comparison-final.png`
- viewport: `1728 × 1044` CSS px，device scale factor `1`
- source pixels: `1728 × 1075`，对比时裁掉底部 31px 系统任务栏，归一化为 `1728 × 1044`
- implementation pixels: `1728 × 1044`
- state: 参考图为已导入/运行态；浏览器实现证据为无 Tauri IPC 的真实空态。只比较应用自有的框架、比例、密度、字体层级、颜色和控件位置，不把两种运行状态的业务数值差异误报为视觉缺陷。

## Findings

- 无剩余 P0/P1/P2 视觉问题。
- 字体与排版：实现采用 `Inter / Segoe UI / Microsoft YaHei UI` 回退链，字号、粗细和紧凑数字层级与参考图一致；中文小字在 100% 缩放下无异常换行。数据卡使用稳定的单行截断，避免数值跳动。
- 间距与布局：最终实测工作区 `1704 × 984`、外边距 `12px`、栏距 `12px`；三栏宽度为 `320px / 1088px / 272px`。状态卡 `78px`、参数卡 `82px`、左侧素材池 `460px`、右侧输出卡最小高度 `120px`、声音卡 `544px`，与参考图主要分区和密度对齐。
- 颜色与视觉 Token：深色基底、卡片层级、细边框及青/蓝/粉/黄状态色与参考图同一视觉语言；状态除颜色外同时有文字和进度表达。未使用渐变或修改 Ant Design 内部选择器。
- 图片与图标：顶栏使用仓库真实应用图标；操作图标统一来自 `@ant-design/icons`，没有 emoji、手绘 SVG、CSS 图形或占位图替代。
- 文案与内容：参考图中的 OBS/RTMP、检测规避、研究频段和实时话术幻化均按本项目产品边界替换为单源播放、基础视频参数、普通声音状态、实际音频出口和只读诊断；这是已确认的产品范围映射，不是视觉遗漏。
- 响应式：`900 × 900` 验证为素材 → 视频/参数 → 声音/输出的单列顺序，页面宽度和根滚动宽度均为 `900px`，无横向溢出。
- 交互与可访问性：关键图标按钮有 `aria-label`，DOM/键盘顺序与视觉职责一致，禁用、加载、空态和错误态使用 Ant Design 语义组件；抽屉保留关闭路径。浏览器最终刷新未产生新的运行时错误或 Ant Design 弃用告警。

## Focused region comparison

- 中栏状态条与参数网格：在 `comparison-final.png` 中逐项核对六张状态卡、七列网格、卡片高度、标签/大数值/进度层级；细节清晰，无需额外放大裁片。
- 左栏与右栏：同一合成图中核对素材池、播放控制、输出入口、声音诊断和处理步骤的顶部位置及高度。输出卡使用 `120px` 最小高度并允许内容自适应，右栏仍按输出、声音诊断、处理引擎的顺序保持参考图节奏，未再出现按钮或说明文字裁切。

## Comparison history

1. Pass 1（`comparison-pass1.png`）：发现 P1 信息密度偏低。状态卡约 `97.7px`、视频参数卡 `120px`，左侧素材区和右侧声音诊断区明显短于参考图，中栏首屏留白过多。
2. 修复：状态卡压缩为 `78px`；参数卡重排为 `82px`；将真实普通声音参数状态和已选预设加入同尺寸只读卡片；素材池固定为 `460px`；声音卡补齐采样率、帧数、RMS、峰值、低频 RMS 和截止频率诊断；右栏按参考图重排为 `96px + 544px`。
3. Final（`comparison-final.png`）：在同一 `1728 × 1044` 视口重新捕获并合成对比，早期 P1 均已消除。最终 56 张真实能力卡形成与参考图相当的首屏密度，无 P0/P1/P2 残留。

## Open Questions

- 浏览器无法访问 Tauri 本地文件/IPC，因此最终证据是产品真实空态。已导入视频后的动态数值、波形和最终效果窗口仍需 Windows Tauri 实机冒烟；这属于运行环境验证，不是当前视觉阻塞。

## Implementation Checklist

- [x] 同尺寸全屏合成对比
- [x] P1 密度和主要分区比例修复
- [x] 字体、间距、颜色、图片/图标、文案五项表面检查
- [x] 900px 窄屏无横向溢出
- [x] 浏览器控制台最终刷新无新增错误
- [x] 保持本地预览运行并标记为可交付

## Follow-up Polish

- P3：在真实 Windows Tauri/WebView2 中导入一段视频后，再补一张运行态截图，可进一步核对长文件名截断、实时波形和状态值变化时的稳定性。

final result: passed

## 声音抽屉增量设计 QA（2026-08-20）

### 对比证据

- source visual truth paths:
  - `E:/aotlve/artifacts/drawer-optimization/01-advanced-before.png`
  - `E:/aotlve/artifacts/drawer-optimization/02-interlude-before.png`
  - `E:/aotlve/artifacts/drawer-optimization/03-fixed-speech-before.png`
- implementation screenshot paths:
  - `E:/aotlve/artifacts/drawer-optimization/01-advanced-after-1280x720.png`
  - `E:/aotlve/artifacts/drawer-optimization/01-advanced-after-bottom-1280x720.png`
  - `E:/aotlve/artifacts/drawer-optimization/02-interlude-after-1280x720.png`
  - `E:/aotlve/artifacts/drawer-optimization/03-fixed-speech-after-1280x720.png`
- narrow viewport evidence:
  - `E:/aotlve/artifacts/drawer-optimization/01-advanced-after-600x720.png`
  - `E:/aotlve/artifacts/drawer-optimization/02-interlude-after-600x720.png`
  - `E:/aotlve/artifacts/drawer-optimization/03-fixed-speech-after-600x720.png`
- full-view comparison evidence:
  - `E:/aotlve/artifacts/drawer-optimization/01-advanced-comparison.png`
  - `E:/aotlve/artifacts/drawer-optimization/02-interlude-comparison.png`
  - `E:/aotlve/artifacts/drawer-optimization/03-fixed-speech-comparison.png`
- viewports: `1280 × 720` 与 `600 × 720` CSS px，device scale factor `1`；同组 source / implementation 使用相同尺寸和空态。

### Findings

- 无剩余 P0/P1/P2 视觉问题。
- 字体与排版：标题改为“主标题 + 真实能力说明”的双行结构，分区标题使用语义化三级标题；表单字段都保留持续可见标签，不再依赖 placeholder 说明用途。
- 间距与布局：高级声音使用 `760px` 响应式宽度，插话和固定话术使用 `560px`；正文独立滚动，头部和底部操作区固定。高级预设按 `4 / 2 / 1` 列降级，参数状态在窄屏使用单列，未产生横向溢出。
- 颜色与视觉 Token：移除原高级抽屉的紫色大面积底色，统一到主工作台的深色中性表面、边框和遮罩层；次要文字对比度提高到 `#9a9ba5`，状态同时有文字说明，不依赖颜色区分。
- 图片与图标：继续使用 Ant Design 图标和公开组件 API；没有 emoji、手绘 SVG、CSS 图形或 `.ant-*` 内部样式覆盖。
- 文案与内容：高级声音只表达普通声音处理、PortAudio 实际出口、预设和参数生效状态；插话只表达本地目录、间隔、音量和原声 duck；固定话术只表达系统本地语音朗读。没有引入实时话术幻化、模型租约或研究参数。
- 交互：逐一验证三个入口打开/关闭；高级抽屉滚动到“缓存管理”时底部操作区保持可见；关闭后重新打开回到顶部；固定话术删除使用二次确认，取消按钮改为语义明确的“停止朗读”；`600px` 下底栏可换行且字段单列。
- 可访问性：状态摘要使用 `aria-live="polite"`，分区标题层级明确，交互控件沿用 Ant Design 键盘和焦点语义；浏览器复验的 warn/error 控制台日志为空。

### Comparison history

1. Baseline：高级声音被默认紫色主题占据，长内容与操作按钮一起滚动；插话参数缺少分组；固定话术主要依赖 placeholder，取消/删除语义不清。
2. Implementation：抽取共享 `FeatureDrawer`、`FeatureDrawerSection` 和 `FeatureDrawerField`，统一宽度、头部状态摘要、卡片分区、独立滚动正文和固定底栏。
3. Code-director review：发现高级参数状态在 `1280px` 视口被 Ant Design 断点渲染为三列，以及关闭重开后保留底部滚动位置；已将所有断点明确收敛为两列/窄屏一列，并启用 `destroyOnHidden`，同时将分区标题修正为语义化 `h3`。
4. Final：在 `1280 × 720` 与 `600 × 720` 重新打开、滚动和截图对比，未发现新的 P0/P1/P2；浏览器控制台无 warn/error。

### Evidence limitations

- 浏览器空态没有用户自建固定话术预设，无法在运行时点击删除确认；该路径已由静态契约测试覆盖。
- 浏览器无法连接 Windows Tauri 的真实音频设备，因此 PortAudio 出口和实际朗读仍需桌面端有设备环境时冒烟；不影响当前抽屉布局与交互验收。

final result: passed
