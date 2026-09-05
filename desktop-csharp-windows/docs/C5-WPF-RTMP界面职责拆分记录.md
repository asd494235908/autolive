# C5 WPF RTMP 界面职责拆分记录

本次仅进行文件职责拆分，不改变 RTMP 行为、字段、XAML 事件名或调用方。

- `MainWindow.xaml.cs` 保留主窗口初始化及其他界面编排。
- `MainWindow.Rtmp.cs` 承载 RTMP 配置校验、开始/停止推流、状态投影，以及媒体变更前停止推流逻辑。
- 两个 partial 文件继续共享同一个 `MainWindow` 实例字段，因此不新增状态副本，不修改 `desktop/` Rust/Tauri 参考实现。

验证：`dotnet format src/GpAutoLive.App/GpAutoLive.App.csproj --no-restore --verify-no-changes`；并执行 App Release 项目构建。
