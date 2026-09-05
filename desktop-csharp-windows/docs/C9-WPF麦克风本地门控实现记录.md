# C9 WPF 麦克风本地能量门控实现记录

## 范围

本增量把已经存在的 PortAudio 输入流、固定容量 PCM 环缓、RMS 能量门控和音频优先级策略接入 Windows WPF 工作台。它只处理本机门控与抢占通知，不改变 Rust/Tauri/React 参考目录，也不新增 Go API、数据库或抖音登录能力。

- 设备枚举同时投影输入、输出设备；输入设备必须来自已校验的外置 `portaudio_x64.dll`，不回退系统 PATH。
- 用户显式点击“启用门控”后才打开单一 PortAudio 输入流；输入配置固定为单声道、48 kHz、256 帧块，数据只进入有界环缓和本地门控。
- `Speaking/Hangover` 会更新共享 `AudioPriorityCoordinator`，其优先级为“麦克风 > 固定话术 > 插话文件 > 原媒体”；麦克风开始说话时 WPF 会取消当前固定话术。
- 登录退出、窗口关闭、取消和启动失败均停止输入、禁用门控、释放取消源并有界等待观察任务；设备快照和 UI 文案不包含 PCM 正文。
- 输入快照投影原生 `Pa_IsStreamActive`/`Pa_IsStreamStopped` 健康状态、回调计数和状态旗标。旧版 PortAudio 缺少可选探针时返回 `Unknown`，不误报故障；观察到 `Stopped`、`Inactive` 或 `QueryError` 时进入 `Failed`、清除麦克风优先级并异步有界停止，修复设备后由用户重新启用，不自动无限重试。

## 文件职责

- `src/GpAutoLive.Windows/WindowsMicrophoneInterludeController.cs`：组合输入流、能量门控和优先级，拥有会话状态、取消、监视和资源释放。
- `src/GpAutoLive.App/MainWindow.xaml[.cs]`：输入设备下拉框、启停按钮、脱敏电平/状态投影、登录门禁和固定话术抢占接线。
- `tests/GpAutoLive.Windows.Tests/WindowsMicrophoneInterludeControllerTests.cs`：覆盖无效 DLL fail-closed、停止幂等和关闭后拒绝启动；`WindowsPortAudioInputStreamTests` 另覆盖未创建状态下的健康回退和回调计数边界。

## 明确未接入

本增量不宣称完整麦克风插话已完成：尚未实现 AEC、降噪、AGC、完整 VAD、可听 PCM 混音/回放、插话文件编排、识别、转写、模型调用、上传、变声、持久化或抖音 M1。当前 UI 必须显示“仅本地能量门控，AEC/降噪/AGC 待验收”，能力状态为“代码已接入·待真实设备验收”。

## 验证

- .NET 10 x64 项目构建通过；本轮 Windows PortAudio 相关测试 **181 项通过**，完整解决方案回归由主线程合并其他子任务后统一记录。
- 控制器测试不加载真实 DLL，不启动系统麦克风；真实 PortAudio 输入设备冒烟仍按目标 Windows 设备门禁单独执行。
- `desktop/` Rust/Tauri/React 参考目录未修改；主 EXE 仍不内嵌 PortAudio DLL，运行资源与符号包保持外置拆分。
- v47 发布候选正式安装根目录 8 个运行文件、1,074,919 bytes，`GpAutoLive.exe` 162,816 bytes；符号包 7 个文件、336,906 bytes；媒体运行时 13 个文件、352,365,694 bytes，清单 5/5 哈希匹配。启动关闭冒烟正常退出且无残留；4 秒空闲基线 3 个样本峰值私有工作集 83,099,648 bytes、工作集峰值 142,663,680 bytes、CPU 峰值 1.79%。

## 后续门禁

1. 在目标 Windows 10/11 x64 机器上验证默认输入设备、权限拒绝、设备拔插、睡眠唤醒和驱动重置。
2. 选定并审查成熟 DSP 方案后，补齐 AEC/降噪/AGC 与真实可听插话混音，再增加端到端音频夹具。
3. 在真实普通声音、固定话术和插话文件同时运行时验证抢占、静音/duck、取消和退出资源释放；未通过前不得把状态升级为“已验收”。
