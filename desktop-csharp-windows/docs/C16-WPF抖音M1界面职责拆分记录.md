# C16 WPF 抖音 M1 界面职责拆分记录

## 本轮目标

依据《CSharp-Windows 桌面端实施计划》，将 WPF 主窗口中的抖音 M1 事件处理、状态投影和本地配置读写移到独立的 `MainWindow.Douyin.cs` partial 文件。此次变更只调整文件职责，不改变抖音 M1 的行为、状态语义或 XAML 事件绑定。

## 文件职责

- `src/GpAutoLive.App/MainWindow.Douyin.cs`
  - 持有抖音 M1 按钮事件：启动、暂停、恢复、停止。
  - 持有 sidecar 停止、快照变更和结果应用逻辑。
  - 持有抖音状态到 WPF 控件的唯一投影入口 `UpdateDouyinProjection`。
  - 持有抖音本地配置的加载、保存和路径创建辅助方法。
- `src/GpAutoLive.App/MainWindow.xaml.cs`
  - 保留窗口生命周期、登录状态、媒体/音频/其他输出功能及字段所有权。
  - 继续调用抖音 partial 暴露的私有成员；partial 合并后不改变运行时入口。

## 移动范围

已移动以下方法：

- `StartDouyinButton_Click`
- `PauseDouyinButton_Click`
- `ResumeDouyinButton_Click`
- `StopDouyinButton_Click`
- `StopDouyinSession`
- `StopDouyinSessionAsync`
- `DouyinProbeHost_SnapshotChanged`
- `ApplyDouyinProbeHostResult`
- `ApplyDouyinOperationResult`
- `UpdateDouyinProjection`
- `LoadDouyinConfigAsync`
- `PersistDouyinConfigAsync`
- `TryCreateDouyinConfigStore`

未移动固定话术、插话、通用登录/窗口生命周期和其他媒体输出逻辑；Rust/Tauri 参考目录 `desktop/` 未修改。

## 行为与安全边界

- 保留原有 XAML 事件方法名，未修改 `MainWindow.xaml`。
- 保留登录门禁、sidecar 优先路径、本地 manager 回退路径、取消令牌、关闭期间短路和错误脱敏策略。
- 抖音凭据仍由现有 sidecar/manager 生命周期管理，本文件不新增网络、模型或持久化字段。
- 状态投影仍由 `UpdateDouyinProjection` 单一入口负责，避免页面出现第二份状态事实源。

## 验证

本轮完成后执行以下本地检查：

- App Release x64 独立 `OutputPath` 构建：通过。
- App 测试：通过。
- `dotnet format ... --verify-no-changes`：通过。
- `git diff --check`：通过。

构建输出目录为本轮临时目录，验证后清理；不删除源码或用户文件。
