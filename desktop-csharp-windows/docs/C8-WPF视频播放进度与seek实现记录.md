# C8 WPF 视频播放进度与 seek 实现记录

## 范围

本增量只处理 Windows WPF 工作台的视频进度投影和绝对 seek，不改变 Rust/Tauri/React 参考目录，也不新增 Go API、数据库或媒体任务。

- mpv 状态观察中的 `time-pos` 只在当前四维 `MediaPlaybackIdentity` 匹配时投影到 `PlaybackProgress`、当前位置和时长。
- 视频源存在合法 `DurationMs` 时启用进度条；鼠标释放或方向键/Home/End 走同一串行播放命令，调用受身份保护的 `WindowsMpvPlaybackController.SeekAsync`。
- 媒体池修订、切源、停止、切换到音频或身份变化会清除旧位置；音频源保持禁用 seek，不把未接入能力显示成可用。

## 文件职责

- `src/GpAutoLive.App/MainWindow.xaml[.cs]`：进度条、时间标签、身份投影、输入事件和 seek 结果状态。
- `src/GpAutoLive.App/Features/Playback/PlaybackTimeFormatter.cs`：可选毫秒值的纯逻辑格式化，缺失显示 `—`，有效时长统一显示 `HH:MM:SS`，与工作台设计图的媒体列表、中央状态栏和底部播放栏一致。
- `tests/GpAutoLive.App.Tests/PlaybackTimeFormatterTests.cs`：缺失、零值、短时长和长时长边界测试。

## 验证

- Release 构建 0 警告/0 错误；全量自动化测试 **273 项通过**（Contracts 13、Core 47、Media 84、Windows 104、App 25）。
- v46 发布候选 `artifacts/csharp-windows-controller-20260903-v46` 已包含该接线；根目录 8 个运行文件、1,054,439 bytes，PDB/XML 独立符号包 7 个文件、332,758 bytes，外置媒体运行时 13 个文件、352,365,694 bytes。
- 资源清单 runtime 1.0.0 的 5/5 大小与 SHA-256 匹配；真实 mpv `Original/Cpu4/Gpu83` 各 1/1 播放时间/EOF 夹具、FFmpeg→PortAudio 健康会话 1/1、启动关闭冒烟和 4 秒性能基线均通过。
- v46 最终复验重新采样：启动关闭冒烟私有工作集 82,235,392 bytes、工作集 139,481,088 bytes、22 线程、1,099 句柄，退出码 0 且无残留；4 秒空闲基线 3 个样本峰值私有工作集 97,497,088 bytes、工作集峰值 158,420,992 bytes、CPU 峰值 3.68%。

## 未完成项

本增量未宣称首帧像素、暂停恢复、目标 GPU/声卡矩阵、设备热插拔/睡眠唤醒、RTMP/RTMPS 网络、AkVirtualCamera、抖音 M1 或 30 分钟长稳已完成；这些仍按总计划门禁执行。
