# Windows 兼容性修复实施计划

> **执行约束：** 直接在当前工作区实施，不创建 Git worktree，不覆盖用户已有改动。

**目标：** 修复上一轮审计确认的 Windows 编译、运行时进程生命周期、媒体工具路径和语音 Worker 打包问题。

**架构：** 保持 Rust/Tauri 负责 Worker 生命周期和打包资源注入，Python Worker 负责本地媒体调用；不把路径解析、进程树终止或业务逻辑扩散到 UI。测试夹具使用平台无关路径，Unix shell 行为测试仅在 Unix 编译运行。

**技术栈：** Rust/Cargo、Python `unittest`、Node.js `node:test`、Tauri 资源目录、PyInstaller。

## 修复边界与验收条件

- Rust 集成测试在 Windows 目标下不再无条件导入 `std::os::unix` 或引用 POSIX 固定路径。
- Node/Python 测试不依赖 `/tmp`、`/usr/bin` 或字符串拼接的 `/` 路径。
- speech-to-speech Python Worker 优先使用 `AUTOLIVE_FFMPEG_PATH`，开发环境回退到 PATH；Tauri 启动/能力探测时向 Worker 注入安装包内的 FFmpeg 路径。
- Windows 取消、超时和异常退出会递归终止 Worker 子进程树；Unix 继续保留现有进程组终止行为。
- PyInstaller 冻结后的 voice clone Worker 不再以自身可执行文件错误地执行 `python -m demucs.separate`，而是使用冻结 Worker 的 Demucs 内部入口；开发模式继续使用当前 Python 模块入口。
- 每项修复先增加能复现问题的测试或静态契约，再实现最小修改；不新增依赖，不重构无关模块。

## 实施任务

1. 跨平台测试夹具：修复 Rust 测试的 Unix 专属导入/脚本边界，并将 Node/Python 测试中的固定 POSIX 路径改为 `path.join`/`Path`。
2. speech-to-speech 工具路径与 Rust 注入：为 Python Worker 增加可配置 FFmpeg 解析，Rust 通过已有媒体资源解析注入打包路径。
3. Windows 进程树清理：为 speech-to-speech 和 research Worker 的终止函数增加 `taskkill /T /F`，并保持等待和清理语义。
4. PyInstaller Demucs 入口：增加冻结模式入口分派和单元测试，确保开发/打包两种运行方式分别可用。
5. 主线程审查与验证：检查 diff、删除本次暴露的未使用导入/变量，运行 Python、Node、Cargo 定向测试和适用的格式化/检查命令；无法在当前 macOS 主机完成的 Windows 原生构建单独列明。
