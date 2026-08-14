from __future__ import annotations

import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import voice_clone_adapter as adapter


class VoiceCloneAdapterTest(unittest.TestCase):
    def test_capability_payload_is_unavailable_when_voice_dependencies_are_missing(self) -> None:
        real_import_module = adapter.importlib.import_module

        def fake_import_module(name: str, package: str | None = None):
            if name in {"demucs.separate", "faster_whisper", "TTS.api"}:
                raise ImportError(name)
            return real_import_module(name, package)

        with (
            patch.object(adapter.importlib, "import_module", side_effect=fake_import_module),
            patch.object(adapter.shutil, "which", side_effect=lambda name: f"/usr/bin/{name}"),
        ):
            payload = adapter.capabilities()

        self.assertFalse(payload["available"])
        self.assertEqual(payload["status"], "unavailable")
        self.assertIsNone(payload["provider"])
        self.assertIsNone(payload["model"])
        self.assertIsInstance(payload["reason"], str)
        self.assertIn("依赖", payload["reason"])

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
                    Path("/usr/bin/ffmpeg"),
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
