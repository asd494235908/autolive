# GUI 多分辨率调整与验证

日期：2026-09-07。

2026-09-09 后续修复：固定话术和抖音弹幕标题的装饰箭头接入真实折叠交互，并修复空格键被播放快捷键拦截；具体范围和验证见 [高级区域折叠交互修复](./2026-09-09-高级区域折叠交互修复.md)。

## 范围与链路

本轮仅调整当前 C# / WPF 桌面端的布局、样式和窗口尺寸。Rust/Tauri、Go、媒体算法、播放状态机、认证、配置保存、IPC 和输出协议均不属于本轮修改范围。

- 主工作台按可用逻辑宽度排列：1250 DIP 起三列，1000–1249 DIP 两列，更窄时单列；分组始终为媒体与播放、诊断与参数、输出与互动。
- 最小窗口为 640×320 DIP。矮窗口保留固定标题栏，主体提供纵向滚动；小窗口可继续滚动到功能与播放操作，正常尺寸下各列填满工作区。
- 顶栏、底部播放区按控件组换行；媒体搜索独占一行；进度条与时间使用独立列。
- 参数标题与状态徽标使用独立列，数值卡每行四项；长文本提供省略提示与完整 ToolTip。下拉内容与箭头分列，RTMP 分辨率、码率标签置于选项上方。
- 虚拟摄像头、麦克风、固定话术、抖音操作按钮移回各自卡片。RTMP 轨道选项、抖音回复池分别放在对应卡片；插话文件池紧邻麦克风区域。顶栏仍保留原有快捷入口。
- 登录与设置的窄窗口处理见 [GUI 辅助窗口说明](./GUI辅助窗口多分辨率说明.md)。最终效果窗口的 HWND、视频尺寸计算和播放链路保持不变。

`MainWindow.Layout.cs` 只写窗口高度、Grid 行列位置、列宽和 Margin，不访问服务或业务状态。`MainWindow.xaml.cs` 本轮仅增加首次显示的工作区宽高限制，以及原有 SizeChanged 中的一次布局更新调用。首次尺寸限制使用系统主工作区；跨不同 DPI 的副屏移动仍需实机验收。

移动控件保留原有名称、事件处理函数、绑定、输入限制、启用条件和命名作用域。静态事件核对未发现原事件丢失：登录 5 项、设置 3 项保持一致；主窗口相对 HEAD 多出的 `TestMicrophoneButton_Click` 是任务开始前已存在的工作区改动。本轮另接入一个纯布局 SizeChanged 事件。

**工作区原有及其他并行的认证、音频、麦克风、RTMP、安装包等未提交改动已保留，不能将整个工作区 diff 视为本轮 GUI 改动。**

## 变更文件

| 文件 | 本轮职责 |
| --- | --- |
| `src/GpAutoLive.App/MainWindow.xaml` | 主窗口响应式排布、控件归组和文字边界 |
| `src/GpAutoLive.App/MainWindow.Layout.cs` | 纯布局行列与高度更新 |
| `src/GpAutoLive.App/MainWindow.xaml.cs` | 初始尺寸限制、连接布局回调 |
| `src/GpAutoLive.App/App.xaml` | 参数文本省略与完整提示 |
| `src/GpAutoLive.App/Themes/ControlStyles.xaml` | 下拉正文与箭头隔离 |
| `src/GpAutoLive.App/Features/Auth/LoginView.xaml` | 登录卡自适应、滚动与按钮换行 |
| `src/GpAutoLive.App/Features/Settings/SettingsWindow.xaml` | 可调整大小、保存区防重叠 |
| `tests/GpAutoLive.App.Tests/ResponsiveWorkbenchLayoutTests.cs` | 分辨率、滚动、文字与功能归组回归 |
| `tests/GpAutoLive.App.Tests/AuxiliaryWindowLayoutTests.cs` | 登录与设置布局回归 |
| `tests/GpAutoLive.App.Tests/MediaPoolSearchAndStyleTests.cs` | 高级区域归组后的折叠面板数量断言 |
| `README.md`、本说明、辅助窗口说明 | 同步 GUI 范围、验证与限制 |

## 验证

全部命令在 `desktop-csharp-windows` 本机目录执行，使用项目锁定的 `.tools/dotnet/dotnet.exe`；无服务器构建，无新依赖。

失败先行：原主窗口在最初 9 组尺寸中有 5 组越界/重叠；功能归组测试初始失败 1 项；辅助窗口初始失败 2 项。修复后对窗口尺寸、滚动内容、高级选项展开、长错误提示与二维码说明进行检查。

最终布局矩阵（DIP）：640×320、640×360、853×480、999×700、1000×700、1024×600、1249×700、1250×700、1280×720、1366×768、1586×992、1920×1080、2560×1440、3840×2160。测试同时使用现有大窗口缩放计算，检查布局切换点；这属于真实 WPF Measure/Arrange/Render 验证，不等同于物理显示器 DPI 切换。

```powershell
$guiFilter = 'FullyQualifiedName~ResponsiveWorkbenchLayoutTests|FullyQualifiedName~AuxiliaryWindowLayoutTests|FullyQualifiedName~MediaPoolSearchAndStyleTests|FullyQualifiedName~DesktopSettingsDraftTests|FullyQualifiedName~LoginViewModelTests|FullyQualifiedName~LoginStartupOrderingTests|FullyQualifiedName~MicrophoneUiTests|FullyQualifiedName~MainWindowRtmpReadinessTests|FullyQualifiedName~FinalEffectWindowSizingTests|FullyQualifiedName~FinalEffectWindowPresentationTests'
./.tools/dotnet/dotnet.exe test tests/GpAutoLive.App.Tests/GpAutoLive.App.Tests.csproj -c Release -p:Platform=x64 --no-restore --filter $guiFilter --verbosity quiet --logger 'trx;LogFileName=gui-responsive-final.trx'
./.tools/dotnet/dotnet.exe build src/GpAutoLive.App/GpAutoLive.App.csproj -c Release -p:Platform=x64 --no-restore --verbosity quiet
./.tools/dotnet/dotnet.exe format whitespace src/GpAutoLive.App/GpAutoLive.App.csproj --no-restore --include src/GpAutoLive.App/MainWindow.Layout.cs src/GpAutoLive.App/MainWindow.xaml.cs --verify-no-changes --verbosity quiet
./.tools/dotnet/dotnet.exe format whitespace tests/GpAutoLive.App.Tests/GpAutoLive.App.Tests.csproj --no-restore --include tests/GpAutoLive.App.Tests/ResponsiveWorkbenchLayoutTests.cs tests/GpAutoLive.App.Tests/AuxiliaryWindowLayoutTests.cs --verify-no-changes --verbosity quiet
git diff --check
```

结果以 `tests/GpAutoLive.App.Tests/TestResults/gui-responsive-final.trx` 为准。渲染证据保存在 `artifacts/gui-responsive-20260907/after/`；设置 `AUTOLIVE_GUI_SCREENSHOT_DIR` 可重新导出。截图中的账号、参数与长提示为 GUI 测试夹具，不代表真实媒体、扫码或推流成功。

最终定向自动化记录：**55/55 通过，0 失败，0 跳过**。其中 14 组主窗口尺寸与 1 项功能归组检查全部通过；其余覆盖登录、设置、搜索/筛选、主题、麦克风门禁、RTMP 就绪投影及最终效果窗口呈现/尺寸。

| 验证类别 | 结果/边界 |
| --- | --- |
| 代码与布局检查 | 事件核对、格式、XML 与差异检查；布局变化仅限呈现属性 |
| 自动化测试 | 最终定向回归 55/55 通过，0 失败、0 跳过 |
| 本地构建 | WPF Release x64 通过，0 警告、0 错误；不生成 NSIS、不替换已安装客户端 |
| 容器健康 | 未执行，本轮不涉及容器 |
| 页面交互 | WPF 控件布局、搜索/筛选与滚动回归；未完成新版安装客户端的人工全流程点击 |
| 外部模型返回 | 未调用，与本轮 GUI 无关 |
| 真实业务结果 | 未重新执行媒体播放、麦克风、推流、抖音和虚拟摄像头实机验收 |
| 全量测试/安装包 | 未执行；本轮限 GUI，按影响范围验证，不扩大到全部媒体与发布门禁 |

## 消融结果

- 取消跨功能的“高级控制”集中区，直接复用各卡片中的现有控件；没有复制控件、处理函数或业务状态。
- 移除不再适用的固定横向偏移、拥挤列宽和本轮临时使用的递归布局遍历；不引入布局框架、转换器、缓存或后台任务。
- 保留媒体/参数/输出的独立滚动和矮窗口整体滚动，保证内容可达；保留鉴权、禁用状态、输入长度、错误提示与安全边界。
- 在所列布局验收矩阵内覆盖原操作入口；不同显示器之间动态 DPI 切换、任务栏布局变化、目标机字体差异和完整人工操作仍需实机验收。

## 渲染预览

1280×720：

![1280×720 GUI 测试夹具](../artifacts/gui-responsive-20260907/after/workbench-1280x720.png)

1586×992：

![1586×992 GUI 测试夹具](../artifacts/gui-responsive-20260907/after/workbench-1586x992.png)

## 2026-09-10 固定话术输入框紧凑高度修复

用户截图中“当前话术”输入框过大，原因是 `MainWindow.xaml` 的 `FixedSpeechTextBox` 局部指定 `Height="60"`，而同排管理按钮及附近操作按钮为 30。仅将该高度改为 **30 DIP**；不修改全局样式、布局结构或朗读业务，保留多行输入、自动换行、自动滚动、500 字限制与原管理入口状态。

实际验证：

- 使用实际 XAML 中该行及项目颜色、字体、控件样式，在独立 WPF 进程中对 340/480 DIP 两种宽度执行 Measure/Arrange，输入框实际高度均为 30，多行与字数限制保留；未创建可见窗口或操作正在运行的应用。验证脚本 `artifacts/vcam-validation/check-compact-speech.ps1`，渲染 [紧凑话术输入行](../artifacts/vcam-validation/compact-speech.png) 已目视核对。
- 本地执行 `.tools/dotnet/dotnet.exe build src/GpAutoLive.App/GpAutoLive.App.csproj -c Release -p:Platform=x64 --no-restore --artifacts-path artifacts/vcam-validation --verbosity quiet`：成功，0 警告、0 错误。产物独立输出，未覆盖运行中的开发端。
- 没有运行全量产品测试：本次仅一处 XAML 高度修改，使用局部布局验证与 XAML 编译作为最小充分检查。现有窗口未重启，实际开发端需通过开发启动入口重新启动后加载更改；整页实机点击与动态 DPI 切换未执行。容器、外部模型和真实朗读业务不涉及本次布局修改。

消融检查：只改变一个属性，未新增抽象、依赖、控件或未使用代码；保留既有输入、键盘、多行滚动及业务边界。无需额外清理其他功能代码。

## 2026-09-10 输出设备常显位置

按用户指定，将原来隐藏在“插话文件池（高级）”的输出设备下拉、刷新按钮及状态移动到整个“插话与互动”区域正上方，独立常显。保持 30 DIP 紧凑行，设备名称同时显示 Host API。停止播放与麦克风后刷新、选择，下次播放生效；完整调用链与验证见 [PortAudio 输出设备实施记录](./2026-09-10-PortAudio输出设备选择与热插拔刷新方案.md)。

`artifacts/vcam-validation/check-audio-devices-layout.ps1` 读取真实控件 XAML 和项目样式，验证控件唯一、无 Expander 祖先且紧接标题之前。300/340/480 DIP 宽度下，下拉实际宽度分别为 142/182/322 DIP，按钮宽 52 DIP，均高 30 DIP、无重叠；[340 DIP 局部渲染](../artifacts/vcam-validation/audio-devices-340.png) 已目视核对。该验证为独立布局渲染，未进行当前开发端点击或真实热插拔验收。
