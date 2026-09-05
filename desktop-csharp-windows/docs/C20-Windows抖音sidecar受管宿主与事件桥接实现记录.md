# C20 Windows 抖音 sidecar 受管宿主与事件桥接实现记录

日期：2026-09-03

## 本轮落地

- 新增 `GpAutoLive.Windows/WindowsDouyinProbeEventParser.cs`：只接受参考探针的固定 JSON 事件白名单，单行限制 16 KiB，未知字段不会被返回，事件正文、Cookie、Token 和异常文本不进入 C# 状态。
- 新增 `GpAutoLive.Windows/WindowsDouyinProbeHost.cs`：消费 C19 生成的 `ExternalProcessPlan`，使用 `conda run --no-capture-output`、隐藏窗口、重定向双流、有限 stdout/stderr、取消/超时和 `WindowsJobObject` 优先的进程树清理。
- `ReadStdoutAsync` 使用固定字符缓冲逐行组装，在收到换行前同样执行 16 KiB 单行上限，避免 `StreamReader.ReadLineAsync` 对恶意无换行输出产生无界暂存。
- sidecar 事件按固定顺序投影到 `DouyinLiveManager`：扫码、登录、房间解析、公屏连接、外部弹幕观察、回复尝试、自回显过滤、通过/证据不足/失败；弹幕正文仍不穿过 C# 事件模型。
- WPF 现通过“启动/停止 M1”入口读取显式环境变量：`AUTOLIVE_DOUYIN_ROOT`、`AUTOLIVE_DOUYIN_PROBE`、`CONDA_EXE`（可选 `AUTOLIVE_CONDA_ENV`/`AUTOLIVE_DOUYIN_TIMEOUT_SEC`）；三项路径未全部提供时保持原有纯本地合同模式，不访问网络。
- `DouyinLiveState` 增加 `Inconclusive`，明确区分“未获得完整证据”和成功；WPF 状态文案与配置编辑门禁同步覆盖该终态。

## 生命周期边界

- 计划校验失败不会启动进程，也不会改变核心会话。
- 进程自然退出但没有 `probe_passed` 时标记 `Inconclusive`；`probe_failed`、`reply_failed`、超时、输出越界和读取失败均 fail-closed。
- 主动停止、窗口关闭和宿主释放都会取消读取、终止进程树、有限等待并清除内存队列/去重状态；停止超时不会伪装成成功。
- QR 路径只有在事件路径由已验证启动计划提供、文件存在且不是 ReparsePoint 时才投影到快照；停止时清除投影，不删除用户未明确交给宿主的文件。

## 明确未完成

本轮没有在真实 Conda 环境、真实 `Douyin_Spider`、抖音网络、二维码登录、WebSocket、平台发送、自回显或账号/许可证门禁上宣称通过。sidecar 仍需目标 Windows 设备的真实兼容验收；`desktop/` Rust/Tauri 文件未修改。

## 验证

- `dotnet build GpAutoLive.Windows.slnx -c Release --no-restore`：0 警告、0 错误。
- `dotnet test GpAutoLive.Windows.slnx -c Release --no-build --no-restore --logger "console;verbosity=minimal"`：Contracts 25、Core 75、Media 91、Windows 126、App 33，合计 **350 项通过**。
- 新增测试覆盖事件白名单/长度边界、事件桥接顺序与终态、核心 sidecar 终态、无效计划 fail-closed 和立即退出进程的非成功投影。
- `dotnet format GpAutoLive.Windows.slnx --verify-no-changes --no-restore`：通过；`tools/verify-scope.ps1`：通过，`desktop/` 跟踪文件未改变。

## v63 发布复验

- 正式候选：`artifacts/csharp-windows-controller-20260903-v63`，根目录 8 个运行文件、1,270,503 bytes，`GpAutoLive.exe` 162,816 bytes；独立符号包 7 个文件、432,779 bytes；外置媒体运行时 13 个文件、352,365,694 bytes，其中 5 个二进制资源通过硬链接复用。
- 资源清单 5/5 大小与 SHA-256 匹配；使用 `.tools/dotnet` 启动/关闭成功，标题为 `GpAutoLive`、`CloseMainWindow=True`、退出码 0，未发现 `GpAutoLive/mpv/ffmpeg/ffprobe` 残留。
- 5 秒空闲基线 6 个样本：私有工作集峰值 85,643,264 bytes、工作集峰值 148,094,976 bytes、CPU 峰值 1.54%。该数据仅是同机空闲对照，不等价于真实 sidecar、网络或 30 分钟长稳门禁。

## v64 安全边界修正复验

- 发布候选：`artifacts/csharp-windows-controller-20260903-v64`，根目录 8 个运行文件、1,270,503 bytes；独立符号包 7 个文件、432,951 bytes；外置媒体运行时 13 个文件、352,365,694 bytes，其中 5 个二进制资源通过硬链接复用。
- 资源清单 5/5 大小与 SHA-256 匹配；启动/关闭冒烟标题为 `GpAutoLive`、`CloseMainWindow=True`、退出码 0，无 `GpAutoLive/mpv/ffmpeg/ffprobe` 残留。
- 5 秒空闲基线 6 个样本：私有工作集峰值 85,688,320 bytes、工作集峰值 148,262,912 bytes、CPU 峰值 2.32%。该数据仅是同机空闲对照，不等价于真实 sidecar、网络或 30 分钟长稳门禁。
