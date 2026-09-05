# C3：WPF 媒体池界面职责拆分记录

## 目标

在不改变媒体池行为、共享状态所有权或 XAML 事件名的前提下，将主窗口中的媒体池 UI 与导入协调编排移到 `MainWindow.MediaPool.cs`，缩小 `MainWindow.xaml.cs` 的职责范围。

## 拆分结果

- `MainWindow.MediaPool.cs` 承载媒体池拖放、导入、选择、上移、下移、移除、清空、确认框、媒体变更前停止以及 FFprobe 运行时按需校验。
- `ApplyMediaOperation` 与媒体池操作同文件，继续复用播放 partial 提供的媒体投影和最终效果快照入口。
- `MainWindow.xaml.cs` 保留字段、构造/关闭生命周期、登录/设置/最终效果窗口和跨功能状态所有权；不新增状态副本。
- 未修改 `desktop/` Rust/Tauri 参考实现，也未删除源码或用户文件。

## 保持不变的契约

- XAML 事件 `ImportButton_Click`、`MediaPool_DragOver`、`MediaPool_Drop`、`MediaListBox_SelectionChanged`、`MoveUpButton_Click`、`MoveDownButton_Click`、`RemoveMediaButton_Click` 和 `ClearMediaButton_Click` 保持原名与绑定。
- 文件选择和拖放仍沿用同一个 `RunImportAsync`；系统拖放候选保持返回顺序并以 `Append` 提交。
- 导入流程仍先取得文件选择结果，再按需校验外置媒体运行资源并创建导入协调器，调用 `StopMediaForMutationAsync` 成功后才开始逐项探测；探测失败、取消或停止失败仍保留原播放池。
- 媒体池编辑仍统一停止 RTMP、虚拟摄像头、插话、PortAudio/FFmpeg 和 mpv 相关活动资源，并继续使用 `_windowCancellation` 与现有身份门禁。
- `MediaImportCoordinator`、`MediaRuntimeBoundary`、`MediaPoolService` 和 `WindowsExternalProcessRunner` 的依赖方向未改变，不引入新依赖或通用大杂烩目录。

## 验证

本次使用独立输出目录执行 App Release x64 构建，结果为 **0 警告/0 错误**：

```powershell
& .tools/dotnet/dotnet.exe build src/GpAutoLive.App/GpAutoLive.App.csproj `
  -c Release --no-restore -p:Platform=x64 `
  -p:OutputPath=artifacts/app-media-pool-build
```

另按项目交付门禁执行 App 测试、`dotnet format --verify-no-changes`、`git diff --check` 与 `tools/verify-scope.ps1`；临时构建目录只用于本轮验证，完成后清理，不停止正在查看的 `GpAutoLive` 窗口。

## 当前状态

代码职责拆分已完成；媒体导入与真实 Windows FFprobe/媒体硬件门禁仍以实施计划中的验收状态为准，不能因 partial 文件拆分而标记为实机验收完成。
