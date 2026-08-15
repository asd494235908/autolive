# Task 3：Tauri 任务所有权与现有媒体/语音路径接入

## Status

完成。五个运行资源 IPC 已接入，`AppState` 持有唯一运行资源任务管理器；正式媒体、固定话术 Worker 和模型路径均切换到 `app_data_dir/runtime-resources/v0.1.0` 的已验证布局。生产 manifest 仅在运行时从 `app.path().resource_dir()/runtime-resources.json` 读取，没有使用 `include_bytes!`，也没有向仓库加入目标专属 manifest。

## 功能边界

- 实现 `get/install/cancel/import/clear` 五个命令及 Tauri 注册。
- 新增聚焦的 `runtime_resource_task.rs`，只负责运行资源操作的状态、取消令牌和线程句柄生命周期。
- 现有媒体探测、处理、speech-to-speech 媒体注入、固定话术 Worker 与模型路径改用已验证 target root/app data 布局。
- debug 继续优先既有四个 `AUTOLIVE_*` 覆盖；媒体与语音覆盖完整且可执行/完整时，状态直接返回 `ready`，不依赖当前 checkout 中不存在的生产 manifest。
- 未修改 UI、打包脚本、CI、Go 服务端或服务器，也未使用 worktree。

## RED / GREEN

### RED

1. `cargo test --test runtime_resource_commands_contract`
   - 首次因 `RuntimeResourceInstaller::from_embedded` 要求 `&'static [u8]` 而出现 E0597，证明运行时读取的 manifest 字节不能传入现有接口。
   - 放宽该参数后，命令注册、运行时 manifest、AppState 所有权、app data 路径和缺失提示等 5 项契约失败。
2. `cargo test --lib runtime_resource_task`
   - 新任务所有权测试最初因 `RuntimeResourceTask`/`RuntimeResourceTaskShutdown` 尚未实现而失败。
3. `cargo test --test runtime_resource_commands_contract duplicate_start_reuses_the_running_task_before_manifest_io -- --exact`
   - 失败，证明启动命令原先会先读取 manifest，再尝试复用任务。
4. `cargo test --lib runtime_resource_task::tests::operation_error_without_status_callback_becomes_failed -- --exact`
   - 编译失败，证明任务包装层还不能接收并兜底早期操作错误。
5. `cargo test --bin autolive-desktop-core commands::tests::development_resource_gate_requires_an_executable_file -- --exact`
   - 新测试最初因可执行文件校验帮助函数尚不存在而失败。

### GREEN

- 上述 RED 均在最小实现后通过。
- 聚焦回归：`cargo test --test runtime_resource_commands_contract --test media_engine_contract --test voice_clone_runtime_contract --test voice_clone_capability_contract`，49 passed，0 failed。
- 任务生命周期单测：5 passed，覆盖重复启动、早期失败终态、完成回收、协作退出和超时保留所有权。

## 线程生命周期

- `AppState` 持有单一 `Arc<RuntimeResourceTask>`；管理器内部唯一持有当前状态、共享取消标记和 `Option<JoinHandle<()>>`。
- install/import/clear 在专属命名线程执行，下载、复制、校验和清理不在 Tauri 主线程执行；get 的磁盘检查使用 `spawn_blocking`。
- 命令先调用 `running_status()`，正在运行时直接返回当前状态，不要求 manifest 仍可读取。并发竞态最终仍由任务管理器持有的 worker 锁串行化，因此只会启动一个操作。
- worker 结束后，status、再次 start、running-status 或 shutdown 会检查 `is_finished()` 并 `join`；panic 转换为 `failed`。installer 在发出首个状态前失败时，任务包装层也会写入 `failed`，不会残留在 `checking`。
- cancel 只设置共享原子取消标记，由 installer 的下载重试、复制和哈希检查点协作退出。
- `RunEvent::ExitRequested` 发出取消并最多等待 3 秒。协作结束时 join；超时时返回 `TimedOut`，`JoinHandle` 仍留在管理器中，没有假装已 join 或主动 detach。
- `reqwest::blocking` 请求设置 10 秒连接超时和 60 秒总超时，但 Rust 无法安全强制终止正在阻塞的请求。退出预算到期后应用继续退出，日志明确记录未在预算内停止；测试覆盖了“超时仍持有句柄，阻塞解除后可回收”。

## Manifest 与路径

- `RuntimeResourceInstaller::from_embedded` 从 `&'static [u8]` 放宽为 `&[u8]`；解析后的 manifest 自持有数据。
- 生产 manifest：`resource_dir/runtime-resources.json`，读取、资源目录、应用数据目录和目标平台不匹配错误均保留上下文。
- 正式大资源：`app_data_dir/runtime-resources/v0.1.0/<target>/...`；共享模型为 `app_data_dir/runtime-resources/v0.1.0/common/voice-models`。
- `configured_media_engine_paths_with_resource_dir` 的参数语义已改为 verified target root，不再自行附加 target triple。
- release 不读取媒体/固定话术开发环境覆盖；缺失 Worker/模型统一提示“需要下载运行资源”，不再提示重装完整版。

## 改动文件

- `desktop/src-tauri/src/commands.rs`
- `desktop/src-tauri/src/lib.rs`
- `desktop/src-tauri/src/main.rs`
- `desktop/src-tauri/src/media_engine.rs`
- `desktop/src-tauri/src/runtime_resource_task.rs`（新增，聚焦任务生命周期）
- `desktop/src-tauri/src/runtime_resources.rs`（仅放宽运行时 manifest 生命周期并增加只读访问器）
- `desktop/src-tauri/tests/runtime_resource_commands_contract.rs`（新增）
- `desktop/src-tauri/tests/media_engine_contract.rs`
- `desktop/src-tauri/tests/voice_clone_runtime_contract.rs`

## 最终验证

均在本地 `/Users/mac/work/gepin/autoLive/desktop/src-tauri` 执行：

- `cargo fmt --all -- --check`：通过。
- `cargo test --test runtime_resource_commands_contract --test media_engine_contract --test voice_clone_runtime_contract --test voice_clone_capability_contract`：49 passed，0 failed。
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`：通过，0 warning。
- `cargo test --workspace --all-features`：最终完整通过，209 passed，0 failed。
- `cargo check --workspace --all-targets --all-features`：通过。
- `git diff --check`：通过。

完整测试第一次并行运行时，仓库既有 `speech_to_speech_worker_contract::context_worker_serializes_context_and_cleans_temporary_context_file` 在固定 2000ms 限制下超时；该用例单独复跑通过（约 0.30 秒），最终全量复跑也完整通过。未修改该任务范围外的既有超时策略。

## 自审结论

- 未发现新增未使用代码、导入、调试输出、`unsafe`、`TODO` 或 `FIXME`。
- commands 中仅生产 manifest 读取仍调用 `resource_dir()`；正式大资源路径均已迁移到 app data 布局。
- 没有 `include_bytes!`，干净 checkout 不依赖目标专属 manifest 编译。
- 没有“请重新安装完整版本”残留。
- 本环境未暴露可用子代理工具，本机 `codex` 子进程也因缺少对应 Apple Silicon 可执行文件而无法启动，因此无法执行独立第二模型审查；已由主线程完成逐项 diff 自审和全量验证。

## 疑虑与未验证项

- 未在 Windows 主机验证 `.exe` 路径和退出事件行为；相关路径有平台条件测试与编译检查，但当前实际运行平台是 macOS arm64。
- 当前 checkout 按约定没有 Task 5 才会提供的生产 `runtime-resources.json`，因此未手工启动 release IPC 下载；manifest 运行时所有权、解析与布局通过自动化测试验证。
- 阻塞中的 reqwest 请求无法在 3 秒预算内被安全强制杀死；实现选择保留句柄所有权并如实返回/记录超时，进程退出后由操作系统回收。
- 独立子线程审查不可用，见自审结论。

---

# Task 3 审查修复追加报告

## Status

已修复审查提出的全部 Important 和 Minor。以下内容覆盖并替代上文关于“退出预算到期后直接退出并由操作系统回收线程”的旧语义：现在首个 3 秒预算超时会阻止退出，由单例后台协调线程等待并回收任务，之后才触发第二次退出。

## RED / GREEN

### RED

- 新增真实 installer/任务测试后，编译因缺少 `inspect_when_idle`、`record_failure` 和 `from_resource_directory` 失败，证明旧实现没有“仅运行态复用、终态重新检查、前置失败入状态”的行为入口。
- clear 行为测试调用新的可取消接口时因旧 `clear_current_release()` 不接收 cancel/回调而失败。
- release speech 契约测试因缺少 `WorkerEnvironmentPolicy`/`worker_environment_policy` 失败。
- 退出协调行为测试因 `AppState` 没有一次性协调门禁方法失败。
- clear 共享取消的端到端测试首次 GREEN 后，临时把 `start_clear` 改为不传共享 cancel，测试稳定变红：期望 `Cancelled`，实际 `NotInstalled`；恢复共享 cancel 后重新转绿。

### GREEN

- 跨组件/损坏：先通过真实 installer 完成 Media 任务，再查询 Voice 得到 `not-installed`；随后损坏 Media 文件，再查 Media 也降为 `not-installed`。
- manifest/路径失败：缺失 manifest、坏 JSON、target 不匹配和不可用 app-data 路径均通过真实 loader 产生带路径上下文的错误，并由 `RuntimeResourceTask::record_failure` 写成 `failed` 终态。
- clear：中途回调设置 cancel 得到 `Cancelled`，部分目录仍存在且 inspect 为 `not-installed`，重试 clear 可收敛；目录 symlink 被删除但外部目标保持不变；任务级共享 cancel 得到 `cancelled` 并回收句柄。
- 生命周期：普通 Drop 会先 cancel，再 join 协作 worker；并发退出门禁只有一个调用能启动协调，finished 后不能再次启动。
- release：纯策略函数同时覆盖 debug/release 决策；`cargo test --release --test speech_to_speech_worker_contract configured_worker_executable_ignores_environment_in_release -- --exact` 通过，证明 release 不读取环境指定 Worker。

## 状态与命令语义

- `get_runtime_resource_status` 只在 `running_status()` 返回 Some 时复用全局任务状态；无运行任务时先走 debug gate，否则重新加载 manifest 并 `inspect(component)`，不再返回上一次 Ready/Failed/Cancelled。
- get/install/import/clear 共用 `spawn_blocking` loader；`app_data_dir`、`resource_dir`、manifest 读取/解析和 target 校验均在阻塞线程执行。
- loader 或 inspect 失败调用 `record_failure(component, error, resource_root)` 并返回该状态，不再只返回 IPC Err。`record_failure` 在并发任务已启动时返回运行中状态，不覆盖活动任务。
- manifest 成功后，install/import/clear 最终仍由 `RuntimeResourceTask::start_operation` 的 worker 锁进行重复启动仲裁。

## clear 文件系统语义

- `clear_current_release(cancel, on_status)` 使用标准库递归遍历，每个目录项和删除动作前检查 cancel。
- 使用 `symlink_metadata` 区分 symlink，不递归进入目录 symlink；Unix 删除链接本身，Windows 按链接类型删除。
- 取消时发布 `Cancelled` 终态，不发布 Ready/NotInstalled；允许已经开始的清理留下部分目录，后续 inspect 会根据真实文件校验返回 `not-installed`，再次 clear 可完成清理。

## release speech-to-speech 信任边界

- `configured_worker_executable()` 仅在 debug 构建读取 `AUTOLIVE_SPEECH_TO_SPEECH_WORKER`；release 明确返回 unavailable，不执行环境指定程序。
- 所有 speech-to-speech 子进程 Command 在 release 都 `env_remove` Worker、FFmpeg、FFprobe 三个 `AUTOLIVE_*` 变量；有已验证 target root 时再注入其 `binaries/ffmpeg` 和 `binaries/ffprobe`。
- 本次新增修改 `desktop/src-tauri/src/speech_to_speech_worker.rs` 及其契约测试，是审查发现的 release 信任边界调用方，属于修复环境劫持的必要影响面；没有对 speech-to-speech 功能做无关重构。

## 退出与句柄所有权

- 首个 `ExitRequested` 发出 cancel 并最多等待 3 秒。
- 若返回 `TimedOut`，立即 `api.prevent_exit()`；AppState 原子门禁只允许启动一个 `runtime-resource-exit-coordinator`。
- 后台协调线程重复进行有限预算等待，直到任务 Idle/Joined，随后标记 finished 并调用 `app_handle.exit(code.unwrap_or(0))`。协调期间其他退出请求继续 prevent；程序化第二次退出看到 finished 后放行。
- `RuntimeResourceTask::Drop` 对普通非 Tauri 析构执行 cancel+join，不再通过丢弃 `JoinHandle` detach。正常 Tauri 路径已在 Drop 前完成回收，因此 Drop 应即时返回。
- Tauri builder 构建失败现在返回 `ExitCode::FAILURE`。

## 本次改动文件

- `desktop/src-tauri/src/commands.rs`
- `desktop/src-tauri/src/main.rs`
- `desktop/src-tauri/src/runtime_resource_task.rs`
- `desktop/src-tauri/src/runtime_resources.rs`
- `desktop/src-tauri/src/speech_to_speech_worker.rs`（必要的 release 信任边界）
- `desktop/src-tauri/tests/runtime_resource_commands_contract.rs`
- `desktop/src-tauri/tests/runtime_resources_contract.rs`
- `desktop/src-tauri/tests/speech_to_speech_worker_contract.rs`

## 最终验证

均在本地 `/Users/mac/work/gepin/autoLive/desktop/src-tauri` 执行：

- 聚焦行为测试（runtime command/resources/task、speech worker、media、voice）：100 passed，0 failed。
- release 门禁测试：1 passed，0 failed。
- `cargo fmt --all -- --check`：通过。
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`：通过，0 warning。
- `cargo test --workspace --all-features`：216 passed，0 failed。
- `cargo check --workspace --all-targets --all-features`：通过。
- `git diff --check`：通过。

## 疑虑与未验证项

- 当前实机为 macOS arm64；Windows 的目录 symlink 删除分支通过标准库平台 API 编译约束，但未在 Windows 主机实跑。
- 当前 checkout 仍按约定没有 Task 5 才提供的生产 manifest，因此没有进行真实生产下载；缺失/坏 manifest、target、app-data、真实 installer 和文件损坏均有自动化行为覆盖。
- 阻塞 reqwest 仍无法在 3 秒内安全强制终止；现在应用会 prevent exit 并由后台协调线程等待请求自身的 60 秒超时/协作取消后 join，不再 detach。
- 环境没有可用子代理执行接口，本机 `codex` 子进程也缺少对应 Apple Silicon 可执行文件；本轮由主线程完成逐项 diff 自审、反向回归验证和全量门禁。

---

# Task 3 第二轮复审最终追加报告

## Status

第二轮复审列出的 5 组问题已全部修复。本节覆盖上文关于标准库 clear 递归和无限退出协调器的旧说明：clear 现已改为 cap-std 目录能力递归；Tauri 退出不再 prevent/后台无限等待，3 秒预算到期或 shutdown 错误会立即强制结束进程。

## RED / GREEN

### RED

- 新增同步 barrier 竞态测试：clear 已完成条目检查、尚未删除文件时，将其父目录重命名并以指向外部目录的 symlink 替换。旧绝对路径递归稳定删除了外部同名文件，测试以 `outside file must remain` 失败；测试不依赖 sleep。
- 新增退出策略行为测试后，因缺少 `handle_runtime_resource_exit` 无法编译，证明旧实现没有可独立验证的有限退出决策边界。
- 首次运行 Release 媒体路径门禁时，生产代码正确注入打包路径，但旧测试仍错误期待开发环境覆盖，断言失败；测试随后改为按纯策略函数分别验证 Debug 覆盖与 Release 打包路径。

### GREEN

- 相同 barrier 竞态下，clear 通过已打开的目录能力删除原目录内容，只解除父能力下的替换 symlink，外部文件保持不变；若竞态留下部分目录，再次 clear 可收敛。
- 正常 clear、目录 symlink、取消后 inspect 为 `not-installed`、重试收敛及任务句柄回收测试全部通过。
- 退出策略行为测试覆盖 Idle、Joined、TimedOut 和 Err：前两者不调用强退回调，后两者各精确调用一次。
- Release 行为测试证明环境指定 Worker 不可用，并证明显式 FFmpeg/FFprobe 环境值会被已验证打包路径替换；Debug 分支仍允许开发覆盖。
- 跨组件查询/磁盘损坏重新 inspect、缺失/坏 manifest 写入 Failed 终态、共享 cancel 和 build/check 门禁均无回归。

## clear 文件系统与取消语义

- clear 先用 ambient capability 打开 `version_root` 的父目录，再仅以相对 release 名打开根目录能力；这样根目录本身也能最终由其父 capability 删除，并避免直接 ambient 打开一个可能已经被替换为逃逸 symlink 的根路径。
- 递归始终持有打开的 `Dir`：`entries()` 枚举当前能力，条目仅使用 `file_name()`；优先 `open_dir(name)` 获取子目录能力，失败时只调用父能力的 `remove_file(name)`。
- 子目录递归完成后，由打开它的父能力删除该相对名称；若名称在递归期间被替换为 symlink，则只解除该链接，不沿绝对拼接路径重新解析。
- 每个条目处理前、进度回调后、删除子目录/根目录前都会检查共享 cancel。取消返回 `Cancelled`，不会错误发布 Ready/NotInstalled；允许留下部分目录，inspect 和重试以磁盘真实状态收敛。
- 绝对路径拼接仅用于状态与错误上下文，不传给递归的 read/remove 调用。未新增依赖或 `unsafe`。

## 有限退出与句柄所有权

- `ExitRequested` 同步调用 `shutdown(Duration::from_secs(3))`；Idle/Joined 正常放行。
- TimedOut 或 shutdown Err 会先记录错误，再通过 `handle_runtime_resource_exit` 精确调用一次 `std::process::exit(code.unwrap_or(0))`。不再调用 `prevent_exit`，不再创建退出协调线程，也没有 AppState 退出原子门禁。
- 强制进程终止不会运行 Rust Drop，因此不会卡在阻塞 reqwest worker 的 join；操作系统结束该 worker。协作任务在 3 秒内结束时已经正常 reap，普通非 Tauri 析构仍由 `RuntimeResourceTask::Drop` 执行 cancel+join，避免 detach。
- Tauri builder 失败路径仍由 `main() -> ExitCode` 返回 `ExitCode::FAILURE`，全目标 check 包含二进制入口并通过。

## 静态测试边界

- 删除了退出预算、二次退出、prevent-exit 和 release env 的源码 contains 断言，相关契约均由纯函数或真实子进程行为测试覆盖。
- Task 3 范围内唯一保留的源码静态测试是五个 Tauri IPC handler 注册检查；完整启动 Tauri event loop 不适合作为该集成测试的本地行为夹具。

## 本次改动文件

- `desktop/src-tauri/src/commands.rs`
- `desktop/src-tauri/src/main.rs`
- `desktop/src-tauri/src/runtime_resource_task.rs`
- `desktop/src-tauri/src/runtime_resources.rs`
- `desktop/src-tauri/tests/runtime_resource_commands_contract.rs`
- `desktop/src-tauri/tests/runtime_resources_contract.rs`
- `desktop/src-tauri/tests/speech_to_speech_worker_contract.rs`

本轮没有继续修改 `speech_to_speech_worker.rs`；仅把上一轮已建立的 release 信任边界改为完整行为断言。该文件在上一轮的必要影响面解释仍有效，没有无关重构。

## 最终验证

均在本地 `/Users/mac/work/gepin/autoLive/desktop/src-tauri` 执行：

- clear 聚焦行为：4 passed，0 failed。
- RuntimeResourceTask 聚焦行为：8 passed，0 failed。
- runtime resource command 契约：6 passed，0 failed。
- speech worker Debug 契约：7 passed，0 failed。
- Release 门禁：`configured_worker_executable_ignores_environment_in_release` 与 `packaged_media_paths_are_injected_for_capability_probe_and_actual_task` 分别通过。
- `cargo fmt --all -- --check`：通过。
- `cargo clippy --all-targets --all-features -- -D warnings`：通过，0 warning。
- `cargo test --all-targets --all-features`：215 passed，0 failed。
- `cargo check --all-targets --all-features`：通过。
- `git diff --check`：通过。

## 疑虑与未验证项

- 当前实机为 macOS arm64；能力递归和 Unix symlink 并发替换已真实执行，Windows 文件系统行为未在 Windows 主机实跑。
- 当前 checkout 按约定没有 Task 5 才提供的生产 `runtime-resources.json`，未执行真实生产下载；manifest 失败、target/app-data 路径、跨组件状态和损坏文件均有真实 loader/installer 行为覆盖。
- 阻塞 reqwest 无法在 3 秒内被 Rust 线程安全强杀；本轮按复审要求在有限预算后终止整个进程，明确依赖操作系统结束 worker，不再声称 join，也不存在后台无限等待。

---

# Task 3 最终 capability 锚点追加报告

## Status

最终复审剩余的 capability 初始锚点、Windows 目录 symlink/junction 删除兼容及 barrier 测试增强均已完成。本节补充并覆盖上节关于 clear 初始锚点的说明。

## RED / GREEN

### RED

- 新增真实外层替换测试：installer 构造完成后，将整个 `app_data/runtime-resources` 重命名，并在原名放置指向 outside 的 symlink。旧 clear 在执行时重新 ambient 打开该绝对父路径，稳定删除了 `outside/v0.1.0/keep.txt`，测试以外部文件不存在失败。
- 新增统一 capability entry helper 契约时，测试因 `remove_capability_entry` 尚不存在而编译失败。该契约要求普通文件与空目录均可删除；两种删除方式都失败时，错误必须同时保留 `remove_file` 和 `remove_dir` 的原始错误文本。

### GREEN

- 相同外层 symlink 替换下，clear 从构造期持有的 app-data capability 相对打开 `runtime-resources`；逃逸 symlink 无法作为目录能力打开，只解除链接本身，outside 文件在首次 clear 和恢复合法目录后的重试 clear 之后均保持不变。
- 既有内部目录 barrier 竞态测试在重试 clear 后新增第二次 outside 内容断言并通过。
- 统一 entry helper 对文件先尝试 `remove_file`，失败后尝试 `remove_dir`；这覆盖 Windows 目录 symlink/junction 的删除差异，两个操作都失败时合并保留两条系统错误。两者均为父目录 capability 下的相对名称操作，不跟随目标。
- installer 的 `Clone + Send + Sync` 编译契约通过；Clone 测试使用 `Arc::ptr_eq` 确认共享同一个 app-data `Dir` 句柄。

## capability 生命周期与 clear 路径

- `RuntimeResourceInstaller::from_manifest` 必要时先创建 app-data 目录，随后立即通过 `Dir::open_ambient_dir(app_data_dir, ambient_authority())` 获取可信 capability，并以 `Arc<Dir>` 持有。
- clear 不再从 `version_root` 或其绝对 parent 重新 ambient 打开锚点。完整路径为：已持有 app-data `Dir` → 相对 `open_dir("runtime-resources")` → 相对 `open_dir("v0.1.0")` → 句柄递归。
- `runtime-resources`、release 根、递归文件条目和递归目录删除都复用 `remove_capability_entry`。所有实际 read/remove 仅接收打开的 `Dir` 与单个相对 entry name；绝对路径只用于状态和错误展示。
- `runtime_resources.rs` 中另一个 ambient open 仍是既有的本地导入源 capability 建立点，与 clear 路径无关。旧 `remove_directory_entry`、`remove_tree_cancellable`、clear 内 ambient 打开和绝对 read-dir helper 均无残留。

## 本次改动文件

- `desktop/src-tauri/src/runtime_resources.rs`
- `desktop/src-tauri/tests/runtime_resources_contract.rs`
- `.superpowers/sdd/task-3-report.md`

未修改 release speech 环境相关文件，因此按复审要求没有重复 Release 门禁。

## 最终验证

均在本地 `/Users/mac/work/gepin/autoLive/desktop/src-tauri` 执行：

- capability helper/Clone/Send/Sync 聚焦测试：2 passed，0 failed。
- clear 聚焦行为：5 passed，0 failed。
- RuntimeResourceTask 聚焦行为：8 passed，0 failed。
- runtime resource command 契约：6 passed，0 failed。
- `cargo fmt --all -- --check`：通过。
- `cargo clippy --all-targets --all-features -- -D warnings`：通过，0 warning。
- `cargo test --all-targets --all-features`：218 passed，0 failed。
- `cargo check --all-targets --all-features`：通过。
- `git diff --check`：通过。

## 疑虑与未验证项

- 当前实机为 macOS arm64；Unix 的外层/内层 symlink 替换均已真实执行。统一 helper 在当前平台完成文件、目录及双错误行为测试并保证跨平台编译，Windows 目录 symlink/junction 的实际系统调用分支仍由 Windows CI 矩阵验证。
- 当前 checkout 仍没有 Task 5 才会提供的生产 manifest，未执行真实生产资源下载；本轮不改变此前已验证的 manifest/下载路径。
