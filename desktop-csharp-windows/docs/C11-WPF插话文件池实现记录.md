# C11 WPF 插话文件池实现记录

日期：2026-09-03

## 本轮落地

- 在 `GpAutoLive.Contracts` 增加插话池稳定合同：最多 1000 个文件、单路径 UTF-8 4 KiB、快照路径总量 512 KiB，并复用现有 17 种媒体扩展名。
- 在 `GpAutoLive.Core` 增加 `InterludeFilePoolService`，作为插话目录快照的唯一所有者。扫描使用有界 DFS，跳过 ReparsePoint（符号链接/联接），规范化目录和文件路径，按路径稳定排序。
- 只有完整扫描成功后才原子替换快照；根目录不存在、路径超限、候选超限、取消或根目录读取失败时保持旧快照。目录内单项访问失败按参考端语义跳过，不把系统异常正文暴露给 UI。
- WPF“声音与互动”卡片增加“插话文件池（递归扫描）”：用户显式选择目录、查看候选数量、清空快照。选择或清空不会删除磁盘文件，也不会启动 FFmpeg、PortAudio 或其他外部进程。

## 与 Rust/Tauri 复刻边界

- Rust/Tauri `interlude_player` 的递归目录、17 种扩展名、视频作为音频候选和 1000 项上限已在 C# 纯逻辑层对齐。
- 22 套声音预设、随机周期、duck 曲线、PCM 多轨混合和可听输出仍未接入；本轮 UI 明确显示“仅建立目录快照，PCM 插话混音待接入”，不伪装为可播放能力。
- 不修改 `E:\aotlve\desktop` 下任何 Rust/Tauri/React 文件；目录快照只存在当前进程内存，不写入普通 JSON/INI。

## 验证

- `dotnet build GpAutoLive.Windows.slnx --no-restore`：0 警告、0 错误。
- `dotnet test GpAutoLive.Windows.slnx --no-build --no-restore`：286 项通过（本轮新增 Core 5 项，并包含此前已接入的媒体混音边界测试）。Release 构建与 Release 测试同样为 0 警告、0 错误、286 项通过。
- `dotnet format GpAutoLive.Windows.slnx --verify-no-changes --no-restore`：通过。
- `tools/verify-scope.ps1`：通过，C# 任务未修改 `desktop/`。

## v49 发布复验

- 正式安装候选：`artifacts/csharp-windows-controller-20260903-v49`，根目录 8 个运行文件、1,090,791 bytes；`GpAutoLive.exe` 162,816 bytes。
- 独立符号包：`artifacts/csharp-windows-symbols-20260903-v49`，7 个文件、347,032 bytes。
- 外置媒体运行时：13 个文件、352,365,694 bytes；资源清单 5/5 大小与 SHA-256 匹配。
- 启动关闭冒烟：`CloseMainWindow=True`、等待退出成功、退出码 0，无 `GpAutoLive/mpv/ffmpeg/ffprobe` 残留；一次采样私有工作集 82,120,704 bytes、工作集 138,227,712 bytes、22 线程、1,078 句柄。
- 最终 4 秒空闲基线 3 个样本：私有工作集峰值 83,283,968 bytes、工作集峰值 142,831,616 bytes、CPU 峰值 1.52%，原始文件为 `artifacts/csharp-windows-baseline-20260903-v49-final3.json`。该数据用于同机回归，不等价于 30 分钟门禁。

## 待验收/后续

- 真实 FFmpeg 多轨解码、可听 PCM 混音、插话与普通声音的 attack/release duck、固定话术/麦克风抢占后的恢复、22 套预设和随机周期仍按总计划执行。
- 目录快照当前不持久化；如后续增加配置，必须沿用版本化 JSON/原子写入边界，并单独限制路径和候选数量。
