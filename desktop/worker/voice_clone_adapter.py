#!/usr/bin/env python3
"""本地 CPU 人声克隆 Worker 适配器。

这个脚本只负责本地编排：
- 能力探测
- 源音频准备
- 参考音色克隆
- 当前话术热替换

它不访问 Go API，不读任何秘密，也不在 shell 里拼接命令字符串。
"""

from __future__ import annotations

import argparse
import contextlib
import hashlib
import importlib
import json
import os
import shutil
import subprocess
import sys
import threading
import uuid
import wave
from pathlib import Path
from typing import Any


VOICE_CLONE_PROVIDER = "local-cpu-voice-clone"
DEFAULT_XTTS_MODEL = "tts_models/multilingual/multi-dataset/xtts_v2"
DEFAULT_WHISPER_MODEL_SIZE = "small"
DEFAULT_REFERENCE_LANGUAGE = "zh-cn"
DEFAULT_REFERENCE_SAMPLE_RATE_HZ = 22_050
DEFAULT_REFERENCE_MAX_SECONDS = 30
MAX_VOICE_CLONE_TEXT_CHARS = 500
MIN_VOICE_SEGMENT_DURATION_MS = 200

FFMPEG_PATH_ENV = "AUTOLIVE_FFMPEG_PATH"
FFPROBE_PATH_ENV = "AUTOLIVE_FFPROBE_PATH"
MODEL_ROOT_ENV = "AUTOLIVE_VOICE_CLONE_MODEL_ROOT"
FROZEN_DEMUCS_DISPATCH_ARG = "--autolive-run-demucs"
DEFAULT_SUBPROCESS_TIMEOUT_SECONDS = 30 * 60


_XTTS_MODEL_CACHE: dict[tuple[str | None, Any], Any] = {}
_XTTS_MODEL_CACHE_LOCK = threading.RLock()


JsonDict = dict[str, Any]


def progress_payload(*, status: str, phase: str, message: str, percent: int) -> JsonDict:
    if status not in {"running", "success", "failed"}:
        raise ValueError("进度状态无效")
    if not phase.strip() or not message.strip():
        raise ValueError("进度阶段和提示不能为空")
    if percent < 0 or percent > 100:
        raise ValueError("进度百分比必须在 0 到 100 之间")
    return {
        "status": status,
        "phase": phase,
        "message": message,
        "percent": percent,
    }


def emit_progress(path: Path, payload: JsonDict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    partial = path.with_name(f"{path.name}.partial")
    try:
        partial.write_text(json.dumps(payload, ensure_ascii=False), encoding="utf-8")
        partial.replace(path)
    except Exception:
        partial.unlink(missing_ok=True)
        raise


def model_root_environment(root: Path) -> dict[str, str]:
    root = root.expanduser()
    huggingface_root = root / "huggingface"
    return {
        MODEL_ROOT_ENV: str(root),
        "TORCH_HOME": str(root / "torch"),
        "HF_HOME": str(huggingface_root),
        "HF_HUB_CACHE": str(huggingface_root / "hub"),
        "TTS_HOME": str(root / "tts"),
    }


def _configured_model_root() -> Path | None:
    configured = os.environ.get(MODEL_ROOT_ENV, "").strip()
    if not configured:
        return None
    path = Path(configured).expanduser()
    if not path.is_absolute():
        raise ValueError(f"{MODEL_ROOT_ENV} 必须是绝对路径")
    os.environ.update(model_root_environment(path))
    return path


def _optional_absolute_path(path_value: Any, field_name: str) -> Path | None:
    if path_value is None or not str(path_value).strip():
        return None
    path = Path(str(path_value)).expanduser()
    if not path.is_absolute():
        raise ValueError(f"{field_name} 必须是绝对路径")
    return path


def _safe_emit_progress(
    path: Path | None,
    *,
    status: str,
    phase: str,
    message: str,
    percent: int,
) -> None:
    if path is None:
        return
    try:
        emit_progress(
            path,
            progress_payload(status=status, phase=phase, message=message, percent=percent),
        )
    except OSError:
        # 进度文件是可选的观测通道，不能让它的磁盘错误中断媒体处理。
        return


def _safe_token(value: str) -> str:
    token = "".join(character if character.isalnum() or character in "-_." else "_" for character in value.strip())
    return token.strip("._") or uuid.uuid4().hex


def _read_json(path: Path) -> JsonDict:
    return json.loads(path.read_text(encoding="utf-8"))


def write_atomic_json(path: Path, value: JsonDict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    partial = path.with_name(f"{path.name}.partial")
    if path.exists():
        raise FileExistsError(path)
    try:
        partial.write_text(json.dumps(value, ensure_ascii=False), encoding="utf-8")
        partial.replace(path)
    except Exception:
        partial.unlink(missing_ok=True)
        raise


def _resolve_executable(env_name: str, default_name: str) -> Path | None:
    configured = os.environ.get(env_name, "").strip()
    if configured:
        candidate = Path(configured).expanduser()
        return candidate if candidate.is_file() and os.access(candidate, os.X_OK) else None
    found = shutil.which(default_name)
    if not found:
        return None
    return Path(found)


def _import_optional_module(module_name: str) -> Any:
    return importlib.import_module(module_name)


def _dependency_reason() -> str | None:
    missing: list[str] = []
    for module_name in ("demucs.separate", "faster_whisper", "TTS.api"):
        try:
            _import_optional_module(module_name)
        except ImportError:
            missing.append(module_name)
    if missing:
        return f"未安装 voice clone 依赖：{', '.join(missing)}"
    return None


def capabilities() -> JsonDict:
    try:
        _configured_model_root()
        dependency_reason = _dependency_reason()
        if dependency_reason is not None:
            return {
                "available": False,
                "status": "unavailable",
                "provider": None,
                "model": None,
                "reason": dependency_reason,
            }

        ffmpeg_path = _resolve_executable(FFMPEG_PATH_ENV, "ffmpeg")
        if ffmpeg_path is None:
            return {
                "available": False,
                "status": "unavailable",
                "provider": None,
                "model": None,
                "reason": f"未找到 FFmpeg，可通过 {FFMPEG_PATH_ENV} 指定绝对路径",
            }

        ffprobe_path = _resolve_executable(FFPROBE_PATH_ENV, "ffprobe")
        if ffprobe_path is None:
            return {
                "available": False,
                "status": "unavailable",
                "provider": None,
                "model": None,
                "reason": f"未找到 FFprobe，可通过 {FFPROBE_PATH_ENV} 指定绝对路径",
            }

        return {
            "available": True,
            "status": "available",
            "provider": VOICE_CLONE_PROVIDER,
            "model": DEFAULT_XTTS_MODEL,
            "reason": None,
            "ffmpeg_path": str(ffmpeg_path),
            "ffprobe_path": str(ffprobe_path),
        }
    except Exception as error:  # noqa: BLE001 - 能力探测必须降级为结构化失败
        return {
            "available": False,
            "status": "unavailable",
            "provider": None,
            "model": None,
            "reason": str(error),
        }


def plan_replacement_duration(generated_duration_ms: int, remaining_ms: int) -> JsonDict:
    if generated_duration_ms <= 0:
        raise ValueError("生成音频时长必须大于 0")
    if remaining_ms <= 0:
        raise ValueError("剩余时长必须大于 0")

    if generated_duration_ms < remaining_ms:
        return {
            "mode": "pad",
            "generated_duration_ms": generated_duration_ms,
            "remaining_ms": remaining_ms,
            "pad_ms": remaining_ms - generated_duration_ms,
            "tempo": None,
            "target_duration_ms": remaining_ms,
        }

    if generated_duration_ms == remaining_ms:
        return {
            "mode": "exact",
            "generated_duration_ms": generated_duration_ms,
            "remaining_ms": remaining_ms,
            "pad_ms": 0,
            "tempo": None,
            "target_duration_ms": remaining_ms,
        }

    ratio = generated_duration_ms / remaining_ms
    if ratio < 0.8 or ratio > 1.25:
        raise ValueError("生成音频与目标时长比例不在安全范围 0.8–1.25")
    return {
        "mode": "tempo",
        "generated_duration_ms": generated_duration_ms,
        "remaining_ms": remaining_ms,
        "pad_ms": 0,
        "tempo": ratio,
        "target_duration_ms": remaining_ms,
    }


def _sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _request_source_identity(request: JsonDict) -> str:
    value = str(request.get("source_sha256") or "").strip().lower()
    if len(value) != 64 or not all(character in "0123456789abcdef" for character in value):
        raise ValueError("请求缺少有效源标识")
    return value


def _probe_audio_stream(ffprobe_path: Path, source_path: Path) -> dict[str, int]:
    command = [
        str(ffprobe_path),
        "-v",
        "error",
        "-select_streams",
        "a:0",
        "-show_entries",
        "stream=sample_rate,channels,duration",
        "-show_entries",
        "format=duration",
        "-of",
        "json",
        str(source_path),
    ]
    completed = subprocess.run(
        command,
        check=False,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    if completed.returncode != 0 or not completed.stdout:
        raise RuntimeError(completed.stderr.decode("utf-8", errors="replace") or "ffprobe 读取源音频失败")
    payload = json.loads(completed.stdout.decode("utf-8"))
    streams = payload.get("streams") or []
    if not streams:
        raise RuntimeError("源媒体缺少可用音频流")
    stream = streams[0]
    sample_rate_hz = int(stream.get("sample_rate") or 0)
    channel_count = int(stream.get("channels") or 0)
    duration_seconds = float(stream.get("duration") or payload.get("format", {}).get("duration") or 0.0)
    if sample_rate_hz <= 0 or channel_count <= 0 or duration_seconds <= 0:
        raise RuntimeError("源媒体音频信息无效")
    return {
        "sample_rate_hz": sample_rate_hz,
        "channel_count": channel_count,
        "duration_ms": max(1, round(duration_seconds * 1000)),
    }


def _run_checked(command: list[str], *, timeout_seconds: int = DEFAULT_SUBPROCESS_TIMEOUT_SECONDS) -> None:
    completed = subprocess.run(
        command,
        check=False,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        timeout=timeout_seconds,
    )
    if completed.returncode != 0:
        stderr = completed.stderr.decode("utf-8", errors="replace").strip()
        message = stderr or "外部命令执行失败"
        raise RuntimeError(message)


def _extract_source_audio(ffmpeg_path: Path, source_path: Path, target_path: Path) -> None:
    command = [
        str(ffmpeg_path),
        "-hide_banner",
        "-loglevel",
        "error",
        "-i",
        str(source_path),
        "-vn",
        "-map",
        "0:a:0",
        "-ac",
        "1",
        "-ar",
        str(DEFAULT_REFERENCE_SAMPLE_RATE_HZ),
        "-f",
        "wav",
        str(target_path),
    ]
    _run_checked(command)


def _find_vocals_path(demucs_output_dir: Path) -> Path:
    for candidate in sorted(demucs_output_dir.rglob("vocals.wav")):
        if candidate.is_file():
            return candidate
    raise FileNotFoundError("未找到 Demucs 输出的 vocals.wav")


def _normalize_reference_audio(
    ffmpeg_path: Path,
    vocals_path: Path,
    target_path: Path,
    *,
    sample_rate_hz: int = DEFAULT_REFERENCE_SAMPLE_RATE_HZ,
    start_seconds: float = 0.0,
    max_seconds: float | None = DEFAULT_REFERENCE_MAX_SECONDS,
) -> None:
    if start_seconds < 0:
        raise ValueError("参考人声起始时间不能小于 0 秒")
    command = [
        str(ffmpeg_path),
        "-hide_banner",
        "-loglevel",
        "error",
        "-i",
        str(vocals_path),
    ]
    if start_seconds > 0:
        command.extend(["-ss", f"{start_seconds:.6f}"])
    command.extend([
        "-vn",
        "-ac",
        "1",
        "-ar",
        str(sample_rate_hz),
    ])
    if max_seconds is not None:
        if max_seconds <= 0:
            raise ValueError("参考人声时长必须大于 0 秒")
        formatted_seconds = f"{max_seconds:.6f}".rstrip("0").rstrip(".")
        command.extend(["-t", formatted_seconds])
    command.extend([
        "-f",
        "wav",
        str(target_path),
    ])
    _run_checked(command)


def _read_wav_duration_ms(path: Path) -> int:
    with wave.open(str(path), "rb") as wav_file:
        frames = wav_file.getnframes()
        rate = wav_file.getframerate()
    if rate <= 0:
        raise RuntimeError("WAV 采样率无效")
    return max(1, round(frames / rate * 1000))


def _read_wav_info(path: Path) -> JsonDict:
    with wave.open(str(path), "rb") as wav_file:
        frames = wav_file.getnframes()
        sample_rate_hz = wav_file.getframerate()
        channel_count = wav_file.getnchannels()
    if sample_rate_hz <= 0 or channel_count <= 0:
        raise RuntimeError("WAV 音频信息无效")
    return {
        "duration_ms": max(1, round(frames / sample_rate_hz * 1000)),
        "sample_rate_hz": sample_rate_hz,
        "channel_count": channel_count,
    }


def _validate_text(text: Any) -> str:
    value = str(text or "").strip()
    if not value:
        raise ValueError("替换文本不能为空")
    if len(value) > MAX_VOICE_CLONE_TEXT_CHARS:
        raise ValueError("替换文本超过 500 字限制")
    return value


def _text_sha256(text: str) -> str:
    digest = hashlib.sha256()
    digest.update(text.encode("utf-8"))
    return digest.hexdigest()


def _read_existing_file(path_value: Any, field_name: str) -> Path:
    path = Path(str(path_value)).expanduser()
    if not path.is_absolute():
        raise ValueError(f"{field_name} 必须是绝对路径")
    if not path.is_file():
        raise FileNotFoundError(path)
    return path


def _output_path(base_path: Path, operation_id: str, suffix: str) -> Path:
    operation_dir = base_path.parent / _safe_token(operation_id)
    operation_dir.mkdir(parents=True, exist_ok=True)
    return operation_dir / suffix


def _result_base(operation_id: str, status: str, model: str | None = None, reason: str | None = None) -> JsonDict:
    return {
        "operation_id": operation_id,
        "status": status,
        "provider": VOICE_CLONE_PROVIDER,
        "model": model,
        "reason": reason,
    }


def _failure_result(operation_id: str, reason: str, *, model: str | None = None) -> JsonDict:
    result = _result_base(operation_id, "failed", model=model, reason=reason)
    return result


def _prepare_segments(
    whisper_model: Any,
    reference_audio_path: Path,
    *,
    language: str,
) -> list[JsonDict]:
    segments, _info = whisper_model.transcribe(
        str(reference_audio_path),
        language=language,
        vad_filter=True,
        word_timestamps=False,
    )
    prepared: list[JsonDict] = []
    for segment in segments:
        text = str(getattr(segment, "text", "")).strip()
        start_ms = max(0, round(float(getattr(segment, "start", 0.0)) * 1000))
        end_ms = max(0, round(float(getattr(segment, "end", 0.0)) * 1000))
        if not text or end_ms - start_ms < MIN_VOICE_SEGMENT_DURATION_MS:
            continue
        prepared.append({"start_ms": start_ms, "end_ms": end_ms, "text": text})
    return prepared


def select_reference_window(
    segments: list[JsonDict],
    max_duration_ms: int = DEFAULT_REFERENCE_MAX_SECONDS * 1000,
) -> JsonDict:
    if max_duration_ms <= 0:
        raise ValueError("未检测到有效人声片段")

    valid_segments: list[JsonDict] = []
    for segment in segments:
        try:
            start_ms = int(segment.get("start_ms", 0))
            end_ms = int(segment.get("end_ms", 0))
        except (TypeError, ValueError):
            continue
        if str(segment.get("text", "")).strip() and end_ms > start_ms:
            valid_segments.append({"start_ms": start_ms, "end_ms": end_ms})

    valid_segments.sort(key=lambda segment: (segment["start_ms"], segment["end_ms"]))
    if not valid_segments:
        raise ValueError("未检测到有效人声片段")

    best_window: JsonDict | None = None
    # ponytail: O(n²) 候选扫描；Whisper 片段数量很小，只有长视频实测成为瓶颈时再改滑动窗口。
    for anchor_index, anchor in enumerate(valid_segments):
        window_start = anchor["start_ms"]
        window_end_limit = window_start + max_duration_ms
        selected_segments = [
            segment
            for segment in valid_segments[anchor_index:]
            if segment["start_ms"] < window_end_limit
        ]
        if not selected_segments:
            continue

        window_end = min(window_end_limit, selected_segments[-1]["end_ms"])
        speech_duration_ms = sum(
            max(
                0,
                min(segment["end_ms"], window_end)
                - max(segment["start_ms"], window_start),
            )
            for segment in selected_segments
        )
        candidate = {
            "start_ms": window_start,
            "duration_ms": max(1, window_end - window_start),
            "speech_duration_ms": speech_duration_ms,
        }
        if best_window is None or (
            candidate["speech_duration_ms"],
            candidate["speech_duration_ms"] / candidate["duration_ms"],
            -candidate["start_ms"],
        ) > (
            best_window["speech_duration_ms"],
            best_window["speech_duration_ms"] / best_window["duration_ms"],
            -best_window["start_ms"],
        ):
            best_window = candidate

    if best_window is None:
        raise ValueError("未检测到有效人声片段")
    return best_window


def build_demucs_command(
    demucs_args: list[str],
    *,
    executable: str | None = None,
    frozen: bool | None = None,
) -> list[str]:
    worker_executable = sys.executable if executable is None else executable
    frozen_mode = bool(getattr(sys, "frozen", False)) if frozen is None else frozen
    entrypoint = (
        [FROZEN_DEMUCS_DISPATCH_ARG]
        if frozen_mode
        else ["-m", "demucs.separate"]
    )
    return [worker_executable, *entrypoint, *demucs_args]


def _dispatch_frozen_demucs(demucs_args: list[str]) -> int:
    demucs_module = _import_optional_module("demucs.separate")
    try:
        result = demucs_module.main(demucs_args)
    except SystemExit as error:
        return error.code if isinstance(error.code, int) else 0 if error.code is None else 1
    return result if isinstance(result, int) else 0


def prepare_source(request: JsonDict, output_json: Path) -> JsonDict:
    operation_id = str(request.get("operation_id") or uuid.uuid4().hex)
    progress_path: Path | None = None
    try:
        progress_path = _optional_absolute_path(request.get("progress_path"), "progress_path")
        _configured_model_root()
        _safe_emit_progress(
            progress_path,
            status="running",
            phase="checking-models",
            message="正在检查模型缓存。",
            percent=0,
        )
        capability = capabilities()
        if not capability["available"]:
            reason = capability["reason"] or "本地 voice clone 能力不可用"
            _safe_emit_progress(
                progress_path,
                status="failed",
                phase="failed",
                message=reason,
                percent=100,
            )
            return _failure_result(operation_id, reason, model=DEFAULT_XTTS_MODEL)

        source_path = _read_existing_file(request.get("source_path"), "source_path")
        ffmpeg_path = _resolve_executable(FFMPEG_PATH_ENV, "ffmpeg")
        ffprobe_path = _resolve_executable(FFPROBE_PATH_ENV, "ffprobe")
        if ffmpeg_path is None or ffprobe_path is None:
            raise RuntimeError("FFmpeg/FFprobe 不可用")

        source_sha256 = _request_source_identity(request)
        source_info = _probe_audio_stream(ffprobe_path, source_path)
        operation_dir = _output_path(output_json, operation_id, "reference.wav").parent
        source_audio_path = operation_dir / "source-audio.wav"
        demucs_output_dir = operation_dir / "demucs"
        reference_audio_path = operation_dir / "reference.wav"
        model_size = str(request.get("model_size") or DEFAULT_WHISPER_MODEL_SIZE)
        language = str(request.get("language") or "zh")

        _safe_emit_progress(
            progress_path,
            status="running",
            phase="downloading-models",
            message="正在下载或加载 Demucs 模型。",
            percent=10,
        )
        _extract_source_audio(ffmpeg_path, source_path, source_audio_path)

        _import_optional_module("demucs.separate")
        demucs_args = [
            "--device",
            "cpu",
            "--two-stems=vocals",
            "-o",
            str(demucs_output_dir),
            str(source_audio_path),
        ]
        demucs_model_name = str(request.get("demucs_model") or "htdemucs").strip() or "htdemucs"
        demucs_args[demucs_args.index("-o"):demucs_args.index("-o")] = ["-n", demucs_model_name]
        demucs_executable = build_demucs_command(demucs_args)
        _safe_emit_progress(
            progress_path,
            status="running",
            phase="separating-voice",
            message="正在分离人声并去除背景音乐。",
            percent=35,
        )
        _run_checked(demucs_executable)

        vocals_path = _find_vocals_path(demucs_output_dir)
        index_audio_path = operation_dir / "transcription.wav"
        _normalize_reference_audio(
            ffmpeg_path,
            vocals_path,
            index_audio_path,
            max_seconds=None,
        )

        _safe_emit_progress(
            progress_path,
            status="running",
            phase="downloading-models",
            message="正在下载或加载 Whisper 模型。",
            percent=55,
        )
        faster_whisper_module = _import_optional_module("faster_whisper")
        whisper_model = faster_whisper_module.WhisperModel(
            model_size,
            device="cpu",
            compute_type="int8",
        )
        _safe_emit_progress(
            progress_path,
            status="running",
            phase="transcribing",
            message="正在识别人声话术。",
            percent=75,
        )
        segments = _prepare_segments(whisper_model, index_audio_path, language=language)
        if not segments:
            raise RuntimeError("未检测到有效人声片段")
        reference_window = select_reference_window(segments)
        _normalize_reference_audio(
            ffmpeg_path,
            vocals_path,
            reference_audio_path,
            start_seconds=reference_window["start_ms"] / 1000,
            max_seconds=reference_window["duration_ms"] / 1000,
        )
        reference_sha256 = _sha256(reference_audio_path)

        model_phase, model_message = _xtts_model_progress()
        _safe_emit_progress(
            progress_path,
            status="running",
            phase=model_phase,
            message=model_message,
            percent=90,
        )
        ensure_xtts_model_ready()

        result = _result_base(operation_id, "success", model=f"demucs+{model_size}+xtts_v2")
        result.update(
            {
                "source_path": str(source_path),
                "source_sha256": source_sha256,
                "reference_audio_path": str(reference_audio_path),
                "reference_audio_sha256": reference_sha256,
                "sample_rate_hz": source_info["sample_rate_hz"],
                "channel_count": source_info["channel_count"],
                "duration_ms": source_info["duration_ms"],
                "segments": segments,
            }
        )
        _safe_emit_progress(
            progress_path,
            status="success",
            phase="ready",
            message="人声准备完成。",
            percent=100,
        )
        return result
    except Exception as error:  # noqa: BLE001 - Worker 必须返回结构化失败
        reason = str(error)
        _safe_emit_progress(
            progress_path,
            status="failed",
            phase="failed",
            message=reason,
            percent=100,
        )
        return _failure_result(operation_id, reason, model=DEFAULT_XTTS_MODEL)


def _adjust_cloned_audio(
    ffmpeg_path: Path,
    input_path: Path,
    output_path: Path,
    *,
    sample_rate_hz: int,
    channel_count: int,
    plan: JsonDict,
) -> None:
    filters: list[str] = []
    mode = str(plan["mode"])
    if mode == "pad":
        filters.append(f"apad=whole_dur={plan['target_duration_ms'] / 1000:.6f}")
        filters.append(f"atrim=duration={plan['target_duration_ms'] / 1000:.6f}")
    elif mode == "tempo":
        filters.append(f"atempo={float(plan['tempo']):.6f}")
        filters.append(f"atrim=duration={plan['target_duration_ms'] / 1000:.6f}")
    else:
        filters.append(f"atrim=duration={plan['target_duration_ms'] / 1000:.6f}")

    command = [
        str(ffmpeg_path),
        "-hide_banner",
        "-loglevel",
        "error",
        "-i",
        str(input_path),
        "-af",
        ",".join(filters),
        "-ar",
        str(sample_rate_hz),
        "-ac",
        str(channel_count),
        "-f",
        "wav",
        str(output_path),
    ]
    _run_checked(command)


def _normalize_generated_audio(
    ffmpeg_path: Path,
    input_path: Path,
    output_path: Path,
    *,
    sample_rate_hz: int,
    channel_count: int,
) -> None:
    command = [
        str(ffmpeg_path),
        "-hide_banner",
        "-loglevel",
        "error",
        "-i",
        str(input_path),
        "-ar",
        str(sample_rate_hz),
        "-ac",
        str(channel_count),
        "-f",
        "wav",
        str(output_path),
    ]
    _run_checked(command)


def _concat_source_with_clone(
    ffmpeg_path: Path,
    source_path: Path,
    clone_path: Path,
    output_path: Path,
    *,
    replace_at_ms: int,
    resume_at_ms: int,
    sample_rate_hz: int,
    channel_count: int,
) -> None:
    replace_seconds = replace_at_ms / 1000
    resume_seconds = resume_at_ms / 1000
    command = [
        str(ffmpeg_path),
        "-hide_banner",
        "-loglevel",
        "error",
        "-i",
        str(source_path),
        "-i",
        str(clone_path),
        "-filter_complex",
        (
            f"[0:a]atrim=0:{replace_seconds:.6f},asetpts=PTS-STARTPTS[pre];"
            f"[0:a]atrim=start={resume_seconds:.6f},asetpts=PTS-STARTPTS[post];"
            f"[pre][1:a][post]concat=n=3:v=0:a=1[out]"
        ),
        "-map",
        "[out]",
        "-ar",
        str(sample_rate_hz),
        "-ac",
        str(channel_count),
        "-f",
        "wav",
        str(output_path),
    ]
    _run_checked(command)


def validate_replacement_bounds(replace_at_ms: int, resume_at_ms: int, total_duration_ms: int) -> None:
    if replace_at_ms < 0 or resume_at_ms <= replace_at_ms or resume_at_ms > total_duration_ms:
        raise ValueError("替换时间范围必须满足 0 <= replace_at_ms < resume_at_ms <= 源音频时长")


def load_xtts_model(tts_module: Any) -> Any:
    try:
        return tts_module.TTS(model_name=DEFAULT_XTTS_MODEL, gpu=False)
    except EOFError as error:
        if os.environ.get(MODEL_ROOT_ENV, "").strip():
            message = "XTTS-v2 模型资源不可用，请重新安装包含完整人声模型的安装包。"
        else:
            message = "XTTS-v2 模型资源不可用：当前开发环境尚未准备模型资源。"
        raise RuntimeError(
            message
        ) from error


def reset_xtts_model_cache() -> None:
    """清空当前 Worker 进程中的 XTTS 模型缓存，仅供测试和受控重置使用。"""
    with _XTTS_MODEL_CACHE_LOCK:
        _XTTS_MODEL_CACHE.clear()


def _xtts_model_cache_key(tts_module: Any) -> tuple[str | None, Any]:
    model_root = _configured_model_root()
    return (str(model_root) if model_root is not None else None, tts_module)


def _get_xtts_module(tts_module: Any | None = None) -> Any:
    _configured_model_root()
    return tts_module or _import_optional_module("TTS.api")


def xtts_model_is_cached(tts_module: Any | None = None) -> bool:
    module = _get_xtts_module(tts_module)
    key = _xtts_model_cache_key(module)
    with _XTTS_MODEL_CACHE_LOCK:
        return key in _XTTS_MODEL_CACHE


def ensure_xtts_model_ready(tts_module: Any | None = None) -> Any:
    module = _get_xtts_module(tts_module)
    key = _xtts_model_cache_key(module)
    with _XTTS_MODEL_CACHE_LOCK:
        cached_model = _XTTS_MODEL_CACHE.get(key)
        if cached_model is not None:
            return cached_model
        model = load_xtts_model(module)
        _XTTS_MODEL_CACHE[key] = model
        return model


def _xtts_model_progress(tts_module: Any | None = None) -> tuple[str, str]:
    if xtts_model_is_cached(tts_module):
        return "reusing-model", "正在复用已加载的 XTTS-v2 模型"
    if _configured_model_root() is not None:
        return "loading-local-model", "正在加载本地 XTTS-v2 模型（首次可能需要一些时间）"
    return "downloading-models", "正在下载或加载 XTTS-v2 模型。"


def replace_current(request: JsonDict, output_json: Path) -> JsonDict:
    operation_id = str(request.get("operation_id") or uuid.uuid4().hex)
    progress_path: Path | None = None
    try:
        progress_path = _optional_absolute_path(request.get("progress_path"), "progress_path")
        _configured_model_root()
        _safe_emit_progress(
            progress_path,
            status="running",
            phase="checking-models",
            message="正在检查模型缓存。",
            percent=0,
        )
        capability = capabilities()
        if not capability["available"]:
            reason = capability["reason"] or "本地 voice clone 能力不可用"
            _safe_emit_progress(
                progress_path,
                status="failed",
                phase="failed",
                message=reason,
                percent=100,
            )
            return _failure_result(operation_id, reason)

        source_path = _read_existing_file(request.get("source_path"), "source_path")
        audio_base_path = _read_existing_file(
            request.get("audio_base_path") or source_path,
            "audio_base_path",
        )
        reference_audio_path = _read_existing_file(request.get("reference_audio_path"), "reference_audio_path")
        text = _validate_text(request.get("text"))
        replace_at_ms = int(request.get("replace_at_ms"))
        resume_at_ms = int(request.get("resume_at_ms"))

        source_sha256 = _request_source_identity(request)

        ffmpeg_path = _resolve_executable(FFMPEG_PATH_ENV, "ffmpeg")
        ffprobe_path = _resolve_executable(FFPROBE_PATH_ENV, "ffprobe")
        if ffmpeg_path is None or ffprobe_path is None:
            raise RuntimeError("FFmpeg/FFprobe 不可用")

        source_info = _probe_audio_stream(ffprobe_path, audio_base_path)
        expected_source_duration_ms = int(request.get("source_duration_ms") or 0)
        if expected_source_duration_ms > 0 and abs(source_info["duration_ms"] - expected_source_duration_ms) > 1_000:
            raise ValueError("当前处理后音频与源视频时长差异过大")
        validate_replacement_bounds(replace_at_ms, resume_at_ms, source_info["duration_ms"])
        remaining_ms = resume_at_ms - replace_at_ms
        sample_rate_hz = int(request.get("sample_rate_hz") or source_info["sample_rate_hz"])
        channel_count = int(request.get("channel_count") or source_info["channel_count"])
        if sample_rate_hz <= 0 or channel_count <= 0:
            raise ValueError("替换输出音频参数无效")
        operation_dir = _output_path(output_json, operation_id, "replacement.wav").parent
        cloned_raw_path = operation_dir / "clone-raw.wav"
        cloned_adjusted_path = operation_dir / "clone-adjusted.wav"
        final_audio_path = operation_dir / "replacement.wav"

        model_phase, model_message = _xtts_model_progress()
        _safe_emit_progress(
            progress_path,
            status="running",
            phase=model_phase,
            message=model_message,
            percent=25,
        )
        tts = ensure_xtts_model_ready()
        _safe_emit_progress(
            progress_path,
            status="running",
            phase="generating",
            message="正在生成固定话术音轨。",
            percent=60,
        )
        tts.tts_to_file(
            text=text,
            speaker_wav=str(reference_audio_path),
            language=DEFAULT_REFERENCE_LANGUAGE,
            file_path=str(cloned_raw_path),
        )

        generated_duration_ms = _read_wav_duration_ms(cloned_raw_path)
        duration_plan = plan_replacement_duration(generated_duration_ms, remaining_ms)
        _adjust_cloned_audio(
            ffmpeg_path,
            cloned_raw_path,
            cloned_adjusted_path,
            sample_rate_hz=sample_rate_hz,
            channel_count=channel_count,
            plan=duration_plan,
        )
        _concat_source_with_clone(
            ffmpeg_path,
            audio_base_path,
            cloned_adjusted_path,
            final_audio_path,
            replace_at_ms=replace_at_ms,
            resume_at_ms=resume_at_ms,
            sample_rate_hz=sample_rate_hz,
            channel_count=channel_count,
        )

        replacement_sha256 = _sha256(final_audio_path)
        result = _result_base(operation_id, "success", model=DEFAULT_XTTS_MODEL)
        result.update(
            {
                "source_path": str(source_path),
                "source_sha256": source_sha256,
                "replacement_audio_path": str(final_audio_path),
                "replacement_sha256": replacement_sha256,
                "replace_at_ms": replace_at_ms,
                "resume_at_ms": resume_at_ms,
                "total_duration_ms": source_info["duration_ms"],
                "sample_rate_hz": sample_rate_hz,
                "channel_count": channel_count,
                "input_text": text,
                "source_generation": request.get("source_generation"),
                "remaining_ms": remaining_ms,
            }
        )
        _safe_emit_progress(
            progress_path,
            status="success",
            phase="ready",
            message="固定话术音轨已生成。",
            percent=100,
        )
        return result
    except Exception as error:  # noqa: BLE001 - Worker 必须返回结构化失败
        reason = str(error)
        _safe_emit_progress(
            progress_path,
            status="failed",
            phase="failed",
            message=reason,
            percent=100,
        )
        return _failure_result(operation_id, reason, model=DEFAULT_XTTS_MODEL)


def synthesize_text(request: JsonDict, output_json: Path) -> JsonDict:
    operation_id = str(request.get("operation_id") or uuid.uuid4().hex)
    progress_path: Path | None = None
    try:
        progress_path = _optional_absolute_path(request.get("progress_path"), "progress_path")
        _configured_model_root()
        _safe_emit_progress(
            progress_path,
            status="running",
            phase="checking-models",
            message="正在检查模型缓存。",
            percent=0,
        )
        capability = capabilities()
        if not capability["available"]:
            reason = capability["reason"] or "本地 voice clone 能力不可用"
            _safe_emit_progress(
                progress_path,
                status="failed",
                phase="failed",
                message=reason,
                percent=100,
            )
            return _failure_result(operation_id, reason, model=DEFAULT_XTTS_MODEL)

        source_path = _read_existing_file(request.get("source_path"), "source_path")
        reference_audio_path = _read_existing_file(request.get("reference_audio_path"), "reference_audio_path")
        text = _validate_text(request.get("text"))

        source_sha256 = _request_source_identity(request)

        expected_reference_sha256 = str(request.get("reference_audio_sha256") or "").strip()
        actual_reference_sha256 = _sha256(reference_audio_path)
        if expected_reference_sha256 and expected_reference_sha256 != actual_reference_sha256:
            raise ValueError("reference_audio_sha256 与当前参考音频不匹配")

        sample_rate_hz = int(request.get("sample_rate_hz") or DEFAULT_REFERENCE_SAMPLE_RATE_HZ)
        channel_count = int(request.get("channel_count") or 1)
        if sample_rate_hz <= 0 or channel_count <= 0:
            raise ValueError("当前文案输出音频参数无效")
        operation_dir = _output_path(output_json, operation_id, "current-text.wav").parent
        raw_audio_path = operation_dir / "current-text-raw.wav"
        audio_path = operation_dir / "current-text.wav"
        model_value = str(request.get("model") or DEFAULT_XTTS_MODEL)

        model_phase, model_message = _xtts_model_progress()
        _safe_emit_progress(
            progress_path,
            status="running",
            phase=model_phase,
            message=model_message,
            percent=25,
        )
        tts = ensure_xtts_model_ready()
        _safe_emit_progress(
            progress_path,
            status="running",
            phase="generating",
            message="正在生成当前文案的克隆声音。",
            percent=60,
        )
        tts.tts_to_file(
            text=text,
            speaker_wav=str(reference_audio_path),
            language=DEFAULT_REFERENCE_LANGUAGE,
            file_path=str(raw_audio_path),
        )

        raw_audio_info = _read_wav_info(raw_audio_path)
        if (
            raw_audio_info["sample_rate_hz"] != sample_rate_hz
            or raw_audio_info["channel_count"] != channel_count
        ):
            ffmpeg_path = _resolve_executable(FFMPEG_PATH_ENV, "ffmpeg")
            if ffmpeg_path is None:
                raise RuntimeError("FFmpeg 不可用，无法匹配当前源音频参数")
            _normalize_generated_audio(
                ffmpeg_path,
                raw_audio_path,
                audio_path,
                sample_rate_hz=sample_rate_hz,
                channel_count=channel_count,
            )
        else:
            shutil.copyfile(raw_audio_path, audio_path)
        audio_info = _read_wav_info(audio_path)
        result = _result_base(operation_id, "success", model=model_value)
        result.update(
            {
                "source_generation": request.get("source_generation"),
                "source_path": str(source_path),
                "source_sha256": source_sha256,
                "reference_audio_path": str(reference_audio_path),
                "reference_audio_sha256": actual_reference_sha256,
                "input_text": text,
                "text_sha256": _text_sha256(text),
                "audio_path": str(audio_path),
                "audio_sha256": _sha256(audio_path),
                "duration_ms": audio_info["duration_ms"],
                "sample_rate_hz": audio_info["sample_rate_hz"],
                "channel_count": audio_info["channel_count"],
            }
        )
        _safe_emit_progress(
            progress_path,
            status="success",
            phase="ready",
            message="当前文案克隆声音已生成。",
            percent=100,
        )
        return result
    except Exception as error:  # noqa: BLE001 - Worker 必须返回结构化失败
        reason = str(error)
        _safe_emit_progress(
            progress_path,
            status="failed",
            phase="failed",
            message=reason,
            percent=100,
        )
        return _failure_result(operation_id, reason, model=DEFAULT_XTTS_MODEL)


def warmup_model(request: JsonDict, output_json: Path) -> JsonDict:
    operation_id = str(request.get("operation_id") or uuid.uuid4().hex)
    progress_path: Path | None = None
    try:
        progress_path = _optional_absolute_path(request.get("progress_path"), "progress_path")
        _configured_model_root()
        _safe_emit_progress(
            progress_path,
            status="running",
            phase="checking-models",
            message="正在检查模型缓存。",
            percent=0,
        )
        capability = capabilities()
        if not capability["available"]:
            reason = capability["reason"] or "本地 voice clone 能力不可用"
            _safe_emit_progress(progress_path, status="failed", phase="failed", message=reason, percent=100)
            return _failure_result(operation_id, reason, model=DEFAULT_XTTS_MODEL)
        model_phase, model_message = _xtts_model_progress()
        _safe_emit_progress(
            progress_path,
            status="running",
            phase=model_phase,
            message=model_message,
            percent=50,
        )
        ensure_xtts_model_ready()
        _safe_emit_progress(
            progress_path,
            status="success",
            phase="ready",
            message="XTTS-v2 模型已预加载。",
            percent=100,
        )
        return _result_base(operation_id, "success", model=DEFAULT_XTTS_MODEL)
    except Exception as error:  # noqa: BLE001 - Worker 必须返回结构化失败
        reason = str(error)
        _safe_emit_progress(progress_path, status="failed", phase="failed", message=reason, percent=100)
        return _failure_result(operation_id, reason, model=DEFAULT_XTTS_MODEL)


def _write_serve_response(output_stream: Any, response: JsonDict) -> None:
    output_stream.write(json.dumps(response, ensure_ascii=False) + "\n")
    output_stream.flush()


def _serve_one_request(request: Any, output_stream: Any) -> None:
    if not isinstance(request, dict):
        _write_serve_response(
            output_stream,
            {"status": "failed", "exit_code": 2, "error": "常驻请求必须是 JSON 对象"},
        )
        return

    raw_args = request.get("args")
    if not isinstance(raw_args, list) or not all(isinstance(item, str) for item in raw_args):
        _write_serve_response(
            output_stream,
            {
                "status": "failed",
                "exit_code": 2,
                "error": "常驻请求必须包含字符串数组 args",
            },
        )
        return
    if "--serve" in raw_args:
        _write_serve_response(
            output_stream,
            {"status": "failed", "exit_code": 2, "error": "常驻请求不能嵌套 --serve"},
        )
        return

    try:
        # 第三方库可能向 stdout 输出日志；协议通道必须只保留逐行 JSON 响应。
        with contextlib.redirect_stdout(sys.stderr):
            exit_code = main(raw_args)
        normalized_exit_code = exit_code if isinstance(exit_code, int) else 0
        response: JsonDict = {
            "status": "completed" if normalized_exit_code == 0 else "failed",
            "exit_code": normalized_exit_code,
        }
        if "request_id" in request:
            response["request_id"] = request["request_id"]
        _write_serve_response(output_stream, response)
    except SystemExit as error:
        exit_code = error.code if isinstance(error.code, int) else 1
        response = {"status": "failed", "exit_code": exit_code}
        if isinstance(error.code, str) and error.code:
            response["error"] = error.code
        _write_serve_response(output_stream, response)
    except Exception as error:  # noqa: BLE001 - 常驻协议必须隔离单条请求失败
        _write_serve_response(
            output_stream,
            {"status": "failed", "exit_code": 1, "error": str(error)},
        )


def serve(input_stream: Any = None, output_stream: Any = None) -> int:
    """以逐行 JSON 协议顺序执行现有 CLI 操作，模型缓存留在本进程内。"""
    request_stream = sys.stdin if input_stream is None else input_stream
    response_stream = sys.stdout if output_stream is None else output_stream
    for line in request_stream:
        if not line.strip():
            continue
        try:
            request = json.loads(line)
        except json.JSONDecodeError as error:
            _write_serve_response(
                response_stream,
                {"status": "failed", "exit_code": 2, "error": f"常驻请求 JSON 无效：{error}"},
            )
            continue
        _serve_one_request(request, response_stream)
    return 0


def main(argv: list[str] | None = None) -> int:
    worker_argv = sys.argv[1:] if argv is None else argv
    parser = argparse.ArgumentParser()
    parser.add_argument("--capabilities-json", type=Path)
    parser.add_argument("--prepare-json", type=Path)
    parser.add_argument("--replace-json", type=Path)
    parser.add_argument("--synthesize-json", type=Path)
    parser.add_argument("--warmup-json", type=Path)
    parser.add_argument("--output-json", type=Path)
    parser.add_argument("--progress-json", type=Path)
    parser.add_argument("--serve", action="store_true")
    if worker_argv and worker_argv[0] == FROZEN_DEMUCS_DISPATCH_ARG:
        if not getattr(sys, "frozen", False):
            parser.error(f"{FROZEN_DEMUCS_DISPATCH_ARG} 仅可由冻结 Worker 使用")
        return _dispatch_frozen_demucs(worker_argv[1:])
    args = parser.parse_args(worker_argv)

    if args.serve:
        if len(worker_argv) != 1:
            parser.error("--serve 不能与其他 Worker 参数同时使用")
        return serve()

    if args.capabilities_json is not None:
        write_atomic_json(args.capabilities_json, capabilities())
        return 0

    operation_args = [
        name
        for name, value in (
            ("--prepare-json", args.prepare_json),
            ("--replace-json", args.replace_json),
            ("--synthesize-json", args.synthesize_json),
            ("--warmup-json", args.warmup_json),
        )
        if value is not None
    ]
    if not operation_args:
        parser.error("必须提供 --capabilities-json、--prepare-json、--replace-json、--synthesize-json 或 --warmup-json 之一")
    if len(operation_args) > 1:
        parser.error("不能同时提供多个 Worker 操作参数")
    if args.output_json is None:
        parser.error("Worker 操作必须提供 --output-json")

    operation_mode = "prepare" if args.prepare_json is not None else "replace" if args.replace_json is not None else "synthesize" if args.synthesize_json is not None else "warmup"
    request_path = args.prepare_json or args.replace_json or args.synthesize_json or args.warmup_json
    try:
        request = _read_json(request_path)
    except Exception as error:  # noqa: BLE001 - 输入 JSON 也必须结构化失败
        write_atomic_json(
            args.output_json,
            _failure_result(
                "invalid-request",
                f"{operation_mode} 请求 JSON 无效：{error}",
                model=DEFAULT_XTTS_MODEL,
            ),
        )
        return 0

    if args.progress_json is not None:
        request["progress_path"] = str(args.progress_json)

    if args.prepare_json is not None:
        result = prepare_source(request, args.output_json)
        write_atomic_json(args.output_json, result)
        return 0

    if args.warmup_json is not None:
        result = warmup_model(request, args.output_json)
        write_atomic_json(args.output_json, result)
        return 0

    if args.replace_json is not None:
        result = replace_current(request, args.output_json)
    else:
        result = synthesize_text(request, args.output_json)
    write_atomic_json(args.output_json, result)
    return 0


if __name__ == "__main__":
    sys.exit(main())
