# C14 AkVirtualCamera 契约与输出状态实现记录

日期：2026-09-03

## 本轮落地

- `GpAutoLive.Contracts/VirtualCameraContracts.cs` 对齐 Rust/Tauri `autolive-virtual-camera-contract` 的首版固定合同：`GpAutoLive Camera`、YUY2、1280×720、30fps、`zero_copy=false`。
- 增加 GPU 真实性事实校验：只接受 Windows Graphics Capture、非 WARP D3D11 适配器、GPU 缩放/色彩转换、`akvcam_mmap_cpu` 末端传输和固定输出规格。
- 增加状态、代际、下游客户端数、latest-wins 帧、YUY2 limited-range 黑帧和回读指标合同。黑帧使用 Y=16/U=128/V=128，避免全零 YUV 被下游渲染成绿色。
- `GpAutoLive.Core/VirtualCameraOutputManager.cs` 作为纯逻辑唯一所有者，提供安装→启动→Ready→Streaming→恢复/停止状态边界；容量 1 的最新帧替换、旧代际拒绝、回读 P50/P95/P99 和 512 样本上限均可测试。
- WPF“GpAutoLive Camera”卡片增加脱敏状态文案；当前只展示“契约已接入、WGC/D3D11/AkVirtualCamera sidecar 待验收”，不创建系统设备、不启动未签名 sidecar。

## 明确边界

本轮没有修改 `desktop/`，也没有把 WGC、D3D11、DirectShow 注册、GPL sidecar 或安装/卸载操作伪装成已完成。真实 Windows 原生捕获、一次有界 GPU→CPU 回读、sidecar IPC、签名、许可证、下游客户端兼容和 30 分钟 GPU 长稳仍按虚拟摄像头专项方案门禁执行。

## 验证

- `dotnet build GpAutoLive.Windows.slnx -c Release --no-restore`：0 警告、0 错误。
- `dotnet test GpAutoLive.Windows.slnx -c Release --no-restore --logger "console;verbosity=minimal"`：Contracts 21、Core 64、Media 91、Windows 111、App 25，合计 **312 项通过**。
- 新增测试覆盖固定配置/JSON 字段、GPU WARP/CPU 转换拒绝、YUY2 黑帧、状态机、下游客户端状态、代际与 latest-wins、黑帧策略、停止清理和有界回读百分位。

## v55 发布复验

- 正式安装候选：`artifacts/csharp-windows-controller-20260903-v55`，根目录 8 个文件、1,174,247 bytes；`GpAutoLive.exe` 162,816 bytes。
- 独立符号包：`artifacts/csharp-windows-symbols-20260903-v55`，7 个文件、394,365 bytes；外置媒体运行时复用 v54 的 13 个硬链接文件、352,365,694 bytes。
- 5 项媒体资源大小与 SHA-256 全部匹配；锁定 `.tools/dotnet` 启动/关闭冒烟退出码 0，无 `GpAutoLive/mpv/ffmpeg/ffprobe` 残留。
- 5 秒空闲基线为 4 个样本，私有工作集峰值 84,037,632 bytes、工作集峰值 145,567,744 bytes、CPU 峰值 1.90%；该数据不等价于 30 分钟 GPU 长稳门禁。
