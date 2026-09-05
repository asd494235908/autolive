# C6 WPF 性能边界审查记录

日期：2026-09-03  
状态：代码已接入·性能门禁未验收

## 审查结论

- `MainWindow.xaml` 的媒体 `ListBox` 保持内置 `ScrollViewer`，启用 `VirtualizingPanel.IsVirtualizing=True` 与 `VirtualizationMode=Recycling`；左栏没有再包一层无限高度的外部 `ScrollViewer`。播放池上限仍由 Core 限制为 100 项。
- 当前 UI 没有缩略图解码、原图缓存或媒体正文缓存，因此不存在“缩略图缓存无上限”路径。后续增加缩略图时必须按显示尺寸解码，并先加入数量/总字节上限和回收测试。
- 当前 UI 没有日志/弹幕 `ObservableCollection` 或无限历史列表；状态只投影为有限短文本。后续增加日志视图时必须使用条目数与 UTF-8 字节双上限，不得把原始 FFmpeg/stdout 或敏感信息写入 UI。
- 性能采样由已有 `WindowsProcessPerformanceSampler` 提供，WPF 用 1Hz `DispatcherTimer` 且有 `_performanceSampleInFlight` 并发门禁；采样结果只投影脱敏短文本，不在计时器内保存历史。
- 麦克风和抖音 sidecar 快照是后台事件。此前每次事件都直接投递 Dispatcher；本轮新增 `LatestWinsAsyncUpdateQueue`，每个来源最多保留一个未执行更新，正在执行时只保留最后一个快照，并在窗口关闭时丢弃待处理更新。

## 变更边界

新增：

- `src/GpAutoLive.App/Features/Performance/LatestWinsAsyncUpdateQueue.cs`：只负责后台快照到 UI Dispatcher 的有界 latest-wins 调度，不拥有业务状态。
- `tests/GpAutoLive.App.Tests/LatestWinsAsyncUpdateQueueTests.cs`：覆盖待执行更新替换、异步更新串行化/保留最新项、Dispose 丢弃和拒绝新更新。

接线：

- `MainWindow.AudioInterlude.cs` 的麦克风快照通过队列投影。
- `MainWindow.Douyin.cs` 的 sidecar 快照通过队列投影。
- `MainWindow.xaml.cs` 在窗口关闭时释放两个队列；更新异常只显示固定脱敏状态，不输出异常正文。

## 验证与未验收项

本轮应执行 App 测试、解决方案全量测试、格式/作用域检查和 C3～C5 离线矩阵。该改动不能证明冷/热启动、100 项 60Hz 滚动、30 分钟内存趋势、目标 GPU、真实声卡或 sidecar 网络门禁达标；这些仍需同机同构建原始报告和真实设备验收。
