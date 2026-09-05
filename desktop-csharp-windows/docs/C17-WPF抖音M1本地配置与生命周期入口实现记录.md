# C17 WPF 抖音 M1 本地配置与生命周期入口实现记录

日期：2026-09-03

## 本轮落地

- WPF“声音与互动”卡片增加 M1 本地配置入口：启用开关、直播间号、逐行回复池和队列容量。编辑态只保留在当前窗口内，正文不写入凭据或服务端。
- 新增 `Features/Douyin/DouyinConfigDraft`，把文本框编辑态转换为 `DouyinLiveConfig`，复用合同层的房间号、回复池、Unicode/UTF-8 长度和队列范围校验，避免页面重复实现规则。
- 增加“启动 M1 / 暂停 / 恢复 / 停止”按钮。启动只进入本地 `WaitingQr` 状态；暂停时允许修改队列容量，恢复时提交新的容量；任何按钮都不创建网络连接、二维码、sidecar 或发送任务。
- 登录失效和窗口关闭会停止并清空本地 M1 状态，避免会话队列跨授权边界残留。状态文案继续明确“扫码/sidecar 待验收”。
- 保持最小实现：没有新增状态仓库、持久化格式、网络客户端或后台 Worker；真实平台接入仍由后续受管 sidecar 负责。

## 明确边界

当前按钮是本地合同/状态演练入口，不是抖音可用连接。没有 QR 登录、`WebcastChatMessage` 真实读取、平台发送、自回显确认、重连和限频；“启用本地自动回应”只控制合同配置，不能绕过真实 sidecar 门禁。

## 验证

- `dotnet build GpAutoLive.Windows.slnx -c Release --no-restore`：0 警告、0 错误。
- `dotnet test GpAutoLive.Windows.slnx -c Release --no-build --no-restore --logger "console;verbosity=minimal"`：Contracts 25、Core 71、Media 91、Windows 116、App 29，合计 **332 项通过**。
- `dotnet format GpAutoLive.Windows.slnx --verify-no-changes --no-restore`：通过；`tools/verify-scope.ps1`：通过，Rust/Tauri `desktop/` 跟踪文件未改变。
- App 测试覆盖逐行文本拆分、空行处理、重复回复规范化、非数字队列容量和不安全房间号拒绝；构建覆盖 WPF 控件名称与事件绑定。

## v58 发布复验

- 正式候选：`artifacts/csharp-windows-controller-20260903-v58`，根目录 8 个文件、1,225,959 bytes，`GpAutoLive.exe` 162,816 bytes；独立符号包 7 个文件、420,368 bytes；外置媒体运行时复用 v57 的 13 个硬链接文件、352,365,694 bytes。
- 资源清单 5/5 大小与 SHA-256 匹配；使用锁定 `.tools/dotnet` 直接启动/关闭成功，标题为 `GpAutoLive`、退出码 0、无 `GpAutoLive/mpv/ffmpeg/ffprobe` 残留。
- 5 秒空闲基线 4 个样本：私有工作集峰值 85,204,992 bytes、工作集峰值 146,878,464 bytes、CPU 峰值 1.65%；该基线不等价于真实抖音或 30 分钟媒体长稳门禁。

## 后续门禁

真实 QR/协议 sidecar、账号授权、平台发送限频、自回显验证、退出清理、目标 GPU/声卡矩阵和长稳测试仍保持“正式需求·待实施/未验收”。
