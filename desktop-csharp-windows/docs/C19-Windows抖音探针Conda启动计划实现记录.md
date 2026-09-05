# C19 Windows 抖音探针 Conda 启动计划实现记录

日期：2026-09-03

## 本轮落地

- 新增 `GpAutoLive.Windows/WindowsDouyinProbeLaunchPlan.cs`，构造与 Rust/Tauri 参考探针一致的 `conda run --no-capture-output -n <env> python <script>` 参数数组。
- 启动计划要求上游目录包含 `builder/auth.py`、`dy_live/server.py` 和 `static/Live_pb2.py`，Conda 与脚本必须是已存在的普通绝对路径，拒绝 ReparsePoint。
- 环境名限制为 1～64 个 ASCII 字母/数字/点/短横线/下划线；房间号和本地回复池复用 M1 合同校验；监听超时固定 30～900 秒整数；二维码输出必须是已有目录下的 PNG 绝对路径。
- 计划使用既有 `ProcessLaunchPolicy.HiddenNoShellProcessTree`，stdout 上限 256 KiB、stderr 上限 64 KiB，并通过 `ExternalProcessPlan` 交给现有 Windows 受管进程边界；本轮只生成计划，不启动进程、不读取凭据、不连接抖音。

## 明确边界

没有实现 QR 登录、Python sidecar 实际启动、WebSocket、弹幕读取、平台发送或自回显确认。真实 sidecar 必须继续使用 Conda 锁定环境，并完成签名/许可、取消/超时、Job Object、输出事件解析和账号安全门禁。

## 验证

- `dotnet build GpAutoLive.Windows.slnx -c Release --no-restore`：0 警告、0 错误。
- `dotnet test GpAutoLive.Windows.slnx -c Release --no-build --no-restore --logger "console;verbosity=minimal"`：Contracts 25、Core 73、Media 91、Windows 120、App 29，合计 **338 项通过**。
- `dotnet format GpAutoLive.Windows.slnx --verify-no-changes --no-restore`：通过；`tools/verify-scope.ps1`：通过，Rust/Tauri `desktop/` 跟踪文件未改变。
- Windows 测试覆盖参数数组规范化、上游文件缺失、环境名/超时/PNG 路径拒绝和安全进程策略复用。

## v60 发布复验

- 正式候选：`artifacts/csharp-windows-controller-20260903-v60`，根目录 8 个文件、1,236,199 bytes，`GpAutoLive.exe` 162,816 bytes；独立符号包 7 个文件、423,211 bytes；外置媒体运行时复用 v59 的 13 个硬链接文件、352,365,694 bytes。
- 资源清单 5/5 大小与 SHA-256 匹配；锁定 `.tools/dotnet` 启动/关闭成功，标题为 `GpAutoLive`、退出码 0、无媒体进程残留。
- 5 秒空闲基线 4 个样本：私有工作集峰值 85,921,792 bytes、工作集峰值 147,005,440 bytes、CPU 峰值 1.52%；该基线不等价于真实 sidecar 或 30 分钟长稳门禁。
