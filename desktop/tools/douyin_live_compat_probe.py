#!/usr/bin/env python3
"""Run the private Douyin live-chat compatibility probe.

The probe imports a user-supplied checkout of ``cv-cat/Douyin_Spider`` and
never vendors that checkout into GpAutoLive. It emits only redacted JSON event
lines. Cookies, QR tokens, chat content, response bodies and signed URLs stay
in memory; an optional QR image is removed when the process exits.
"""

from __future__ import annotations

import argparse
import atexit
import gzip
import json
import re
import secrets
import ssl
import sys
import threading
import time
from pathlib import Path
from urllib.parse import urlencode


UPSTREAM_COMMIT = "9afaf79580b1ee84e8954ff906ff26869d5b7f1f"
ROOM_RE = re.compile(r"^(?:\d{1,20}|https://live\.douyin\.com/(\d{1,20}))$")


def emit(event: str, **values: object) -> None:
    print(json.dumps({"event": event, **values}, ensure_ascii=False, separators=(",", ":")), flush=True)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--upstream-root", required=True, type=Path, help="Douyin_Spider checkout")
    parser.add_argument("--room-id", help="自有测试直播间号或 https://live.douyin.com/<id>")
    parser.add_argument("--reply", action="append", help="回复候选；可重复传入，入队时随机选一条")
    parser.add_argument("--timeout", type=int, default=300, help="扫码和监听总超时（秒）")
    parser.add_argument("--qr-output", type=Path, help="可选临时二维码 PNG 路径")
    args = parser.parse_args()
    if args.timeout < 30 or args.timeout > 900:
        parser.error("--timeout 必须在 30～900 秒")
    room = args.room_id or ""
    if room and not ROOM_RE.fullmatch(room):
        parser.error("--room-id 只接受 1～20 位数字或标准 live.douyin.com URL")
    if room.startswith("https://"):
        args.room_id = room.rsplit("/", 1)[-1]
    reply_pool = [str(item).strip() for item in (args.reply or ["GpAutoLive探针✅"])]
    if len(reply_pool) > 100 or any(
        not item or len(item) > 80 or len(item.encode("utf-8")) > 320 or any(ord(ch) < 32 for ch in item)
        for item in reply_pool
    ):
        parser.error("--reply 候选最多 100 条；每条必须是 1～80 个可打印 Unicode 字符且不超过 320 UTF-8 字节")
    args.reply_pool = tuple(dict.fromkeys(reply_pool))
    if not args.reply_pool:
        parser.error("--reply 去重后至少保留一条")
    return args


def load_upstream(root: Path):
    root = root.resolve()
    required = (root / "builder" / "auth.py", root / "dy_live" / "server.py", root / "static" / "Live_pb2.py")
    missing = [str(path) for path in required if not path.is_file()]
    if missing:
        raise FileNotFoundError("上游 checkout 缺少必要文件: " + ", ".join(missing))
    sys.path.insert(0, str(root))
    import static.Live_pb2 as live_pb2
    from builder.auth import DouyinAuth
    from builder.header import HeaderBuilder
    from builder.params import Params
    from dy_apis.douyin_api import DouyinAPI
    from utils import http_client
    from utils.dy_util import generate_signature

    return live_pb2, DouyinAuth, HeaderBuilder, Params, DouyinAPI, http_client, generate_signature


def force_tls_verification(http_client) -> None:
    """The upstream snapshot has legacy verify=False calls; fail closed here."""

    original_request = http_client.request

    def safe_request(method, url, **kwargs):
        kwargs["verify"] = True
        return original_request(method, url, **kwargs)

    http_client.request = safe_request
    for name in ("get", "post", "put", "delete", "head"):
        setattr(http_client, name, lambda url, _name=name, **kwargs: safe_request(_name.upper(), url, **kwargs))

    original_session_request = http_client.Session.request

    def safe_session_request(self, method, url, **kwargs):
        kwargs["verify"] = True
        return original_session_request(self, method, url, **kwargs)

    http_client.Session.request = safe_session_request


def build_ws_url(auth, live_id: str, room: dict, live_pb2, header_builder, params_type, douyin_api, signature) -> str:
    initial = douyin_api.get_webcast_detail(
        auth,
        str(room["user_id"]),
        str(room["room_id"]),
        f"https://live.douyin.com/{live_id}",
    )
    frame = live_pb2.LiveResponse()
    frame.ParseFromString(initial)
    params = params_type()
    (
        params.add_param("app_name", "douyin_web")
        .add_param("version_code", "180800")
        .add_param("webcast_sdk_version", "1.0.15")
        .add_param("update_version_code", "1.0.15")
        .add_param("compress", "gzip")
        .add_param("device_platform", "web")
        .add_param("cookie_enabled", "true")
        .add_param("screen_width", "1707")
        .add_param("screen_height", "960")
        .add_param("browser_language", "zh-CN")
        .add_param("browser_platform", "Win32")
        .add_param("browser_name", "Mozilla")
        .add_param("browser_version", header_builder.ua.split("Mozilla/")[-1])
        .add_param("browser_online", "true")
        .add_param("tz_name", "Etc/GMT-8")
        .add_param("cursor", str(frame.cursor))
        .add_param("internal_ext", frame.internalExt)
        .add_param("host", "https://live.douyin.com")
        .add_param("aid", "6383")
        .add_param("live_id", "1")
        .add_param("did_rule", "3")
        .add_param("endpoint", "live_pc")
        .add_param("support_wrds", "1")
        .add_param("user_unique_id", str(room["user_id"]))
        .add_param("im_path", "/webcast/im/fetch/")
        .add_param("identity", "audience")
        .add_param("need_persist_msg_count", "15")
        .add_param("insert_task_id", "")
        .add_param("live_reason", "")
        .add_param("room_id", str(room["room_id"]))
        .add_param("heartbeatDuration", "0")
        .add_param("signature", signature(str(room["room_id"]), str(room["user_id"])))
    )
    return "wss://webcast100-ws-web-hl.douyin.com/webcast/im/push/v2/?" + urlencode(params.get())


def main() -> int:
    args = parse_args()
    live_pb2, DouyinAuth, HeaderBuilder, Params, DouyinAPI, http_client, signature = load_upstream(args.upstream_root)
    force_tls_verification(http_client)
    qr_output = args.qr_output.resolve() if args.qr_output else None

    def cleanup_qr() -> None:
        if qr_output and qr_output.is_file():
            try:
                qr_output.unlink()
            except OSError:
                pass

    atexit.register(cleanup_qr)

    emit("probe_started", upstream_commit=UPSTREAM_COMMIT, tls_verify=True)

    def on_qrcode(url: str) -> None:
        if qr_output:
            qr_output.parent.mkdir(parents=True, exist_ok=True)
            import qrcode

            qrcode.make(url).save(qr_output)
            emit("qr_issued", preview_path=str(qr_output))
        else:
            emit("qr_issued")

    emit("qr_waiting", instruction="请用抖音 App 扫描二维码并确认")
    auth = DouyinAuth.from_qrcode_login(
        timeout=args.timeout,
        show_qr=qr_output is None,
        on_qrcode=on_qrcode,
        bootstrap_creator=False,
    )
    emit("login_confirmed")
    self_uid = str(auth.get_uid())
    emit("self_identity_ready")

    live_id = args.room_id or input("请输入自有测试直播间号或 live.douyin.com URL: ").strip()
    if live_id.startswith("https://"):
        live_id = live_id.rsplit("/", 1)[-1]
    if not ROOM_RE.fullmatch(live_id):
        raise ValueError("room_input_invalid")
    room = DouyinAPI.get_live_info(auth, live_id)
    if not room or not room.get("room_id") or not room.get("user_id"):
        raise RuntimeError("room_resolve_failed")
    emit("room_resolved", room_status=str(room.get("room_status", "")), has_room=True)
    ws_url = build_ws_url(auth, live_id, room, live_pb2, HeaderBuilder, Params, DouyinAPI, signature)
    reply = secrets.choice(args.reply_pool)
    emit("reply_selected", candidate_count=len(args.reply_pool))

    import websocket

    stop = threading.Event()
    outcome = {"external_chat": None, "reply_attempted": False, "self_echo_filtered": False}
    seen: set[str] = set()
    ws_holder = {"ws": None}

    def ping_loop() -> None:
        while not stop.wait(5):
            ws = ws_holder["ws"]
            if ws is None:
                continue
            try:
                ws.send(live_pb2.PushFrame(payloadType="hb").SerializeToString(), opcode=0x02)
            except Exception:
                return

    def on_open(ws) -> None:
        ws_holder["ws"] = ws
        emit("websocket_connected")
        threading.Thread(target=ping_loop, name="douyin-probe-ping", daemon=True).start()

    def on_message(ws, raw) -> None:
        frame = live_pb2.PushFrame()
        frame.ParseFromString(raw)
        response = live_pb2.LiveResponse()
        response.ParseFromString(gzip.decompress(frame.payload))
        if response.needAck:
            ack = live_pb2.PushFrame(
                payloadType="ack",
                payload=response.internalExt.encode("utf-8"),
                logId=frame.logId,
            )
            ws.send(ack.SerializeToString(), opcode=0x02)
        for item in response.messagesList:
            if item.method != "WebcastChatMessage":
                continue
            chat = live_pb2.ChatMessage()
            chat.ParseFromString(item.payload)
            msg_id = str(item.msgId)
            author_id = str(chat.user.id)
            content = str(chat.content)
            if author_id == self_uid:
                if outcome["reply_attempted"] and content == reply:
                    outcome["self_echo_filtered"] = True
                    emit("self_echo_filtered")
                    stop.set()
                    ws.close()
                continue
            if msg_id == "0" or msg_id in seen or outcome["external_chat"] is not None:
                continue
            seen.add(msg_id)
            outcome["external_chat"] = msg_id
            emit("chat_received", unique=True)
            outcome["reply_attempted"] = True
            try:
                result = DouyinAPI.sendMsgInRoom(auth, str(room["room_id"]), reply)
                status = str((result or {}).get("status_code", (result or {}).get("status", "unknown")))
                emit("reply_attempted", result_status=status)
            except Exception as exc:
                emit("reply_failed", error_type=type(exc).__name__)
                stop.set()
                ws.close()

    def on_error(_ws, error) -> None:
        emit("websocket_error", error_type=type(error).__name__)

    def on_close(_ws, code, _reason) -> None:
        emit("websocket_closed", code=code)
        stop.set()

    app = websocket.WebSocketApp(
        ws_url,
        header={
            "Pragma": "no-cache",
            "Accept-Language": "zh-CN,zh;q=0.9,en;q=0.8",
            "User-Agent": HeaderBuilder.ua,
            "Upgrade": "websocket",
            "Cache-Control": "no-cache",
            "Connection": "Upgrade",
        },
        cookie=auth.cookie_str,
        on_open=on_open,
        on_message=on_message,
        on_error=on_error,
        on_close=on_close,
    )
    timer = threading.Timer(args.timeout, app.close)
    timer.daemon = True
    timer.start()
    try:
        app.run_forever(origin="https://live.douyin.com", sslopt={"cert_reqs": ssl.CERT_REQUIRED}, ping_interval=0)
    finally:
        timer.cancel()
        stop.set()

    if outcome["external_chat"] is None:
        emit("probe_inconclusive", reason="no_unique_external_chat")
        return 2
    if not outcome["reply_attempted"]:
        emit("probe_failed", reason="reply_not_attempted")
        return 3
    if not outcome["self_echo_filtered"]:
        emit("probe_inconclusive", reason="self_echo_not_observed_before_timeout_or_close")
        return 4
    emit("probe_passed", read_one=True, reply_one=True, self_echo_filtered=True)
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except KeyboardInterrupt:
        emit("probe_cancelled")
        raise SystemExit(130)
    except Exception as exc:
        emit("probe_failed", reason=type(exc).__name__)
        raise SystemExit(1)
