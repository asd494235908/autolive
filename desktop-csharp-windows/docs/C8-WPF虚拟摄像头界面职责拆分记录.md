# C8 WPF 虚拟摄像头界面职责拆分记录

本次仅进行 `MainWindow` 的文件职责拆分，不改变虚拟摄像头行为、字段、XAML 事件名、状态语义或异步/取消策略。

## 拆分结果

- 新增 `src/GpAutoLive.App/MainWindow.VirtualCamera.cs`，继续使用同一个 `MainWindow` partial 类型和原有私有字段。
- 移入虚拟摄像头输出状态投影、安装/sidecar/D3D11/WGC 探测、启动/停止控制、探测刷新和媒体池变更前停止逻辑。
- 同时移入只服务于最终效果 HWND 绑定的 `RefreshVirtualCameraSurfaceBinding`；最终效果窗口仍由主窗口负责创建和生命周期管理。
- `MainWindow.xaml.cs` 保留窗口生命周期、登录/媒体编排和调用边界；不新增状态副本、不引入通用工具层。
- 未修改 `desktop/` Rust/Tauri 参考实现，也未删除任何源码或用户文件。

## 保持不变的边界

- XAML 仍绑定原有 `RefreshVirtualCameraButton_Click`、`StartVirtualCameraButton_Click` 和 `StopVirtualCameraButton_Click`。
- 启动前仍要求安装探测、受信任 sidecar、D3D11、WGC 和最终效果 HWND 全部通过；未通过时不启动进程或设备注册。
- 所有异步探测、启动、停止继续使用 `_windowCancellation`，窗口关闭时保持原有取消和有界清理语义。
- 媒体池编辑仍先停止虚拟摄像头，停止失败则保持旧媒体池；不改变 RTMP、本地播放和音频流程。

## 验证

本次拆分完成后执行：

```powershell
dotnet build src/GpAutoLive.App/GpAutoLive.App.csproj -c Release --no-restore -p:Platform=x64 -p:OutputPath=artifacts/app-virtual-camera-build/
dotnet format GpAutoLive.Windows.slnx --no-restore --verify-no-changes --verbosity minimal
git diff --check
```

构建产物使用独立输出目录，避免影响用户当前查看的开发端窗口；测试结果以主线程统一回归记录为准。
