# 映声工坊管理端登录页 Design QA

- source visual truth path: `C:\Users\asd49\xwechat_files\wxid_rud357apqrci12_129a\msg\file\2026-09\映声工坊登录页.html` 内嵌的首个 PNG（提取检查路径：`C:\Users\asd49\AppData\Local\Temp\yingsheng-login-reference.png`）
- repository visual asset: `E:\aotlve\admin-web\src\features\auth\assets\yingsheng-login.png`
- reported mismatch paths: `C:\Users\asd49\AppData\Local\Temp\codex-clipboard-a02dbfa4-33e2-4e82-b1d2-0e6623b4d397.png`、`C:\Users\asd49\AppData\Local\Temp\codex-clipboard-c98d838b-eae3-4bb9-a934-5de9ac7a3ab5.png`
- implementation screenshot path: Codex 内置浏览器截图仅以内存证据返回，未暴露文件系统路径；被测 Docker 页面为 `http://127.0.0.1:18092/login`
- viewport: `1280 × 720` CSS px，`deviceScaleFactor=1`；`.login-stage` 为 `1279.3125 × 720`
- source pixels: 原始视觉 `1672 × 941`；问题局部截图分别为 `251 × 103` 与 `368 × 119`
- implementation pixels: 浏览器截图按 `1280 × 720` CSS px、`deviceScaleFactor=1` 检查
- density normalization: 整页按原始 `1672:941` 比例等比缩放；局部 hover 比较以浏览器计算样式和同状态截图复核
- state: 账号页签 hover、登录按钮 hover；另保留初始态与既有表单交互检查

**Findings**

- 修复后无 P0/P1/P2 视觉差异。问题根因是 Ant Design 的 hover 选择器覆盖了透明热点：账号页签重复显示 DOM 文案，登录按钮则显示组件库默认蓝色。修复后浏览器实测账号页签 hover 的 `color` 与 `background` 均为透明；登录按钮 hover 为 `rgba(255, 255, 255, 0.1)`，文字保持透明。
- 字体与排版：品牌字、标题、说明、表单文案和功能卡文案均来自参考原图，字形、字号、字重、行高、换行与抗锯齿保持一致。
- 间距与布局节奏：使用参考 HTML 的原始热区坐标换算为百分比，卡片、字段、按钮、标签和底部能力卡的位置与原图一致。
- 颜色与视觉令牌：首屏颜色、渐变、透明度、光晕、阴影、圆角均来自原始 PNG，没有用 CSS 近似重绘可见资产。
- 图像质量与资产一致性：仓库 PNG 与 HTML 内嵌 PNG 的 SHA-256 为 `191696D898B1C6CA5CFC592A414D763D61B2D26139925A2F2760878D17F6CF3B`；未使用占位图、CSS 图形、手绘 SVG 或替代 Logo。
- 文案：参考页文案完整保留，透明热点不会再把底图已有的“账号登录”或“登录”重复绘制。手机登录、忘记密码和注册没有对应后端能力，点击时明确提示暂未开放，不伪造可用流程。

**Full-view comparison evidence**

- 原始 PNG 与 Docker `/login` 实现继续使用相同 `1672:941` 宽高比；`1280 × 720` 下截图未发现布局、裁切、字体、色彩、图片或文案新增差异。
- 修复只调整自有热点状态选择器，没有改动视觉底板、热区坐标、表单或响应式计算。

**Focused region comparison evidence**

- 对照用户提供的两个局部截图复现并检查同一状态：账号页签 hover 后 `:hover=true`、`color=rgba(0,0,0,0)`、`background=rgba(0,0,0,0)`，没有第二份文字；登录按钮 hover 动画完成后 `:hover=true`、`color=rgba(0,0,0,0)`、`background=rgba(255,255,255,0.1)`。
- 键盘 `:focus-visible` 轮廓仍独立保留；本次没有用 hover 替代焦点反馈。

**Comparison history**

1. 用户局部截图发现 P2：透明热点在 hover 时被 Ant Design 高优先级状态样式覆盖，出现重复文案和默认蓝色按钮。
2. 将自有 hover/active 选择器提高到页面作用域内的明确优先级；普通热点恢复透明，仅提交热点按原始 HTML 应用 `10%` 白色 hover 蒙层与 `12%` 蓝色 active 蒙层。
3. Docker 镜像重建后在同一页面复验，两处计算样式均符合参考，console warning/error 为 `0`；最终结果通过。

**Primary interactions tested**

- 账号输入：通过。
- 密码输入与掩码：通过。
- 密码显示/隐藏：通过。
- 记住我切换：通过。
- 手机登录暂未开放反馈：通过。
- 账号页签 hover 不重复显示文字：通过。
- 登录按钮 hover 不再显示 Ant Design 默认蓝色：通过。
- 浏览器 console warning/error：0 条。

**Implementation Checklist**

- [x] 原始视觉资产按 1:1 尺寸接入。
- [x] Ant Design 真实表单控件覆盖在对应热区。
- [x] 既有 `/api/v1/auth/login` 与 `product=autolive` 契约保持不变。
- [x] 键盘语义、可访问名称和焦点反馈可用。
- [x] hover/active 状态不会覆盖底图已有文案或颜色。
- [x] 类型检查、测试和本地生产构建通过。

**Follow-up Polish**

- 无阻塞项。窄屏继续忠实采用参考 HTML 的整页缩放，因此控件视觉尺寸较小；若后续需要移动端重排，应作为独立设计需求处理，而不是本次 1:1 复刻的一部分。

final result: passed


---

## 既有桌面端参数预览 QA（原文保留）

# 桌面端参数预览设计 QA

> 2026-08-24 当前最新满宽、自动网格、卡片内容、逐卡颜色与高亮动效决策：参数内容区填满桌面中栏实际可用宽度，不设置约 `880px`、`860–900px` 或其他固定最大宽度，宽中栏两侧不得因参数容器上限留下大面积空白。外层保持双主栏，左栏为普通视频与高级视觉、右栏为普通声音；每个主栏内部卡网格根据实际可用宽度自动决定列数并自然换行，不限制每排卡片数量，也不设置固定列数或卡宽。极窄时外层双主栏按 DOM 顺序降为单列，全程不横向裁剪。普通参数卡固定为统一 `80px` 高度，只显示参数名、当前值/进度和“已接入 / 正式需求待实现 / 待确认”三态标签，不显示底部范围、单位、接入情况或待实现原因说明句。固定视觉频段权重固定为独立 `196px` 高度、跨满所属栏，完整显示三列且不得裁剪。范围、默认值和能力状态事实仍保留在正式契约、校验及媒体边界中。每张参数卡或独立参数模块分别从《媒体参数范围与默认值》的参考图五色色板分配自己的颜色，不再按普通视频、高级视觉、普通声音大分类固定一种颜色；五色可以循环复用，分配顺序应尽量避免视觉上相邻的卡片同色，且单次挂载期内每卡颜色保持稳定。挂载后显示 `value` 实际变化时按该卡自己的颜色执行一次约 `900ms` 高亮：先短暂增强，再平滑淡出；初始挂载、相同值、仅重排和普通重渲染不触发，`prefers-reduced-motion: reduce` 下禁用。算法状态仍只由三态文字表达，颜色不表达算法状态，三态不等于编辑权限。
>
> 2026-08-24 当前最新双周期状态决策：主参数区必须同时显示视频周期与声音周期两个状态模块。两者分别显示现有配置范围、真实变化次数，并以各自同一条真实 N+1 计划的目标时间和周期长度计算进度；声音状态标签读取真实 `audio processing status`。独立模式各自推进，联动模式仍分别显示两条状态但共用同一个真实联动目标。处理关闭、播放暂停、对应周期未启用或真实计划缺失时进度为 `0` 且保持非活动，次数沿用现有调度器状态。本决策只调整状态投影，不修改调度、范围、默认值或后端算法。
>
> 下方约 `880px`/`860–900px` 居中、`172px/196px` 旧普通卡宽、固定/目标 `270–290px`、“每卡占满整栏、每横排左右两张”、“每栏两卡、整行最多四卡、先固定降栏内单列”、“说明文字至少 `12px` 且显示状态原因”、按普通视频/高级视觉/普通声音大分类固定一种颜色、约 `290ms` 瞬时闪动、主状态只显示视频周期、参数三栏及既有截图只记录此前实现和验收历史，已被当前决策覆盖，不能作为新布局、逐卡颜色、高亮动效与双周期状态的通过证据。当前契约仍需重新执行自动化与真实 Tauri 宽屏/窄屏截图验收；本次调整不扩展实时话术幻化。

## 当前契约验收（待复验）

- [ ] 参数内容区填满桌面中栏实际可用宽度，不受约 `880px`、`860–900px` 或其他最大宽度限制，宽中栏两侧没有由参数容器上限造成的大面积空白；外层桌面工作区三栏不变，参数区内部保持双主栏“左：普通视频 + 高级视觉 / 右：普通声音”，每个主栏内部卡网格根据实际可用宽度自动决定列数并自然换行，不限制每排卡片数量。
- [ ] 逐步放大和缩小时验证栏内网格列数同步增加或减少，极窄时外层双主栏按“普通视频 → 高级视觉 → 普通声音”的 DOM 顺序降为单列；所有状态均不设置固定列数或固定/目标 `270–290px` 卡宽、不产生横向裁剪。普通参数卡固定为统一 `80px` 高度，只显示参数名、当前值/进度和三态标签，不存在底部范围、单位、接入情况或待实现原因说明句；固定视觉频段权重卡固定为独立 `196px` 高度、跨满所属栏并完整显示三列，不能裁剪。
- [ ] 参数范围、默认值和能力状态定义继续由正式契约、校验及媒体边界持有；隐藏卡内说明句不能改变进度归一化、三态标签或未接入值拒绝语义。
- [ ] 每张参数卡或独立参数模块分别从参考图五色色板分配自己的颜色，不按普通视频、高级视觉、普通声音大分类共用固定色；允许五色循环复用，检查分配顺序尽量避免视觉相邻卡片同色。同次挂载期间重新渲染、值更新和排序均不改变既有逐卡颜色映射。
- [ ] 参数卡挂载后显示 `value` 实际变化时只执行一次约 `900ms` 的该卡自身颜色高亮，视觉过程为短暂增强后平滑淡出；不得继续使用约 `290ms` 的瞬时闪动。初始挂载、相同值、仅重排和普通重渲染不触发，`prefers-reduced-motion: reduce` 下保持静止。
- [ ] “已接入 / 正式需求待实现 / 待确认”三态文字完整可见，颜色不替代状态，三态不决定主面板编辑权限。
- [ ] 主参数区同屏持续显示视频周期和声音周期两个状态模块；两者分别显示既有配置范围、当前真实变化次数，并以各自同一条真实 N+1 计划的目标时间和周期长度计算进度。声音状态标签与真实 `audio processing status` 一致，不复用视频标签或本地倒计时推断。
- [ ] 独立模式下视频周期与声音周期各自推进；联动模式下两条状态保持可见并引用同一个真实联动目标。分别关闭视频/声音处理、暂停播放、关闭对应周期或清除真实计划时，进度为 `0` 且非活动，次数按现有调度器状态显示；确认不因轮询、重渲染或前端计时器伪造变化，范围、默认值、调度和后端算法未改变。
- [ ] 在真实 Tauri `1920px`、`960px` 及 Windows `125%/150%` 缩放下重新捕获对比图，并记录自动化、构建与交互结果。

## 历史对比证据（已被当前决策覆盖）

- source visual truth path:
  - `C:\Users\asd49\xwechat_files\wxid_rud357apqrci12_129a\temp\RWTemp\2026-08\615eea485d41d697d0197cc66da82d52\561db9c711ff30f746b8738c087bf3d5.png`
  - `C:\Users\asd49\AppData\Local\Temp\codex-clipboard-2373c116-c21a-415d-b696-7372540467f0.png`
  - `C:\Users\asd49\AppData\Local\Temp\codex-clipboard-91bef011-a438-46b5-9be4-44fb7b9e7931.png`
- implementation screenshot path:
  - `E:\aotlve\artifacts\design-qa\media-parameters-wide-1920x1032.jpg`
  - `E:\aotlve\artifacts\design-qa\media-parameters-narrow-960x1032.jpg`
- combined full comparison path: `E:\aotlve\artifacts\design-qa\full-comparison-source-top-implementation-bottom.png`
- combined focused comparison path: `E:\aotlve\artifacts\design-qa\focused-progress-comparison-source-top-implementation-bottom.png`
- viewport: 1920 × 1032 与 960 × 1032 CSS px 的真实 Tauri 主窗口。
- source pixels: 整页参考 1728 × 1075；进度卡局部参考 363 × 138。
- implementation pixels: 宽屏 1920 × 1032；窄屏 960 × 1032；进度卡局部裁切 292 × 114。
- density normalization: 均按 Windows 1× 逻辑像素捕获。全景比较把宽屏实现等比缩至 1728 × 929 后与 1728 × 1075 参考纵向拼接；局部比较把实现卡片等比放大到 363 × 142 后与 363 × 138 参考纵向拼接。
- state: Windows 深色主题、真实 Tauri WebView、已登录主页、未导入素材、参数面板只读默认值；分别检查最大化宽屏和系统半屏 960 px 最小宽度。

**Historical Findings**

- 无 P0/P1/P2 问题。分段进度已从 Ant Design 默认宽块改为显式紧凑尺寸，末段完整收在卡片内，没有裁切、越界或水平滚动。
- 无 P0/P1/P2 问题。普通视频、普通声音、高级视觉仍在同一参数面板内保持三栏；960 px 窗口仅重排外围工作区，没有破坏参数三栏。
- 无 P0/P1/P2 问题。青、橙、紫、品红、黄按分类/分组分配，绿色“已接入”和橙色“正式需求待实现”继续表达状态，不与装饰色混淆。

**Historical Required Fidelity Surfaces**

- Fonts and typography: 沿用现有桌面端字体和 Ant Design 文字层级；紧凑卡片标题允许自然换行，状态标签不会挤出卡片。
- Spacing and layout rhythm: 卡片内边距、卡片间距和分组间距同步收紧；宽屏保持高密度，960 px 外围区域纵向排列后仍可滚动到完整参数面板。
- Colors and visual tokens: 只使用 Ant Design 公开主题令牌的 cyan/orange/purple/magenta/yellow；背景、边框和状态色继续使用既有语义令牌。
- Image quality and asset fidelity: 参数页没有需替换的图片资产；分段进度使用成熟的 Ant Design `Progress`，没有新增手绘 SVG、CSS 图形或位图替代控件。
- Copy and content: 仍明确区分“已接入”“正式需求待实现”“待确认”；待实现说明改为“主面板始终只读”，不再把只读误写为暂时限制。
- Accessibility: 每条进度只保留 Ant Design 自身的单一 `progressbar` 语义，并提供包含名称、当前值和范围的 `aria-label`；没有嵌套重复角色。

**Historical Full-view Comparison Evidence**

- `full-comparison-source-top-implementation-bottom.png` 把参考整页与真实 Tauri 宽屏实现放在同一图中检查。实现遵循用户已确认的“三个正式分类三栏”信息结构，因此不复制参考图的七列小卡网格；高密度、多色分组和深色层级保持同一视觉方向。
- 1920 px 宽屏未见卡片、进度块或状态标签越界；右侧输出区和左侧播放区没有被中央参数区覆盖。

**Historical Focused Region Comparison Evidence**

- `focused-progress-comparison-source-top-implementation-bottom.png` 同时展示参考“饱和度”卡与实现“亮度”卡。两者均为无手柄矩形分段，已达范围和未达范围清楚区分。
- 实现的 16 段数值进度实际总宽约 94 px，在 960 px 窗口的最窄卡片内仍完整可见；末段没有通过 `overflow: hidden` 伪装截断。

**Comparison History**

1. 代码审查发现 Ant Design `steps` 默认单块约 14 px，旧实现即使修改高度仍会越界；同时发现用裁切隐藏末段会造成范围误读。
2. 修复为公开 `size={[4, 6]}`/`size={[5, 5]}`、固定步数和 2 px 步间距约束，并删除进度容器裁切。
3. 首次真实宽屏检查确认多色卡片和末段完整；随后用 Windows 半屏贴靠在 960 × 1032 复查外围响应式与参数三栏，未发现新的 P0/P1/P2 问题。

**Historical Primary Interactions And Runtime Evidence**

- 在真实 Tauri 开发窗口执行激活、三栏滚动、Windows 半屏贴靠、960 px 窄屏滚动和最大化恢复。
- `pnpm test`：229/229 通过；新增契约覆盖公开紧凑尺寸、总宽上限、五色色系、只读文案和单一 ARIA 角色。
- `pnpm build`：通过；Vite HMR 终端没有出现本次改动引起的编译错误。构建仍有既存的单块大于 500 kB 提示。

**Historical Implementation Checklist**

- [x] 缩小分段方块并保证完整显示。
- [x] 用五种参考色系丰富分类和分组。
- [x] 状态色与装饰色保持不同语义。
- [x] 960 px 最小宽度下外围布局可用、参数三栏保留。
- [x] 同步测试、产品需求、架构、参数契约和长任务计划。

**Historical Follow-up Polish**

- P3：尚未在 Windows 125%/150% 系统缩放下保存独立截图；当前逻辑尺寸、文本换行和进度总宽已有静态约束，风险较低。

historical result: passed; current contract: pending revalidation

final result: passed
