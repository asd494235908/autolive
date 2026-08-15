# Phase 2：Worker 当前文案生成

Parent PRD: [当前文案克隆语音循环播放](../prd-voice-clone-current-playback.md)

Status: Not Started
Last Updated: 2026-08-14

## Objective

在现有常驻 Python Worker 中增加“单条当前文案 → 独立克隆音频片段”的操作，复用已经加载的 XTTS-v2，不再通过 `replace_current` 拼接完整源音轨。

## Context From Master PRD

- Goals covered: G-1、G-2、G-6
- Success Criteria: SC-1、SC-2、SC-7
- Requirements covered: FR-1、FR-2、FR-3、NFR-1、NFR-2、NFR-5

## Phase Discovery Gate

- [ ] 阅读 `desktop/worker/voice_clone_adapter.py` 的 JSON 请求解析、`ensure_xtts_model_ready`、音频归一化、进度写入和 `--serve` 分发。
- [ ] 阅读 `desktop/worker/test_voice_clone_adapter.py` 的模型缓存、EOF 错误和 Worker 协议测试。
- [ ] 核对当前 `requirements-voice-clone.txt`、模型资源目录和 `准备语音模型资源.mjs` 的正式包路径。
- [ ] 确认输出音频使用现有 FFmpeg/FFprobe 归一化方式，避免新增自定义音频编码器。

## Scope

### In Scope

- 新增一个只接收单条文本的 Worker 操作，例如 `synthesize_text`。
- 生成独立 WAV/PCM 音频并返回完整元数据。
- 复用常驻进程内 XTTS-v2 模型缓存。

### Out of Scope

- 不处理文案队列。
- 不把输出音频与原始音轨拼接。
- 不改变 `prepare_source` 的 Demucs/Whisper 流程和旧 `replace_current` 兼容入口。

## Implementation Checklist

- [ ] 在 `desktop/src-tauri/src/voice_clone.rs` 对应的 JSON 契约中固定单文本请求字段：`source_generation`、`source_path`、`source_sha256`、`reference_audio_path`、`reference_audio_sha256`、`text`、`sample_rate_hz`、`channel_count`、`operation_id`。
- [ ] 在 `desktop/worker/voice_clone_adapter.py` 增加 `synthesize_text(request, output_json)`，先校验文本非空、长度上限、参考音频存在、源代际字段完整，再调用 `ensure_xtts_model_ready` 和 XTTS-v2 `tts_to_file`。
- [ ] 输出只包含克隆音频片段路径，不写入源 MP4，不拼接前后原音；返回 `audio_sha256`、`duration_ms`、采样率、声道数、模型标识和输入身份。
- [ ] 在 `_serve_one_request` 增加新操作分发；保持逐行 JSON 协议、进度文件原子提交、取消检查和异常结构化返回。
- [ ] 对 XTTS-v2 首次加载和 EOF/模型资源缺失错误保持可行动中文提示；播放操作不得隐式联网下载模型。
- [ ] 在 `desktop/worker/test_voice_clone_adapter.py` 增加：单文本请求成功、空文本拒绝、输出不含原音拼接、模型缓存只加载一次、旧请求字段缺失、模型加载失败和取消/失败结果测试。

## Validation Strategy

使用 Python 单元测试和临时目录验证 Worker 输出契约。使用 fake TTS 模型替代真实模型，避免测试依赖 GPU、网络或完整模型资源。

## Validation Checklist

- [ ] `python -m unittest discover -s desktop/worker -p 'test*.py'`
- [ ] 使用 fake XTTS 模型验证新操作只接收一条字符串。
- [ ] 验证输出 JSON 的 SHA、时长、采样率和声道字段可被 Rust 校验。
- [ ] 验证模型缓存命中时不会重复调用加载器。
- [ ] 验证 Worker 进程仍能处理旧 `prepare_source` 和 `replace_current` 请求。

## Exit Criteria

- [ ] 新 Worker 操作能生成单条当前文案的独立音频片段。
- [ ] 相同 Worker 进程内 XTTS-v2 只加载一次。
- [ ] 所有输出可校验，失败不会伪造成功结果。
- [ ] 旧 Worker 协议测试保持通过。

## Phase-End Multi-Pass Review

- [ ] 1. 只生成当前文案，没有数组、队列或隐式批处理。
- [ ] 2. 失败、取消、EOF、超时和模型缺失均有结构化错误。
- [ ] 3. Worker 不承担播放器静音、循环和音轨恢复责任。
- [ ] 4. 使用已有成熟音频处理链，不新增重复工具。
- [ ] 5. 删除死代码、调试输出和未使用依赖。
- [ ] 6. 输入路径来自 Rust 受控请求，输出路径在受控缓存目录。
- [ ] 7. 长文本、长音频和并发请求有现有上限和取消路径。
- [ ] 8. fake 模型测试覆盖关键错误分支。
- [ ] 9. Rust 契约字段与 Python 输出字段完全一致。
- [ ] 10. 主 PRD 和 Phase 3 的接口说明已同步。
