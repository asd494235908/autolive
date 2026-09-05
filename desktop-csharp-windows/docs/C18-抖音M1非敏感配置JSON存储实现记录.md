# C18 抖音 M1 非敏感配置 JSON 存储实现记录

日期：2026-09-03

## 本轮落地

- 新增 `GpAutoLive.Core/Configuration/DouyinLiveConfigStore.cs`，复用现有 `VersionedJsonStore<T>` 版本封套、大小上限、敏感字段保护和原子写入，不新增第二套 JSON 基础设施。
- 配置路径固定为 `%LocalAppData%\\GpAutoLive\\profiles\\douyin\\default.json`，只保存 `enabled`、规范化房间号、去重后的本地回复池和队列容量；不保存 Cookie、Token、二维码、登录态或平台响应正文。
- WPF 启动时异步读取并校验本地配置，启动 M1 成功后原子保存当前配置；读写失败只显示脱敏提示，不阻塞窗口或取消当前内存会话。
- 停止、退出和登录失效仍清理队列/去重状态；JSON 仅用于非敏感用户偏好，不作为平台身份事实源。

## 明确边界

该文件不是抖音登录缓存，也不改变“重启重新扫码”的安全规则。真实 QR、协议 sidecar、平台发送、自回显确认和网络恢复仍未接入。

## 验证

- `dotnet build GpAutoLive.Windows.slnx -c Release --no-restore`：0 警告、0 错误。
- `dotnet test GpAutoLive.Windows.slnx -c Release --no-build --no-restore --logger "console;verbosity=minimal"`：Contracts 25、Core 73、Media 91、Windows 116、App 29，合计 **334 项通过**。
- `dotnet format GpAutoLive.Windows.slnx --verify-no-changes --no-restore`：通过；`tools/verify-scope.ps1`：通过，Rust/Tauri `desktop/` 跟踪文件未改变。
- 新增 Core 测试覆盖版本化 JSON 往返、原子临时文件清理、规范化和无效配置拒绝；App 保持异步加载/保存边界。

## v59 发布复验

- 正式候选：`artifacts/csharp-windows-controller-20260903-v59`，根目录 8 个文件、1,230,055 bytes，`GpAutoLive.exe` 162,816 bytes；独立符号包 7 个文件、421,711 bytes；外置媒体运行时复用 v58 的 13 个硬链接文件、352,365,694 bytes。
- 资源清单 5/5 大小与 SHA-256 匹配；锁定 `.tools/dotnet` 启动/关闭成功，退出码 0 且无 `GpAutoLive/mpv/ffmpeg/ffprobe` 残留。
- 5 秒空闲基线 4 个样本：私有工作集峰值 85,856,256 bytes、工作集峰值 146,931,712 bytes、CPU 峰值 1.92%；该基线不等价于真实平台或 30 分钟媒体长稳门禁。

## 后续门禁

真实 sidecar 的 Conda 环境、凭据内存生命周期、QR 文件清理、平台协议兼容、发送限频、自回显验证和退出 Job Object 仍保持“正式需求·待实施/未验收”。
