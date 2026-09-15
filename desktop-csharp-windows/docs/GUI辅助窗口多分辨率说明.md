# GUI 辅助窗口多分辨率说明

2026-09-07。本次仅修改 WPF 登录门禁与设置窗口的 XAML 布局；认证、授权、密码清理、配置保存、媒体执行和最终效果 HWND 链路保持现有实现。

## 布局

- 登录卡从固定宽度改为最大 430 DIP，左右保留边距；低高度时纵向滚动，授权操作随可用宽度换行。错误正文、账号、密码、登录、重试、离线续播与激活入口仍属于同一登录卡。
- 设置窗口保留默认 560×480 DIP，允许用户调整至最小 360×320 DIP，最大尺寸由 `SystemParameters.WorkArea` 限制。正文继续使用既有滚动区，底部提示与保存/取消按钮用独立列排列，长提示自动换行。
- 不改最终效果窗口：它没有正文叠加，已有工作区/DPI尺寸计算、视频比例和 HWND/WGC 边界。

## 验证

使用仓库 `desktop-csharp-windows/.tools/dotnet/dotnet.exe`，工作目录 `desktop-csharp-windows`：

```powershell
& '.\.tools\dotnet\dotnet.exe' test tests/GpAutoLive.App.Tests/GpAutoLive.App.Tests.csproj -c Release -p:Platform=x64 --no-restore --filter FullyQualifiedName~AuxiliaryWindowLayoutTests --verbosity normal
```

新增两个 WPF 实际 Measure/Arrange 布局回归：登录 360×280 DIP 下输入框不越界且滚动后激活按钮可见；设置 360×280 DIP 内容区域下长状态提示不侵入保存/取消按钮。修改前 2 项失败，修改后 2 项通过；对应测试构建为 0 警告、0 错误。

同一命令将 filter 扩展为 `FullyQualifiedName~AuxiliaryWindowLayoutTests|FullyQualifiedName~DesktopSettingsDraftTests|FullyQualifiedName~LoginViewModelTests|FullyQualifiedName~LoginStartupOrderingTests` 后，布局、设置草稿、登录模型与启动顺序回归共 17 项通过。四个变更文件的 `git diff --check` 通过，仅提示仓库 LF/CRLF 转换。

本次布局测试不等同于操作系统 DPI 切换实测。`SystemParameters.WorkArea` 的主屏工作区限制无法证明不同 DPI 副屏移动场景；100%、125%、150%、200% 缩放、多屏迁移与真实页面点击仍需实机验收。未进行完整产品构建、安装打包、外部模型调用、容器或媒体业务验收，这些均不属于辅助布局改动的验证范围。

## 最小化检查

复用 WPF 原生 ScrollViewer、WrapPanel、Grid 和现有主题，不增加依赖、布局转换器、后台任务、状态字段或业务接口；仅移除会阻止适配的固定尺寸约束，保留授权提示、全部事件/绑定、键盘入口与设置持久化边界。黄金路径和现有授权条件不变。
