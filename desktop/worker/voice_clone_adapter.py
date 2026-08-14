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
import hashlib
import importlib
import json
import os
import shutil
import subprocess
import sys
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
DEFAULT_SUBPROCESS_TIMEOUT_SECONDS = 30 * 60


JsonDict = dict[str, Any]


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
    max_seconds: int | None = DEFAULT_REFERENCE_MAX_SECONDS,
) -> None:
    command = [
        str(ffmpeg_path),
        "-hide_banner",
        "-loglevel",
        "error",
        "-i",
        str(vocals_path),
        "-vn",
        "-ac",
        "1",
        "-ar",
        str(sample_rate_hz),
    ]
    if max_seconds is not None:
        command.extend(["-t", str(max_seconds)])
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


def _validate_text(text: Any) -> str:
    value = str(text or "").strip()
    if not value:
        raise ValueError("替换文本不能为空")
    if len(value) > MAX_VOICE_CLONE_TEXT_CHARS:
        raise ValueError("替换文本超过 500 字限制")
    return value


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


def prepare_source(request: JsonDict, output_json: Path) -> JsonDict:
    operation_id = str(request.get("operation_id") or uuid.uuid4().hex)
    capability = capabilities()
    if not capability["available"]:
        return _failure_result(operation_id, capability["reason"] or "本地 voice clone 能力不可用", model=DEFAULT_XTTS_MODEL)

    try:
        source_path = _read_existing_file(request.get("source_path"), "source_path")
        ffmpeg_path = _resolve_executable(FFMPEG_PATH_ENV, "ffmpeg")
        ffprobe_path = _resolve_executable(FFPROBE_PATH_ENV, "ffprobe")
        if ffmpeg_path is None or ffprobe_path is None:
            raise RuntimeError("FFmpeg/FFprobe 不可用")

        source_sha256 = _sha256(source_path)
        source_info = _probe_audio_stream(ffprobe_path, source_path)
        operation_dir = _output_path(output_json, operation_id, "reference.wav").parent
        source_audio_path = operation_dir / "source-audio.wav"
        demucs_output_dir = operation_dir / "demucs"
        reference_audio_path = operation_dir / "reference.wav"
        model_size = str(request.get("model_size") or DEFAULT_WHISPER_MODEL_SIZE)
        language = str(request.get("language") or "zh")

        _extract_source_audio(ffmpeg_path, source_path, source_audio_path)

        _import_optional_module("demucs.separate")
        demucs_executable = [
            str(sys.executable),
            "-m",
            "demucs.separate",
            "--device",
            "cpu",
            "--two-stems=vocals",
            "-o",
            str(demucs_output_dir),
            str(source_audio_path),
        ]
        demucs_model_name = str(request.get("demucs_model") or "").strip()
        if demucs_model_name:
            demucs_executable[demucs_executable.index("-o"):demucs_executable.index("-o")] = ["-n", demucs_model_name]
        _run_checked(demucs_executable)

        vocals_path = _find_vocals_path(demucs_output_dir)
        _normalize_reference_audio(ffmpeg_path, vocals_path, reference_audio_path)
        index_audio_path = operation_dir / "transcription.wav"
        _normalize_reference_audio(
            ffmpeg_path,
            vocals_path,
            index_audio_path,
            max_seconds=None,
        )
        reference_sha256 = _sha256(reference_audio_path)

        faster_whisper_module = _import_optional_module("faster_whisper")
        whisper_model = faster_whisper_module.WhisperModel(
            model_size,
            device="cpu",
            compute_type="int8",
        )
        segments = _prepare_segments(whisper_model, index_audio_path, language=language)
        if not segments:
            raise RuntimeError("未检测到有效人声片段")

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
        return result
    except Exception as error:  # noqa: BLE001 - Worker 必须返回结构化失败
        return _failure_result(operation_id, str(error), model=DEFAULT_XTTS_MODEL)


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


def replace_current(request: JsonDict, output_json: Path) -> JsonDict:
    operation_id = str(request.get("operation_id") or uuid.uuid4().hex)
    capability = capabilities()
    if not capability["available"]:
        return _failure_result(operation_id, capability["reason"] or "本地 voice clone 能力不可用")

    try:
        source_path = _read_existing_file(request.get("source_path"), "source_path")
        audio_base_path = _read_existing_file(
            request.get("audio_base_path") or source_path,
            "audio_base_path",
        )
        reference_audio_path = _read_existing_file(request.get("reference_audio_path"), "reference_audio_path")
        text = _validate_text(request.get("text"))
        replace_at_ms = int(request.get("replace_at_ms"))
        resume_at_ms = int(request.get("resume_at_ms"))

        expected_source_sha256 = str(request.get("source_sha256") or "").strip()
        actual_source_sha256 = _sha256(source_path)
        if expected_source_sha256 and expected_source_sha256 != actual_source_sha256:
            raise ValueError("source_sha256 与当前源文件不匹配")

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

        tts_module = _import_optional_module("TTS.api")
        tts = tts_module.TTS(model_name=DEFAULT_XTTS_MODEL, gpu=False)
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
                "source_sha256": actual_source_sha256,
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
        return result
    except Exception as error:  # noqa: BLE001 - Worker 必须返回结构化失败
        return _failure_result(operation_id, str(error), model=DEFAULT_XTTS_MODEL)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--capabilities-json", type=Path)
    parser.add_argument("--prepare-json", type=Path)
    parser.add_argument("--replace-json", type=Path)
    parser.add_argument("--output-json", type=Path)
    args = parser.parse_args(argv)

    if args.capabilities_json is not None:
        write_atomic_json(args.capabilities_json, capabilities())
        return 0

    if args.prepare_json is None and args.replace_json is None:
        parser.error("必须提供 --capabilities-json、--prepare-json 或 --replace-json 之一")
    if args.prepare_json is not None and args.replace_json is not None:
        parser.error("不能同时提供 --prepare-json 和 --replace-json")
    if args.output_json is None:
        parser.error("准备或替换模式必须提供 --output-json")

    operation_mode = "prepare" if args.prepare_json is not None else "replace"
    request_path = args.prepare_json or args.replace_json
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

    if args.prepare_json is not None:
        result = prepare_source(request, args.output_json)
        write_atomic_json(args.output_json, result)
        return 0

    result = replace_current(request, args.output_json)
    write_atomic_json(args.output_json, result)
    return 0


if __name__ == "__main__":
    sys.exit(main())
