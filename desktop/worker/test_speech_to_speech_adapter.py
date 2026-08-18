from __future__ import annotations

import os
import sys
import tempfile
import types
import unittest
from pathlib import Path
from unittest.mock import patch

import speech_to_speech_adapter as adapter


class SpeechToSpeechAdapterTest(unittest.TestCase):
    def test_capabilities_prefers_explicit_ffmpeg_path_over_path_lookup(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            ffmpeg_path = Path(temp_dir) / "ffmpeg"
            ffmpeg_path.write_bytes(b"ffmpeg")
            ffmpeg_path.chmod(0o755)

            with (
                patch.dict(os.environ, {"AUTOLIVE_FFMPEG_PATH": str(ffmpeg_path)}, clear=False),
                patch.dict(sys.modules, {"websockets": types.ModuleType("websockets")}),
                patch.object(adapter.shutil, "which", return_value=None) as which,
            ):
                payload = adapter.capabilities()

        self.assertTrue(payload["available"])
        which.assert_not_called()

    def test_extract_pcm_uses_explicit_ffmpeg_path(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            ffmpeg_path = root / "ffmpeg"
            source_path = root / "source.wav"
            ffmpeg_path.write_bytes(b"ffmpeg")
            ffmpeg_path.chmod(0o755)
            source_path.write_bytes(b"source")
            captured: list[list[str]] = []
            captured_options: list[dict[str, object]] = []

            def fake_run(command, **kwargs):
                captured.append(command)
                captured_options.append(kwargs)
                return types.SimpleNamespace(returncode=0, stdout=b"pcm", stderr=b"")

            context = {
                "audio_path_or_stream_ref": str(source_path),
                "start_at_ms": 0,
                "target_duration_ms": 1_000,
            }
            with (
                patch.dict(os.environ, {"AUTOLIVE_FFMPEG_PATH": str(ffmpeg_path)}, clear=False),
                patch.object(adapter.subprocess, "run", side_effect=fake_run),
            ):
                self.assertEqual(adapter.extract_pcm(context), b"pcm")

        self.assertEqual(captured[0][0], str(ffmpeg_path.resolve()))
        self.assertEqual(
            captured_options[0]["creationflags"],
            int(getattr(adapter.subprocess, "CREATE_NO_WINDOW", 0)),
        )

    def test_normalize_audio_uses_explicit_ffmpeg_path(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            ffmpeg_path = Path(temp_dir) / "ffmpeg"
            ffmpeg_path.write_bytes(b"ffmpeg")
            ffmpeg_path.chmod(0o755)
            captured: list[list[str]] = []
            captured_options: list[dict[str, object]] = []

            def fake_run(command, **kwargs):
                captured.append(command)
                captured_options.append(kwargs)
                return types.SimpleNamespace(returncode=0, stdout=b"wav", stderr=b"")

            with (
                patch.dict(os.environ, {"AUTOLIVE_FFMPEG_PATH": str(ffmpeg_path)}, clear=False),
                patch.object(adapter.subprocess, "run", side_effect=fake_run),
            ):
                self.assertEqual(
                    adapter.normalize_audio(b"\x00\x00" * 16_000, 16_000, 24_000, 1, 1_000),
                    b"wav",
                )

        self.assertEqual(captured[0][0], str(ffmpeg_path.resolve()))
        self.assertEqual(
            captured_options[0]["creationflags"],
            int(getattr(adapter.subprocess, "CREATE_NO_WINDOW", 0)),
        )


if __name__ == "__main__":
    unittest.main()
