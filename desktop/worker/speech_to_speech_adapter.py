#!/usr/bin/env python3
"""autoLive 的 Hugging Face speech-to-speech 本地 Worker 适配器。

Rust 负责进程生命周期、超时、取消、上下文门禁和最终候选提交；本文件只负责：
1. 读取 SpeechToSpeechContext JSON；
2. 从本地媒体提取当前片段 PCM；
3. 通过 Hugging Face speech-to-speech 的 OpenAI Realtime-compatible WebSocket
   发送音频并接收 TTS 音频；
4. 在受控输出目录写 WAV 和结构化结果 JSON。

这不是服务端，也不上传视频。依赖和本地 speech-to-speech serve 进程由用户环境配置。
"""

from __future__ import annotations

import argparse
import asyncio
import base64
import hashlib
import io
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
import uuid
import wave
from pathlib import Path
from typing import Any


DEFAULT_ENDPOINT = "ws://127.0.0.1:8765/v1/realtime"
DEFAULT_OUTPUT_SAMPLE_RATE_HZ = 24_000
INPUT_SAMPLE_RATE_HZ = 16_000
INPUT_CHANNEL_COUNT = 1
MAX_OUTPUT_BYTES = 64 * 1024 * 1024


def write_json(path: Path, value: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    partial = path.with_name(f"{path.name}.partial")
    if path.exists():
        raise FileExistsError(path)
    partial.write_text(json.dumps(value, ensure_ascii=False), encoding="utf-8")
    partial.replace(path)


def capabilities() -> dict[str, Any]:
    endpoint = os.environ.get("AUTOLIVE_SPEECH_TO_SPEECH_ENDPOINT", DEFAULT_ENDPOINT).strip()
    try:
        import websockets  # noqa: F401
    except ImportError:
        return {
            "available": False,
            "status": "unavailable",
            "provider": None,
            "model": None,
            "reason": "未安装 Python websockets 依赖",
        }
    if not endpoint:
        return {
            "available": False,
            "status": "unavailable",
            "provider": None,
            "model": None,
            "reason": "未配置 AUTOLIVE_SPEECH_TO_SPEECH_ENDPOINT",
        }
    if shutil.which("ffmpeg") is None:
        return {
            "available": False,
            "status": "unavailable",
            "provider": None,
            "model": None,
            "reason": "未安装 ffmpeg，无法提取和归一化本地音频",
        }
    return {
        "available": True,
        "status": "available",
        "provider": "huggingface-speech-to-speech",
        "model": os.environ.get("AUTOLIVE_SPEECH_TO_SPEECH_MODEL", "local-realtime"),
        "reason": None,
    }


def fallback(reason: str, model: str = "hf-speech-to-speech-adapter") -> dict[str, Any]:
    return {
        "decision": "keep_original",
        "text": "沿用原始话术",
        "audio_path_or_stream_ref": None,
        "audio_sha256": None,
        "duration_ms": None,
        "sync_offset_ms": None,
        "sample_rate_hz": None,
        "channel_count": None,
        "latency_ms": 1,
        "model": model,
        "fallback_reason": reason,
    }


def read_context(path: Path) -> dict[str, Any]:
    context = json.loads(path.read_text(encoding="utf-8"))
    required = (
        "track_id",
        "segment_id",
        "audio_path_or_stream_ref",
        "start_at_ms",
        "target_duration_ms",
        "max_chars",
        "language",
        "sample_rate_hz",
        "channel_count",
    )
    if not isinstance(context, dict) or any(not str(context.get(key, "")).strip() for key in required):
        raise ValueError("上下文缺少必要字段")
    if int(context["target_duration_ms"]) <= 0 or int(context["max_chars"]) <= 0:
        raise ValueError("上下文目标时长或最大字数无效")
    return context


def local_path(reference: str) -> Path:
    value = reference.removeprefix("file://")
    path = Path(value).expanduser().resolve()
    if not path.is_file():
        raise FileNotFoundError(path)
    return path


def extract_pcm(context: dict[str, Any]) -> bytes:
    source = local_path(str(context["audio_path_or_stream_ref"]))
    start_seconds = max(0.0, int(context.get("start_at_ms", 0)) / 1000)
    duration_seconds = int(context["target_duration_ms"]) / 1000
    command = [
        "ffmpeg",
        "-hide_banner",
        "-loglevel",
        "error",
        "-ss",
        f"{start_seconds:.3f}",
        "-t",
        f"{duration_seconds:.3f}",
        "-i",
        str(source),
        "-vn",
        "-ac",
        str(INPUT_CHANNEL_COUNT),
        "-ar",
        str(INPUT_SAMPLE_RATE_HZ),
        "-f",
        "s16le",
        "pipe:1",
    ]
    completed = subprocess.run(command, check=False, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    if completed.returncode != 0 or not completed.stdout:
        raise RuntimeError("无法从源媒体提取当前音频片段")
    return completed.stdout


def prompt_for(context: dict[str, Any]) -> str:
    locked = ", ".join(str(value) for value in context.get("locked_field_values", [])) or "无"
    previous = str(context.get("previous_variant_text") or "无")
    transcript = str(context.get("transcript_text") or "")
    return (
        "你是本地授权视频的口播改写器。保持原意和卖点，只做自然口语化改写；"
        "不得编造或修改锁定字段，不输出解释。请直接生成可播放的中文口播。\n"
        f"原文：{transcript}\n锁定字段值：{locked}\n上一版本：{previous}\n"
        f"最大字数：{context['max_chars']}；目标时长：{context['target_duration_ms']}ms；"
        f"语言：{context['language']}；改写规则：{context.get('rewrite_policy', 'keep_meaning')}"
    )


async def request_realtime_audio(context: dict[str, Any], pcm: bytes) -> tuple[str, bytes]:
    try:
        import websockets
    except ImportError as error:
        raise RuntimeError("未安装 Python websockets 依赖") from error

    endpoint = os.environ.get("AUTOLIVE_SPEECH_TO_SPEECH_ENDPOINT", DEFAULT_ENDPOINT).strip()
    model = os.environ.get("AUTOLIVE_SPEECH_TO_SPEECH_MODEL", "local-realtime")
    output: bytearray = bytearray()
    text_parts: list[str] = []
    headers = {}
    token = os.environ.get("AUTOLIVE_SPEECH_TO_SPEECH_TOKEN", "").strip()
    if token:
        headers["Authorization"] = f"Bearer {token}"

    async with websockets.connect(endpoint, additional_headers=headers, open_timeout=5, close_timeout=2) as socket:
        await socket.send(json.dumps({
            "type": "session.update",
            "session": {
                "type": "realtime",
                "instructions": prompt_for(context),
                "audio": {
                    "input": {"turn_detection": {"type": "server_vad"}},
                    "output": {"voice": os.environ.get("AUTOLIVE_SPEECH_TO_SPEECH_VOICE", "default")},
                },
            },
        }))
        for offset in range(0, len(pcm), 640):
            chunk = pcm[offset : offset + 640]
            await socket.send(json.dumps({
                "type": "input_audio_buffer.append",
                "audio": base64.b64encode(chunk).decode("ascii"),
            }))
        await socket.send(json.dumps({"type": "input_audio_buffer.commit"}))
        await socket.send(json.dumps({"type": "response.create", "response": {"modalities": ["text", "audio"]}}))

        deadline = time.monotonic() + max(5.0, int(context["timeout_ms"]) / 1000)
        while time.monotonic() < deadline:
            event = json.loads(await asyncio.wait_for(socket.recv(), timeout=1.0))
            event_type = str(event.get("type", ""))
            if event_type.endswith("audio.delta"):
                output.extend(base64.b64decode(event.get("delta", "")))
                if len(output) > MAX_OUTPUT_BYTES:
                    raise RuntimeError("Worker 音频产物超过大小上限")
            elif event_type.endswith("text.delta") or event_type.endswith("transcript.delta"):
                text_parts.append(str(event.get("delta", "")))
            elif event_type in {"response.done", "response.completed"}:
                break
        if not output:
            raise RuntimeError("speech-to-speech 未返回音频产物")
    return "".join(text_parts).strip(), bytes(output)


def atempo_filters(factor: float) -> list[str]:
    filters: list[str] = []
    remaining = factor
    while remaining > 2.0:
        filters.append("atempo=2.0")
        remaining /= 2.0
    while remaining < 0.5:
        filters.append("atempo=0.5")
        remaining /= 0.5
    filters.append(f"atempo={remaining:.6f}")
    return filters


def normalize_audio(
    audio: bytes,
    source_sample_rate_hz: int,
    target_sample_rate_hz: int,
    target_channel_count: int,
    target_duration_ms: int,
) -> bytes:
    if not audio or source_sample_rate_hz <= 0 or target_sample_rate_hz <= 0:
        raise ValueError("音频采样率或内容无效")
    if target_channel_count <= 0 or target_duration_ms <= 0:
        raise ValueError("目标音频格式或时长无效")
    source_duration_seconds = len(audio) / 2 / source_sample_rate_hz
    target_duration_seconds = target_duration_ms / 1000
    if source_duration_seconds <= 0:
        raise ValueError("音频时长为空")
    # atempo 的倍率是“输出时长 / 输入时长”的倒数，因此输入较长时需要加速。
    tempo = source_duration_seconds / target_duration_seconds
    filters = atempo_filters(tempo)
    filters.extend(
        [
            f"apad=whole_dur={target_duration_seconds:.6f}",
            f"atrim=duration={target_duration_seconds:.6f}",
        ]
    )
    command = [
        "ffmpeg",
        "-hide_banner",
        "-loglevel",
        "error",
        "-f",
        "s16le",
        "-ar",
        str(source_sample_rate_hz),
        "-ac",
        "1",
        "-i",
        "pipe:0",
        "-af",
        ",".join(filters),
        "-ar",
        str(target_sample_rate_hz),
        "-ac",
        str(target_channel_count),
        "-f",
        "wav",
        "pipe:1",
    ]
    completed = subprocess.run(command, input=audio, check=False, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    if completed.returncode != 0 or not completed.stdout:
        raise RuntimeError("无法将候选音频转换为当前音轨格式")
    return completed.stdout


def write_wav(
    audio: bytes,
    source_sample_rate_hz: int,
    target_sample_rate_hz: int,
    target_channel_count: int,
    target_duration_ms: int,
) -> tuple[Path, str, int]:
    normalized = normalize_audio(
        audio,
        source_sample_rate_hz,
        target_sample_rate_hz,
        target_channel_count,
        target_duration_ms,
    )
    output_dir = Path(os.environ.get("AUTOLIVE_SPEECH_TO_SPEECH_OUTPUT_DIR", tempfile.gettempdir())) / "autolive-speech-variants"
    output_dir.mkdir(parents=True, exist_ok=True)
    output_path = output_dir / f"variant-{uuid.uuid4().hex}.wav"
    partial = output_path.with_suffix(".wav.partial")
    partial.write_bytes(normalized)
    partial.replace(output_path)
    digest = hashlib.sha256(output_path.read_bytes()).hexdigest()
    with wave.open(io.BytesIO(normalized), "rb") as wav_file:
        duration_ms = round(wav_file.getnframes() / wav_file.getframerate() * 1000)
    return output_path, digest, duration_ms


def run(context_path: Path) -> dict[str, Any]:
    started = time.monotonic()
    try:
        context = read_context(context_path)
        if not capabilities()["available"]:
            return fallback("本地 speech-to-speech 依赖或端点不可用")
        pcm = extract_pcm(context)
        text, audio = asyncio.run(request_realtime_audio(context, pcm))
        source_sample_rate_hz = int(
            os.environ.get(
                "AUTOLIVE_SPEECH_TO_SPEECH_OUTPUT_SAMPLE_RATE_HZ",
                DEFAULT_OUTPUT_SAMPLE_RATE_HZ,
            )
        )
        target_sample_rate_hz = int(context["sample_rate_hz"])
        target_channel_count = int(context["channel_count"])
        target_duration_ms = int(context["target_duration_ms"])
        path, digest, duration_ms = write_wav(
            audio,
            source_sample_rate_hz,
            target_sample_rate_hz,
            target_channel_count,
            target_duration_ms,
        )
        if not text:
            text = str(context.get("transcript_text") or "沿用原始话术")
        return {
            "decision": "rewrite",
            "text": text[: int(context["max_chars"])],
            "audio_path_or_stream_ref": str(path),
            "audio_sha256": digest,
            "duration_ms": duration_ms,
            "sync_offset_ms": 0,
            "sample_rate_hz": target_sample_rate_hz,
            "channel_count": target_channel_count,
            "latency_ms": max(1, round((time.monotonic() - started) * 1000)),
            "model": os.environ.get("AUTOLIVE_SPEECH_TO_SPEECH_MODEL", "local-realtime"),
            "fallback_reason": None,
        }
    except Exception as error:  # noqa: BLE001 - Worker 必须将失败降级成结构化原声结果
        return fallback(str(error))


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--capabilities-json", type=Path)
    parser.add_argument("--input-json", type=Path)
    parser.add_argument("--output-json", type=Path)
    args = parser.parse_args()
    if args.capabilities_json:
        write_json(args.capabilities_json, capabilities())
        return 0
    if not args.input_json or not args.output_json:
        parser.error("必须同时提供 --input-json 和 --output-json")
    write_json(args.output_json, run(args.input_json))
    return 0


if __name__ == "__main__":
    sys.exit(main())
