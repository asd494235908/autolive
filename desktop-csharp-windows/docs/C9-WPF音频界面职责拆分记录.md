# C9：WPF 音频界面职责拆分记录

## 目标

在不改变现有行为的前提下，将主窗口中固定话术、插话文件池和麦克风本地门控的界面编排方法移到独立的 `MainWindow.AudioInterlude.cs` partial 文件，降低 `MainWindow.xaml.cs` 的维护复杂度，并保持 WPF XAML 事件绑定稳定。

## 功能边界

本次拆分仅覆盖以下三组 UI 编排职责：

- 固定话术：Windows SAPI 启动、取消、完成观察、状态投影和适配器按需创建。
- 插话文件池：目录递归扫描、清空、播放/停止、优先级停止、完成观察、配置投影、音频源创建以及配置 JSON 读写。
- 麦克风本地门控：PortAudio 设备枚举、输入流启停、快照事件、优先级抢占和状态投影。

不在本文件中复制或拥有运行时字段，也不移动通用播放、RTMP、虚拟摄像头、抖音或媒体池方法。`MainWindow.xaml.cs` 继续作为共享状态唯一所有者；partial 文件只通过同一组私有字段访问现有生命周期、取消令牌和控制器。

## 保持不变的契约

- XAML `Click` 事件名称保持不变，WPF 事件仍由 `MainWindow` 处理。
- 固定话术的本地 SAPI、取消、抢占和脱敏状态语义保持不变。
- 插话的音频优先级、停止/完成观察、有限资源、FFmpeg 运行资源校验和 RTMP/本地音频选择保持不变。
- 麦克风仍只执行本地能量门控，不新增识别、转写、上传、AEC、降噪或 AGC 行为。
- 没有改动 `desktop/` Rust/Tauri 参考目录。

## 文件职责与依赖方向

| 文件 | 职责 |
| --- | --- |
| `MainWindow.xaml.cs` | 主窗口字段、构造/关闭生命周期、通用播放编排和跨功能状态所有权 |
| `MainWindow.AudioInterlude.cs` | 音频相关 XAML 事件处理、控制器调用和 UI 状态投影 |
| `MainWindow.xaml` | 控件及事件绑定，不因本次拆分修改 |

该文件使用 C# partial 合并为同一个 `MainWindow` 类型，不引入新的服务、全局状态、重复缓存或额外依赖。

## 验证记录

本次应执行以下 Windows 本机构建与静态检查：

```powershell
dotnet build src/GpAutoLive.App/GpAutoLive.App.csproj -c Release --no-restore -r win-x64 --self-contained false -p:Platform=x64 -p:OutputPath=artifacts/app-audio-interlude-build/
dotnet test tests/GpAutoLive.App.Tests/GpAutoLive.App.Tests.csproj -c Release --no-restore -p:Platform=x64
dotnet format GpAutoLive.Windows.slnx --no-restore --verify-no-changes --verbosity minimal
git diff --check
```

构建输出目录为本次验证产生的临时目录，验证完成后清理；不删除源码或用户文件。

## 当前状态

代码拆分已完成，行为复刻和真实 Windows 音频设备门禁仍沿用 C5 既有状态，不能因文件拆分而标记为已完成实机验收。
