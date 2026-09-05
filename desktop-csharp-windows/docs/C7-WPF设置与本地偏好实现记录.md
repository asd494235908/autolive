# C7 WPF 设置与本地偏好实现记录

## 范围

本增量只处理 Windows 桌面端的低敏感 UI 偏好，不改变 Rust/Tauri/React 参考目录，也不新增 Go API、数据库或媒体任务。

- “设置”按钮和 `Ctrl+,` 打开单实例模态设置窗口。
- 性能采样开关控制标题栏 1Hz `DispatcherTimer`；关闭后停止定时器和采样任务，并显示已关闭状态。
- 快速参数卡展开开关控制主窗口卡片可见性，保存后即时应用。
- 下次输出入口只记忆 `preview`、`rtmp` 或 `virtual_camera`，不会自动启动推流或虚拟摄像头。
- 主窗口“输出与交互”标题旁显示已记忆的下次入口，避免设置保存后没有可见反馈。
- 主题和语言目前固定为深色/简体中文，字段保留在 Core 白名单中但不在 UI 中伪装成已支持的切换能力。

## 文件职责

- `src/GpAutoLive.Core/Configuration/UserPreferences.cs`：维护主题、语言和输出模式白名单，并提供 `WithUiSettings` 规范化入口。
- `src/GpAutoLive.App/Features/Settings/DesktopSettingsDraft.cs`：隔离设置窗口编辑态，只携带低敏感 UI 字段。
- `src/GpAutoLive.App/Features/Settings/DesktopPreferencesCoordinator.cs`：以同一原子 INI 边界保存设置和主窗口几何。
- `src/GpAutoLive.App/Features/Settings/SettingsWindow.xaml[.cs]`：WPF 设置界面与键盘/辅助功能入口。
- `src/GpAutoLive.App/MainWindow.xaml[.cs]`：设置入口、偏好加载和关闭时保存；性能采样与偏好应用实现拆分到同目录的 `MainWindow.Performance.cs`，保持同一 `MainWindow` partial 生命周期。

## 保存与生命周期

设置窗口提交前不会触碰媒体池、播放会话、RTMP 地址、音频内容或凭据。提交时由 Core 校验白名单，再由 `IniUserPreferencesStore` 原子替换 `app.ini`；保存失败只返回脱敏状态，既有媒体会话不被取消。主窗口关闭仍会保存几何和当前偏好，窗口关闭时先取消采样和媒体任务，再释放设置窗口。

## 验证

- `UserPreferencesTests` 覆盖允许值规范化和非法值 fail-closed。
- `DesktopSettingsDraftTests` 覆盖低敏感字段复制边界。
- Release 构建 0 警告/0 错误；全量自动化测试 **273 项通过**（Contracts 13、Core 47、Media 84、Windows 104、App 25）；`dotnet format --verify-no-changes --no-restore` 与 `tools/verify-scope.ps1` 通过。
- v46 本地安装候选根目录 8 个运行文件、1,054,439 bytes；符号包 7 个文件、332,758 bytes；媒体运行时仍独立为 13 个文件、352,365,694 bytes，清单 runtime 1.0.0 的 5/5 哈希匹配。
- v46 启动关闭冒烟成功（私有工作集 81,854,464 bytes、工作集 139,038,720 bytes、22 线程、1,100 句柄，退出码 0、无残留进程）；4 秒空闲基线 3 个样本，私有工作集峰值 95,473,664 bytes、工作集峰值 157,405,184 bytes、CPU 峰值 3.57%。真实 mpv `Original/Cpu4/Gpu83` 各 1/1 通过，FFmpeg→PortAudio 健康会话 1/1 通过。
- v46 最终复验重新采样：启动关闭冒烟私有工作集 82,235,392 bytes、工作集 139,481,088 bytes、22 线程、1,099 句柄，退出码 0 且无残留；4 秒空闲基线 3 个样本峰值私有工作集 97,497,088 bytes、工作集峰值 158,420,992 bytes、CPU 峰值 3.68%。

## 未完成项

设置窗口不宣称主题/语言即时切换；真实设备拔出/睡眠唤醒、目标 GPU/声卡矩阵、ZLMediaKit/RTMPS 网络、AkVirtualCamera、抖音 M1 和 30 分钟长稳仍按总计划门禁执行。
