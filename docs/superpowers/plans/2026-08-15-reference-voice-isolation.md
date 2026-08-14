# 参考人声自动分离与片段选择 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 默认去除背景音乐，并从分离后的人声中自动选出最适合 XTTS-v2 的参考片段。

**Architecture:** 继续使用现有 Demucs 分离和 faster-whisper 转写流程。新增一个纯函数根据 Whisper 语音段选择最长 30 秒的人声窗口；`prepare_source` 先转写完整分离人声，再用 FFmpeg 从 `vocals.wav` 裁剪窗口生成 `reference.wav`。

**Tech Stack:** Python 3、FFmpeg、Demucs、faster-whisper、现有 unittest。

## Global Constraints

- 默认执行 Demucs `--two-stems=vocals`，不回退到原始混音。
- 参考音频最长 30 秒，完整 `transcription.wav` 不受该上限影响。
- 不新增依赖、不新增 UI 开关、不改视频播放链路。
- 所有外部命令继续使用参数数组并检查返回码。
- 新行为先写失败测试，再写生产代码。

---

### Task 1: 添加参考窗口选择的纯逻辑

**Files:**
- Modify: `/Users/mac/work/gepin/autoLive/desktop/worker/test_voice_clone_adapter.py`
- Modify: `/Users/mac/work/gepin/autoLive/desktop/worker/voice_clone_adapter.py`

**Interfaces:**
- Produces `select_reference_window(segments: list[JsonDict], max_duration_ms: int = DEFAULT_REFERENCE_MAX_SECONDS * 1000) -> JsonDict`，返回 `start_ms`、`duration_ms`、`speech_duration_ms`。

- [ ] **Step 1: Write the failing tests**

```python
    def test_reference_window_skips_music_intro_and_prefers_dense_speech(self) -> None:
        segments = [
            {"start_ms": 1_000, "end_ms": 2_000, "text": "片头"},
            {"start_ms": 40_000, "end_ms": 48_000, "text": "第一句"},
            {"start_ms": 49_000, "end_ms": 57_000, "text": "第二句"},
            {"start_ms": 58_000, "end_ms": 64_000, "text": "第三句"},
        ]

        window = adapter.select_reference_window(segments, max_duration_ms=30_000)

        self.assertEqual(window["start_ms"], 40_000)
        self.assertEqual(window["duration_ms"], 24_000)
        self.assertEqual(window["speech_duration_ms"], 22_000)

    def test_reference_window_rejects_empty_or_invalid_segments(self) -> None:
        with self.assertRaisesRegex(ValueError, "未检测到有效人声片段"):
            adapter.select_reference_window([])

        with self.assertRaisesRegex(ValueError, "未检测到有效人声片段"):
            adapter.select_reference_window([{"start_ms": 10, "end_ms": 10, "text": ""}])
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `python -m unittest desktop/worker/test_voice_clone_adapter.py -k reference_window`

Expected: FAIL because `select_reference_window` does not exist.

- [ ] **Step 3: Implement the minimal selector**

```python
def select_reference_window(
    segments: list[JsonDict],
    max_duration_ms: int = DEFAULT_REFERENCE_MAX_SECONDS * 1000,
) -> JsonDict:
    valid = sorted(
        (
            {"start_ms": int(segment["start_ms"]), "end_ms": int(segment["end_ms"])}
            for segment in segments
            if str(segment.get("text", "")).strip()
            and int(segment.get("end_ms", 0)) > int(segment.get("start_ms", 0))
        ),
        key=lambda segment: (segment["start_ms"], segment["end_ms"]),
    )
    if not valid or max_duration_ms <= 0:
        raise ValueError("未检测到有效人声片段")

    best: JsonDict | None = None
    for anchor_index, anchor in enumerate(valid):
        window_start = anchor["start_ms"]
        window_end_limit = window_start + max_duration_ms
        selected = [segment for segment in valid[anchor_index:] if segment["start_ms"] < window_end_limit]
        if not selected:
            continue
        window_end = min(window_end_limit, selected[-1]["end_ms"])
        speech_duration_ms = sum(
            max(0, min(segment["end_ms"], window_end) - max(segment["start_ms"], window_start))
            for segment in selected
        )
        candidate = {
            "start_ms": window_start,
            "duration_ms": max(1, window_end - window_start),
            "speech_duration_ms": speech_duration_ms,
        }
        if best is None or (
            candidate["speech_duration_ms"],
            candidate["speech_duration_ms"] / candidate["duration_ms"],
            -candidate["start_ms"],
        ) > (
            best["speech_duration_ms"],
            best["speech_duration_ms"] / best["duration_ms"],
            -best["start_ms"],
        ):
            best = candidate
    if best is None:
        raise ValueError("未检测到有效人声片段")
    return best
```

- [ ] **Step 4: Run the focused tests**

Run: `python -m unittest desktop/worker/test_voice_clone_adapter.py -k reference_window`

Expected: PASS.

### Task 2: 使用自动选择窗口生成参考音频

**Files:**
- Modify: `/Users/mac/work/gepin/autoLive/desktop/worker/test_voice_clone_adapter.py`
- Modify: `/Users/mac/work/gepin/autoLive/desktop/worker/voice_clone_adapter.py`

**Interfaces:**
- Extends `_normalize_reference_audio` with `start_seconds: float = 0` and uses `-ss` only when a selected window is provided.
- `prepare_source` keeps full `transcription.wav`, calls Whisper before creating `reference.wav`, and passes the selected window to FFmpeg.

- [ ] **Step 1: Add the failing command assertion**

```python
    def test_normalize_reference_audio_can_crop_selected_window(self) -> None:
        with patch.object(adapter, "_run_checked") as run_checked:
            adapter._normalize_reference_audio(
                Path("/tmp/ffmpeg"),
                Path("/tmp/vocals.wav"),
                Path("/tmp/reference.wav"),
                start_seconds=40.0,
                max_seconds=24,
            )

        command = run_checked.call_args.args[0]
        self.assertIn("-ss", command)
        self.assertEqual(command[command.index("-ss") + 1], "40.000000")
        self.assertEqual(command[command.index("-t") + 1], "24")
```

- [ ] **Step 2: Run the focused test to verify it fails**

Run: `python -m unittest desktop/worker/test_voice_clone_adapter.py -k crop_selected_window`

Expected: FAIL because `_normalize_reference_audio` does not accept `start_seconds`.

- [ ] **Step 3: Implement the smallest flow change**

Move the existing reference normalization below `_prepare_segments`, call `select_reference_window(segments)`, normalize only the selected window, and include progress text `正在分离人声并去除背景音乐。` during Demucs. Keep `transcription.wav` normalization with `max_seconds=None` before Whisper.

- [ ] **Step 4: Run the focused tests**

Run: `python -m unittest desktop/worker/test_voice_clone_adapter.py -k 'reference_window or crop_selected_window or full_transcription'`

Expected: PASS.

### Task 3: Full verification and cleanup

**Files:**
- Modify: `/Users/mac/work/gepin/autoLive/desktop/worker/test_voice_clone_adapter.py` only if a regression test exposes a real behavior gap.

- [ ] **Step 1: Run the complete Worker tests**

Run: `python -m unittest desktop/worker/test_voice_clone_adapter.py`

Expected: all tests pass with zero failures.

- [ ] **Step 2: Run static checks**

Run: `python -m py_compile desktop/worker/voice_clone_adapter.py desktop/worker/test_voice_clone_adapter.py && git diff --check`

Expected: exit code 0 and no whitespace errors.

- [ ] **Step 3: Inspect the diff for unused code**

Run: `git diff -- desktop/worker/voice_clone_adapter.py desktop/worker/test_voice_clone_adapter.py`

Expected: only the selector, crop parameters, prepare ordering, progress message, and their tests remain; no unused imports or dead helpers are introduced.
