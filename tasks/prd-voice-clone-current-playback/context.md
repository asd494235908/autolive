# Context：当前文案克隆语音循环播放

## Reviewed Inputs

- 产品与架构：`产品需求文档.md`、`系统架构总览.md`、`桌面客户端架构.md`、`媒体参数范围与默认值.md`、`长任务开发总计划.md`、`实时音频幻化与循环播放方案.md`。
- React：`desktop/ui/src/App.tsx`、`desktop/ui/src/voiceCloneStatus.ts`、相关 `*.test.mjs`。
- Rust：`desktop/src-tauri/src/lib.rs`、`desktop/src-tauri/src/commands.rs`、`desktop/src-tauri/src/voice_clone.rs`、`desktop/src-tauri/tests/voice_clone_runtime_contract.rs`。
- Python：`desktop/worker/voice_clone_adapter.py`、`desktop/worker/test_voice_clone_adapter.py`。
- 构建入口：`desktop/ui/package.json`、`desktop/ui/../工具/准备语音模型资源.mjs`、`desktop/ui/../工具/准备语音Worker资源.mjs`。

## Current Data Flow

1. React 导入 MP4 后调用 `probe_local_mp4`、`start_playback`，随后等待源 MP4 完整 SHA-256。
2. 哈希完成且 Worker 可用时，React 自动调用 `prepare_voice_clone_source`。
3. Rust 通过常驻 `VoiceCloneWorkerServer` 调用 Python `prepare_source`。
4. Python 使用 FFmpeg 解码、Demucs 分离 vocals、faster-whisper 建立片段索引，并加载 XTTS-v2。
5. 当前 `start_voice_clone_replacement` 传入当前位置和文本，Python `replace_current` 生成包含前后原音的完整替换 WAV。
6. React 通过隐藏 `voiceCloneAudio` 播放结果；`getEffectiveAudioSource` 和 `complete_playback_loop` 控制音轨选择及循环恢复。

## Design Implications

- 新需求的主产物应是“当前文案对应的独立克隆音频片段”，不能继续把每次操作都实现为源音轨拼接。
- 原音轨必须保留且只在克隆片段播放窗口内静音；克隆音频 `ended`、取消、停止或失败时需要可验证地恢复。
- 循环边界不得自动触发或重播克隆音频；用户没有再次点击时，下一轮继续播放原音轨。若克隆音频跨越边界仍在播放，则保持原音轨静音直到其自然结束。
- `playback_generation`、源 MP4 SHA-256、参考人声 SHA-256、文案 SHA-256、模型版本和 operation ID 构成克隆音频身份；`loop_index` 只用于防止循环事件重复处理，不得让正常循环使当前克隆播放失效。
- 新命令和新状态应与旧 `VoiceCloneReplacementState` 并存，避免修改实时音频候选和现有兼容替换调用方。

## Validation Surface

- Python：`python -m unittest discover -s desktop/worker -p 'test*.py'`。
- Rust：在 `desktop/src-tauri` 执行 `cargo fmt --all -- --check`、`cargo clippy --all-targets -- -D warnings`、`cargo test --all-targets`。
- UI：在 `desktop/ui` 执行 `pnpm test`、`pnpm build`。
- 打包：在 `desktop/ui` 执行 `pnpm voice-models:test`、`pnpm voice-worker:test`、`pnpm tauri:build`。
- 人工：导入至少一个包含人声的 MP4，验证两轮循环、克隆结束恢复原声、切换文案、暂停/拖动/停止和更换视频。

## Constraints

- 不使用 NVIDIA 专用实现；首版 CPU/安装包内置资源路径必须可用。
- 不修改源 MP4，不生成视频版本队列，不上传视频或完整音频。
- 不把 React 自己的状态作为播放事实源；播放器状态以 Rust snapshot 和媒体事件为准。
- 不使用任意路径或 shell 拼接；Worker 只接收 Rust 创建的受控 JSON 和缓存路径。
