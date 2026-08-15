from __future__ import annotations

import tempfile
import unittest
from io import StringIO
import json
import os
from pathlib import Path
from unittest.mock import patch
import wave

import voice_clone_adapter as adapter


class VoiceCloneAdapterTest(unittest.TestCase):
    def test_worker_source_identity_uses_request_value_without_hashing_source_file(self) -> None:
        source_identity = "a" * 64

        with patch.object(adapter, "_sha256", side_effect=AssertionError("不应读取源视频计算内容哈希")):
            self.assertEqual(
                adapter._request_source_identity({"source_sha256": source_identity}),
                source_identity,
            )

    def test_worker_source_identity_rejects_missing_or_invalid_request_value(self) -> None:
        with self.assertRaisesRegex(ValueError, "源标识"):
            adapter._request_source_identity({})

        with self.assertRaisesRegex(ValueError, "源标识"):
            adapter._request_source_identity({"source_sha256": "not-a-sha256"})

    def test_demucs_command_uses_python_module_in_development_mode(self) -> None:
        executable = str(Path(tempfile.gettempdir()) / "python")
        command = adapter.build_demucs_command(
            ["--device", "cpu", "input.wav"],
            executable=executable,
            frozen=False,
        )

        self.assertEqual(
            command,
            [executable, "-m", "demucs.separate", "--device", "cpu", "input.wav"],
        )

    def test_demucs_command_uses_worker_dispatch_in_frozen_mode(self) -> None:
        executable = str(Path(tempfile.gettempdir()) / "autolive-voice-clone-worker.exe")
        command = adapter.build_demucs_command(
            ["--device", "cpu", "input.wav"],
            executable=executable,
            frozen=True,
        )

        self.assertEqual(
            command,
            [
                executable,
                adapter.FROZEN_DEMUCS_DISPATCH_ARG,
                "--device",
                "cpu",
                "input.wav",
            ],
        )

    def test_frozen_demucs_dispatch_passes_arguments_and_returns_exit_code(self) -> None:
        class FakeDemucsModule:
            @staticmethod
            def main(arguments: list[str]) -> int:
                self.assertEqual(arguments, ["--device", "cpu", "input.wav"])
                return 7

        with patch.object(adapter, "_import_optional_module", return_value=FakeDemucsModule):
            with patch.object(adapter.sys, "frozen", True, create=True):
                result = adapter.main(
                    [
                        adapter.FROZEN_DEMUCS_DISPATCH_ARG,
                        "--device",
                        "cpu",
                        "input.wav",
                    ]
                )

        self.assertEqual(result, 7)

    def test_capability_payload_is_unavailable_when_voice_dependencies_are_missing(self) -> None:
        real_import_module = adapter.importlib.import_module

        def fake_import_module(name: str, package: str | None = None):
            if name in {"demucs.separate", "faster_whisper", "TTS.api"}:
                raise ImportError(name)
            return real_import_module(name, package)

        with (
            patch.object(adapter.importlib, "import_module", side_effect=fake_import_module),
            patch.object(
                adapter.shutil,
                "which",
                side_effect=lambda name: str(Path(tempfile.gettempdir()) / name),
            ),
        ):
            payload = adapter.capabilities()

        self.assertFalse(payload["available"])
        self.assertEqual(payload["status"], "unavailable")
        self.assertIsNone(payload["provider"])
        self.assertIsNone(payload["model"])
        self.assertIsInstance(payload["reason"], str)
        self.assertIn("依赖", payload["reason"])

    def test_progress_payload_uses_expected_phase_names(self) -> None:
        payload = adapter.progress_payload(
            status="running",
            phase="checking-models",
            message="正在检查模型缓存。",
            percent=0,
        )

        self.assertEqual(
            payload,
            {
                "status": "running",
                "phase": "checking-models",
                "message": "正在检查模型缓存。",
                "percent": 0,
            },
        )

    def test_model_root_environment_overrides_cache_dirs(self) -> None:
        root = Path(tempfile.gettempdir()) / "autolive-model-root"
        huggingface_root = root / "huggingface"

        env = adapter.model_root_environment(root)

        self.assertEqual(env["TORCH_HOME"], str(root / "torch"))
        self.assertEqual(env["HF_HOME"], str(huggingface_root))
        self.assertEqual(env["TTS_HOME"], str(root / "tts"))

    def test_progress_payload_is_written_atomically(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            progress_path = Path(temp_dir) / "progress.json"
            payload = adapter.progress_payload(
                status="running",
                phase="downloading-models",
                message="正在下载模型。",
                percent=25,
            )

            with patch.object(Path, "replace", side_effect=RuntimeError("boom")):
                with self.assertRaises(RuntimeError):
                    adapter.emit_progress(progress_path, payload)

            self.assertFalse(progress_path.exists())
            self.assertFalse(progress_path.with_name("progress.json.partial").exists())

    def test_xtts_eof_is_converted_to_an_actionable_model_message(self) -> None:
        class FakeTtsModule:
            @staticmethod
            def TTS(**_kwargs):
                raise EOFError("EOF when reading a line")

        with self.assertRaisesRegex(RuntimeError, "XTTS-v2 模型资源不可用"):
            adapter.load_xtts_model(FakeTtsModule)

    def test_prepare_model_readiness_uses_the_same_xtts_loader_as_replacement(self) -> None:
        fake_model = object()

        class FakeTtsModule:
            pass

        with patch.object(adapter, "load_xtts_model", return_value=fake_model) as load_model:
            self.assertIs(adapter.ensure_xtts_model_ready(FakeTtsModule), fake_model)

        load_model.assert_called_once_with(FakeTtsModule)

    def test_xtts_model_is_reused_within_the_worker_process(self) -> None:
        fake_model = object()

        class FakeTtsModule:
            pass

        adapter.reset_xtts_model_cache()
        try:
            with patch.object(adapter, "load_xtts_model", return_value=fake_model) as load_model:
                self.assertIs(
                    adapter.ensure_xtts_model_ready(FakeTtsModule),
                    fake_model,
                )
                self.assertIs(
                    adapter.ensure_xtts_model_ready(FakeTtsModule),
                    fake_model,
                )

            load_model.assert_called_once_with(FakeTtsModule)
        finally:
            adapter.reset_xtts_model_cache()

    def test_xtts_model_cache_is_scoped_by_model_root_and_module(self) -> None:
        first_model = object()
        second_model = object()
        third_model = object()

        class FirstTtsModule:
            pass

        class SecondTtsModule:
            pass

        first_root = Path(tempfile.gettempdir()) / "autolive-model-root-one"
        second_root = Path(tempfile.gettempdir()) / "autolive-model-root-two"
        adapter.reset_xtts_model_cache()
        try:
            with patch.object(
                adapter,
                "load_xtts_model",
                side_effect=[first_model, second_model, third_model],
            ) as load_model:
                with patch.dict(os.environ, {adapter.MODEL_ROOT_ENV: str(first_root)}):
                    self.assertIs(
                        adapter.ensure_xtts_model_ready(FirstTtsModule),
                        first_model,
                    )
                    self.assertIs(
                        adapter.ensure_xtts_model_ready(FirstTtsModule),
                        first_model,
                    )
                with patch.dict(os.environ, {adapter.MODEL_ROOT_ENV: str(second_root)}):
                    self.assertIs(
                        adapter.ensure_xtts_model_ready(FirstTtsModule),
                        second_model,
                    )
                    self.assertIs(
                        adapter.ensure_xtts_model_ready(SecondTtsModule),
                        third_model,
                    )

            self.assertEqual(load_model.call_count, 3)
        finally:
            adapter.reset_xtts_model_cache()

    def test_xtts_progress_uses_local_model_message_before_first_load(self) -> None:
        fake_model = object()

        class FakeTtsModule:
            pass

        adapter.reset_xtts_model_cache()
        try:
            with patch.dict(
                os.environ,
                {adapter.MODEL_ROOT_ENV: str(Path(tempfile.gettempdir()) / "autolive-xtts")},
            ):
                with patch.object(adapter, "load_xtts_model", return_value=fake_model):
                    self.assertEqual(
                        adapter._xtts_model_progress(FakeTtsModule),
                        (
                            "loading-local-model",
                            "正在加载本地 XTTS-v2 模型（首次可能需要一些时间）",
                        ),
                    )
                    adapter.ensure_xtts_model_ready(FakeTtsModule)
                    self.assertEqual(
                        adapter._xtts_model_progress(FakeTtsModule),
                        ("reusing-model", "正在复用已加载的 XTTS-v2 模型"),
                    )
        finally:
            adapter.reset_xtts_model_cache()

    def test_xtts_progress_uses_download_message_without_model_root(self) -> None:
        class FakeTtsModule:
            pass

        adapter.reset_xtts_model_cache()
        try:
            with patch.dict(os.environ, {}, clear=True):
                self.assertEqual(
                    adapter._xtts_model_progress(FakeTtsModule),
                    ("downloading-models", "正在下载或加载 XTTS-v2 模型。"),
                )
        finally:
            adapter.reset_xtts_model_cache()

    def test_synthesize_text_generates_one_audio_clip_without_source_audio_join(self) -> None:
        class FakeTtsModel:
            def __init__(self) -> None:
                self.calls: list[dict[str, object]] = []

            def tts_to_file(self, **kwargs: object) -> None:
                self.calls.append(kwargs)
                file_path = Path(str(kwargs["file_path"]))
                file_path.parent.mkdir(parents=True, exist_ok=True)
                with wave.open(str(file_path), "wb") as wav_file:
                    wav_file.setnchannels(1)
                    wav_file.setsampwidth(2)
                    wav_file.setframerate(22_050)
                    wav_file.writeframes(b"\x00\x00" * 220)

        fake_model = FakeTtsModel()
        with tempfile.TemporaryDirectory() as temp_dir:
            output_json = Path(temp_dir) / "result.json"
            source_path = Path(temp_dir) / "source.bin"
            reference_audio_path = Path(temp_dir) / "reference.wav"
            source_path.write_bytes(b"source-data")
            with wave.open(str(reference_audio_path), "wb") as wav_file:
                wav_file.setnchannels(1)
                wav_file.setsampwidth(2)
                wav_file.setframerate(22_050)
                wav_file.writeframes(b"\x00\x00" * 220)
            request = {
                "operation_id": "current-text-operation",
                "source_generation": 7,
                "source_path": str(source_path),
                "source_sha256": adapter._sha256(source_path),
                "reference_audio_path": str(reference_audio_path),
                "reference_audio_sha256": adapter._sha256(reference_audio_path),
                "text": "当前文案",
                "sample_rate_hz": 22_050,
                "channel_count": 1,
                "model": adapter.DEFAULT_XTTS_MODEL,
            }

            with (
                patch.object(adapter, "capabilities", return_value={"available": True, "reason": None}),
                patch.object(adapter, "_get_xtts_module", return_value=object()),
                patch.object(adapter, "_resolve_executable", return_value=Path("/usr/bin/ffmpeg")),
                patch.object(
                    adapter,
                    "_probe_audio_stream",
                    return_value={"sample_rate_hz": 22_050, "channel_count": 1, "duration_ms": 1_000},
                ),
                patch.object(adapter, "ensure_xtts_model_ready", return_value=fake_model),
                patch.object(adapter, "_concat_source_with_clone") as concat_clone,
            ):
                result = adapter.synthesize_text(request, output_json)

        self.assertEqual(len(fake_model.calls), 1)
        self.assertEqual(fake_model.calls[0]["text"], "当前文案")
        self.assertNotIn("replace_at_ms", fake_model.calls[0])
        self.assertNotIn("resume_at_ms", fake_model.calls[0])
        self.assertNotIn("texts", fake_model.calls[0])
        concat_clone.assert_not_called()
        self.assertEqual(result["status"], "success")
        self.assertIn("audio_path", result)
        self.assertIn("audio_sha256", result)
        self.assertIn("duration_ms", result)
        self.assertEqual(result["operation_id"], "current-text-operation")

    def test_synthesize_text_reuses_cached_xtts_model(self) -> None:
        class FakeTtsModule:
            pass

        class FakeTtsModel:
            def __init__(self) -> None:
                self.calls = 0

            def tts_to_file(self, **kwargs: object) -> None:
                self.calls += 1
                file_path = Path(str(kwargs["file_path"]))
                file_path.parent.mkdir(parents=True, exist_ok=True)
                with wave.open(str(file_path), "wb") as wav_file:
                    wav_file.setnchannels(1)
                    wav_file.setsampwidth(2)
                    wav_file.setframerate(22_050)
                    wav_file.writeframes(b"\x00\x00" * 220)

        fake_model = FakeTtsModel()
        request = {
            "operation_id": "current-text-operation",
            "source_generation": 7,
            "source_path": "",
            "source_sha256": "",
            "reference_audio_path": "",
            "reference_audio_sha256": "",
            "text": "当前文案",
            "sample_rate_hz": 22_050,
            "channel_count": 1,
            "model": adapter.DEFAULT_XTTS_MODEL,
        }

        adapter.reset_xtts_model_cache()
        try:
            with tempfile.TemporaryDirectory() as temp_dir:
                source_path = Path(temp_dir) / "source.bin"
                reference_audio_path = Path(temp_dir) / "reference.wav"
                source_path.write_bytes(b"source-data")
                with wave.open(str(reference_audio_path), "wb") as wav_file:
                    wav_file.setnchannels(1)
                    wav_file.setsampwidth(2)
                    wav_file.setframerate(22_050)
                    wav_file.writeframes(b"\x00\x00" * 220)
                request.update(
                    {
                        "source_path": str(source_path),
                        "source_sha256": adapter._sha256(source_path),
                        "reference_audio_path": str(reference_audio_path),
                        "reference_audio_sha256": adapter._sha256(reference_audio_path),
                    }
                )
                output_one = Path(temp_dir) / "one.json"
                output_two = Path(temp_dir) / "two.json"
                with (
                    patch.object(adapter, "_get_xtts_module", return_value=FakeTtsModule),
                    patch.object(adapter, "load_xtts_model", return_value=fake_model) as load_model,
                    patch.object(adapter, "capabilities", return_value={"available": True, "reason": None}),
                    patch.object(adapter, "_resolve_executable", return_value=Path("/usr/bin/ffmpeg")),
                    patch.object(
                        adapter,
                        "_probe_audio_stream",
                        return_value={"sample_rate_hz": 22_050, "channel_count": 1, "duration_ms": 1_000},
                    ),
                    patch.object(adapter, "_concat_source_with_clone"),
                ):
                    adapter.synthesize_text(request, output_one)
                    adapter.synthesize_text(request, output_two)

            load_model.assert_called_once_with(FakeTtsModule)
            self.assertEqual(fake_model.calls, 2)
        finally:
            adapter.reset_xtts_model_cache()

    def test_synthesize_text_normalizes_output_to_prepared_source_format(self) -> None:
        class FakeTtsModel:
            def tts_to_file(self, **kwargs: object) -> None:
                file_path = Path(str(kwargs["file_path"]))
                file_path.parent.mkdir(parents=True, exist_ok=True)
                with wave.open(str(file_path), "wb") as wav_file:
                    wav_file.setnchannels(1)
                    wav_file.setsampwidth(2)
                    wav_file.setframerate(22_050)
                    wav_file.writeframes(b"\x00\x00" * 220)

        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            source_path = root / "source.mp4"
            reference_audio_path = root / "reference.wav"
            output_json = root / "result.json"
            source_path.write_bytes(b"source")
            with wave.open(str(reference_audio_path), "wb") as wav_file:
                wav_file.setnchannels(1)
                wav_file.setsampwidth(2)
                wav_file.setframerate(22_050)
                wav_file.writeframes(b"\x00\x00" * 220)
            request = {
                "operation_id": "normalize-operation",
                "source_generation": 2,
                "source_path": str(source_path),
                "source_sha256": adapter._sha256(source_path),
                "reference_audio_path": str(reference_audio_path),
                "reference_audio_sha256": adapter._sha256(reference_audio_path),
                "text": "当前文案",
                "sample_rate_hz": 48_000,
                "channel_count": 2,
            }

            def normalize_output(_ffmpeg: Path, _input: Path, output: Path, *, sample_rate_hz: int, channel_count: int) -> None:
                with wave.open(str(output), "wb") as wav_file:
                    wav_file.setnchannels(channel_count)
                    wav_file.setsampwidth(2)
                    wav_file.setframerate(sample_rate_hz)
                    wav_file.writeframes(b"\x00\x00\x00\x00" * 480)

            with (
                patch.object(adapter, "capabilities", return_value={"available": True, "reason": None}),
                patch.object(adapter, "_resolve_executable", return_value=Path("/usr/bin/ffmpeg")),
                patch.object(adapter, "_xtts_model_progress", return_value=("loading-local-model", "loading")),
                patch.object(adapter, "ensure_xtts_model_ready", return_value=FakeTtsModel()),
                patch.object(adapter, "_normalize_generated_audio", side_effect=normalize_output, create=True) as normalize,
            ):
                result = adapter.synthesize_text(request, output_json)

        normalize.assert_called_once()
        self.assertEqual(result["sample_rate_hz"], 48_000)
        self.assertEqual(result["channel_count"], 2)

    def test_warmup_model_loads_xtts_without_generating_audio(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            output_json = Path(temp_dir) / "warmup-result.json"
            with (
                patch.object(adapter, "capabilities", return_value={"available": True, "reason": None}),
                patch.object(adapter, "_xtts_model_progress", return_value=("loading-local-model", "loading")),
                patch.object(adapter, "ensure_xtts_model_ready") as ensure_model,
            ):
                result = adapter.warmup_model({}, output_json)

        ensure_model.assert_called_once_with()
        self.assertEqual(result["status"], "success")
        self.assertEqual(result["model"], adapter.DEFAULT_XTTS_MODEL)

    def test_warmup_model_reports_reusing_model_when_cache_is_ready(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            output_json = Path(temp_dir) / "warmup-result.json"
            with (
                patch.object(adapter, "capabilities", return_value={"available": True, "reason": None}),
                patch.object(
                    adapter,
                    "_xtts_model_progress",
                    return_value=("reusing-model", "正在复用已加载的 XTTS-v2 模型"),
                ) as model_progress,
                patch.object(adapter, "ensure_xtts_model_ready"),
                patch.object(adapter, "_safe_emit_progress") as emit_progress,
            ):
                result = adapter.warmup_model({}, output_json)

        model_progress.assert_called_once_with()
        emit_progress.assert_any_call(
            None,
            status="running",
            phase="reusing-model",
            message="正在复用已加载的 XTTS-v2 模型",
            percent=50,
        )
        self.assertEqual(result["status"], "success")

    def test_serve_executes_json_arg_arrays_sequentially_and_flushes_responses(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            first_output = Path(temp_dir) / "first.json"
            second_output = Path(temp_dir) / "second.json"
            input_stream = StringIO(
                json.dumps(
                    {"args": ["--capabilities-json", str(first_output)]},
                    ensure_ascii=False,
                )
                + "\n"
                + json.dumps(
                    {"args": ["--capabilities-json", str(second_output)]},
                    ensure_ascii=False,
                )
                + "\n"
            )
            output_stream = StringIO()

            with patch.object(
                adapter,
                "capabilities",
                return_value={"available": True, "status": "available"},
            ):
                result = adapter.serve(input_stream, output_stream)

            self.assertEqual(result, 0)
            self.assertEqual(first_output.read_text(encoding="utf-8"), second_output.read_text(encoding="utf-8"))
            responses = [json.loads(line) for line in output_stream.getvalue().splitlines()]
            self.assertEqual(len(responses), 2)
            self.assertTrue(all(response["status"] == "completed" for response in responses))
            self.assertTrue(all(response["exit_code"] == 0 for response in responses))

    def test_serve_returns_structured_failure_for_invalid_request_and_continues(self) -> None:
        output_stream = StringIO()
        input_stream = StringIO('{"args": "not-an-array"}\n' + '{broken\n')

        result = adapter.serve(input_stream, output_stream)

        self.assertEqual(result, 0)
        responses = [json.loads(line) for line in output_stream.getvalue().splitlines()]
        self.assertEqual(len(responses), 2)
        self.assertTrue(all(response["status"] == "failed" for response in responses))
        self.assertTrue(all(response["exit_code"] != 0 for response in responses))

    def test_serve_keeps_operation_stdout_out_of_the_protocol_stream(self) -> None:
        input_stream = StringIO('{"args": []}\n')
        output_stream = StringIO()
        error_stream = StringIO()

        def noisy_main(_arguments: list[str]) -> int:
            print("third-party log")
            return 0

        with patch.object(adapter, "main", side_effect=noisy_main):
            with patch.object(adapter.sys, "stderr", error_stream):
                self.assertEqual(adapter.serve(input_stream, output_stream), 0)

        responses = [json.loads(line) for line in output_stream.getvalue().splitlines()]
        self.assertEqual(responses, [{"status": "completed", "exit_code": 0}])
        self.assertIn("third-party log", error_stream.getvalue())

    def test_replacement_duration_ratio_allows_0_8_to_1_25(self) -> None:
        lower = adapter.plan_replacement_duration(8_000, 10_000)
        upper = adapter.plan_replacement_duration(12_500, 10_000)

        self.assertEqual(lower["mode"], "pad")
        self.assertEqual(lower["pad_ms"], 2_000)
        self.assertEqual(lower["remaining_ms"], 10_000)
        self.assertEqual(upper["mode"], "tempo")
        self.assertAlmostEqual(upper["tempo"], 1.25)
        self.assertEqual(upper["remaining_ms"], 10_000)

    def test_replacement_duration_ratio_rejects_values_outside_safe_range(self) -> None:
        with self.assertRaises(ValueError):
            adapter.plan_replacement_duration(12_501, 10_000)
        with self.assertRaises(ValueError):
            adapter.plan_replacement_duration(0, 10_000)

    def test_replacement_bounds_stay_inside_source_duration(self) -> None:
        adapter.validate_replacement_bounds(0, 10_000, 10_000)
        with self.assertRaises(ValueError):
            adapter.validate_replacement_bounds(-1, 1_000, 10_000)
        with self.assertRaises(ValueError):
            adapter.validate_replacement_bounds(1_000, 10_001, 10_000)

    def test_full_transcription_audio_is_not_limited_to_reference_window(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            target_path = Path(temp_dir) / "transcription.wav"
            with patch.object(adapter, "_run_checked") as run_checked:
                adapter._normalize_reference_audio(
                    Path(tempfile.gettempdir()) / "ffmpeg",
                    Path(temp_dir) / "vocals.wav",
                    target_path,
                    max_seconds=None,
                )

            command = run_checked.call_args.args[0]
            self.assertNotIn("-t", command)
            self.assertEqual(command[-1], str(target_path))

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

    def test_atomic_json_output_does_not_leave_partial_file(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            output_path = Path(temp_dir) / "result.json"
            with patch.object(Path, "replace", side_effect=RuntimeError("boom")):
                with self.assertRaises(RuntimeError):
                    adapter.write_atomic_json(output_path, {"status": "ok"})

            self.assertFalse(output_path.exists())
            self.assertFalse(output_path.with_name("result.json.partial").exists())

    def test_malformed_request_writes_structured_failure(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            request_path = Path(temp_dir) / "request.json"
            output_path = Path(temp_dir) / "result.json"
            request_path.write_text("{broken", encoding="utf-8")

            self.assertEqual(
                adapter.main(
                    [
                        "--replace-json",
                        str(request_path),
                        "--output-json",
                        str(output_path),
                    ]
                ),
                0,
            )
            payload = adapter._read_json(output_path)
            self.assertEqual(payload["status"], "failed")
            self.assertEqual(payload["operation_id"], "invalid-request")


if __name__ == "__main__":
    unittest.main()
