from __future__ import annotations

import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import voice_clone_adapter as adapter


class VoiceCloneAdapterTest(unittest.TestCase):
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
