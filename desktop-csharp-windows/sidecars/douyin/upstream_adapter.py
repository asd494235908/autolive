"""Memory-only adapter for the pinned DouYin_Spider checkout."""
import base64
from collections import OrderedDict
import gzip
import io
import os
from pathlib import Path
import ssl
import subprocess
import sys
import threading
import time
import types
import unicodedata
from urllib.parse import urlencode

from login_diagnostics import LoginDiagnostics, observe_login_stages

COMMIT = "9afaf79580b1ee84e8954ff906ff26869d5b7f1f"
_operation = threading.local()


class Cancelled(BaseException):
    """Escape upstream broad Exception handlers during owned cancellation."""


class AuthenticationExpired(Exception):
    """An explicit unauthorized response, not an inferred network failure."""


def checkpoint():
    stop = getattr(_operation, "stop", None)
    if stop is not None and stop.is_set():
        raise Cancelled()


def cancellable_sleep(seconds):
    stop = getattr(_operation, "stop", None)
    if stop is None:
        time.sleep(seconds)
    elif stop.wait(seconds):
        raise Cancelled()


def clean_text(value, limit):
    return "".join(ch for ch in str(value or "") if unicodedata.category(ch) != "Cc")[:limit]


class UpstreamAdapter:
    def __init__(self, root):
        root = Path(root).resolve()
        actual = subprocess.check_output(
            ["git", "-C", str(root), "rev-parse", "HEAD"], timeout=10,
            creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0), stderr=subprocess.DEVNULL,
        ).decode().strip()
        if actual != COMMIT:
            raise ValueError("upstream_commit_mismatch")
        if subprocess.check_output(
            ["git", "-C", str(root), "status", "--porcelain", "--untracked-files=no"],
            timeout=10, creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0),
        ).strip():
            raise ValueError("upstream_modified")
        # The upstream QR bootstrap otherwise imports local .env credentials.
        import dotenv
        dotenv.load_dotenv = lambda *args, **kwargs: False
        for name in tuple(os.environ):
            if name.startswith("DY_"):
                os.environ.pop(name)
        from loguru import logger
        logger.remove()
        sys.path.insert(0, str(root))
        from curl_cffi import requests
        original = requests.Session.request

        def verified_request(session, method, url, **kwargs):
            checkpoint()
            kwargs["verify"] = True
            kwargs["timeout"] = 10
            result = original(session, method, url, **kwargs)
            checkpoint()
            diagnostic = getattr(_operation, "diagnostic", None)
            if diagnostic is not None:
                diagnostic.response(result)
            if result.status_code == 401:
                raise AuthenticationExpired()
            return result

        requests.Session.request = verified_request
        from builder.auth import DouyinAuth
        from builder.header import HeaderBuilder
        from dy_apis.douyin_api import DouyinAPI
        from dy_apis import login_api
        from static import Live_pb2
        from utils.dy_util import generate_signature
        original_auth_init = DouyinAuth.__init__

        def owned_auth_init(auth):
            original_auth_init(auth)
            _operation.auth = auth

        DouyinAuth.__init__ = owned_auth_init
        # Only the login module gets an interruptible clock; do not patch global time.
        login_api.time = types.SimpleNamespace(**vars(time))
        login_api.time.sleep = cancellable_sleep
        observe_login_stages(login_api.DYLoginApi, _operation)
        self.Auth, self.API, self.Header = DouyinAuth, DouyinAPI, HeaderBuilder
        self.pb, self.signature = Live_pb2, generate_signature
        self.auth = None
        self.uid = ""
        self.ws = None
        self.seen = OrderedDict()

    def login(self, stop, timeout, on_qr, on_diagnostic=None):
        _operation.stop = stop
        diagnostic = LoginDiagnostics(on_diagnostic or (lambda payload: None))
        _operation.diagnostic = diagnostic
        diagnostic.enter('auth_start')
        import qrcode

        def qr(url):
            checkpoint()
            data = io.BytesIO()
            qrcode.make(url).save(data, format="PNG")
            on_qr(base64.b64encode(data.getvalue()).decode("ascii"), int((time.time() + 60) * 1000))
            diagnostic.record('qr_issued')

        try:
            auth = self.Auth.from_qrcode_login(timeout=timeout, show_qr=False,
                                             on_qrcode=qr, bootstrap_creator=False)
            checkpoint()
            diagnostic.enter('self_identity')
            uid = str(auth.get_uid() or "")
            if not uid or uid == "0":
                diagnostic.record('identity_missing')
                raise ValueError("auth_expired")
            checkpoint()
            self.auth, self.uid = auth, uid
            diagnostic.record('confirmed', final=True)
        except Cancelled:
            diagnostic.record('cancelled', exception_type='cancelled', final=True)
            raise
        except Exception as error:
            diagnostic.failure(error)
            raise
        finally:
            _operation.diagnostic = None
            candidate = getattr(_operation, "auth", None)
            if candidate is not None and candidate is not self.auth:
                if getattr(candidate, "http", None) is not None:
                    candidate.http.close()
            _operation.auth = None

    def logout(self):
        auth, self.auth = self.auth, None
        self.uid = ""
        if auth is not None and getattr(auth, "http", None) is not None:
            auth.http.close()

    def resolve(self, web_rid, stop):
        _operation.stop = stop
        if self.auth is None:
            raise AuthenticationExpired()
        room = self.API.get_live_info(self.auth, web_rid, timeout=10)
        checkpoint()
        if not room or not room.get("room_id") or not room.get("user_id"):
            raise ValueError("room_resolve_failed")
        if str(room.get("room_status")) != "2":
            raise ValueError("room_not_live")
        return room

    def ws_url(self, web_rid, room):
        raw = self.API.get_webcast_detail(self.auth, str(room["user_id"]),
                                         str(room["room_id"]), f"https://live.douyin.com/{web_rid}")
        frame = self.pb.LiveResponse()
        frame.ParseFromString(raw)
        self.seen = OrderedDict((str(item.msgId), None) for item in frame.messagesList if item.msgId)
        params = dict(app_name="douyin_web", version_code="180800", webcast_sdk_version="1.0.15",
                      update_version_code="1.0.15", compress="gzip", device_platform="web",
                      cookie_enabled="true", screen_width="1707", screen_height="960",
                      browser_language="zh-CN", browser_platform="Win32", browser_name="Mozilla",
                      browser_version=self.Header.ua.split("Mozilla/")[-1], browser_online="true",
                      tz_name="Etc/GMT-8", cursor=str(frame.cursor), internal_ext=frame.internalExt,
                      host="https://live.douyin.com", aid="6383", live_id="1", did_rule="3",
                      endpoint="live_pc", support_wrds="1", user_unique_id=str(room["user_id"]),
                      im_path="/webcast/im/fetch/", identity="audience", need_persist_msg_count="15",
                      insert_task_id="", live_reason="", room_id=str(room["room_id"]),
                      heartbeatDuration="0", signature=self.signature(str(room["room_id"]), str(room["user_id"])))
        return "wss://webcast100-ws-web-hl.douyin.com/webcast/im/push/v2/?" + urlencode(params)

    def listen(self, web_rid, room, stop, on_state, on_chat):
        import websocket
        _operation.stop = stop
        url = self.ws_url(web_rid, room)
        checkpoint()
        # One owned receive thread also sends heartbeats; no detached ping worker.
        try:
            ws = websocket.create_connection(url, timeout=10, origin="https://live.douyin.com",
                                             cookie=self.auth.cookie_str,
                                             header={"User-Agent": self.Header.ua},
                                             sslopt={"cert_reqs": ssl.CERT_REQUIRED, "check_hostname": True})
        except websocket.WebSocketBadStatusException as error:
            if error.status_code == 401:
                raise AuthenticationExpired() from None
            raise
        self.ws = ws
        try:
            checkpoint()
            ws.settimeout(1)
            on_state("connected")
            heartbeat = 0.0
            while not stop.is_set():
                if time.monotonic() >= heartbeat:
                    ws.send(self.pb.PushFrame(payloadType="hb").SerializeToString(), opcode=2)
                    heartbeat = time.monotonic() + 5
                try:
                    raw = ws.recv()
                except websocket.WebSocketTimeoutException:
                    continue
                if not raw:
                    break
                self.consume_frame(ws, raw, on_chat)
        finally:
            ws.close(timeout=1)
            self.ws = None

    def consume_frame(self, ws, raw, on_chat):
        if len(raw) > 4 * 1024 * 1024:
            raise ValueError("ws_protocol_invalid")
        frame = self.pb.PushFrame()
        frame.ParseFromString(raw)
        response = self.pb.LiveResponse()
        with gzip.GzipFile(fileobj=io.BytesIO(frame.payload)) as compressed:
            payload = compressed.read(4 * 1024 * 1024 + 1)
        if len(payload) > 4 * 1024 * 1024:
            raise ValueError("ws_protocol_invalid")
        response.ParseFromString(payload)
        if response.needAck:
            ws.send(self.pb.PushFrame(payloadType="ack", payload=response.internalExt.encode(),
                                    logId=frame.logId).SerializeToString(), opcode=2)
        for item in response.messagesList:
            if item.method != "WebcastChatMessage":
                continue
            chat = self.pb.ChatMessage()
            chat.ParseFromString(item.payload)
            content = clean_text(chat.content, 16384)
            if item.msgId and content:
                on_chat(dict(msg_id=str(item.msgId), author_id=str(chat.user.id),
                             nickname=clean_text(chat.user.nickname, 64) or "用户", content=content,
                             received_at_unix_ms=int(time.time() * 1000),
                             is_self=str(chat.user.id) == self.uid,
                             is_replay=str(item.msgId) in self.seen))
                self.seen[str(item.msgId)] = None
                if len(self.seen) > 5000:
                    self.seen.popitem(last=False)

    def close(self):
        ws = self.ws
        if ws is not None:
            ws.close(timeout=1)

    def send(self, room, content, stop):
        _operation.stop = stop
        checkpoint()
        return self.API.sendMsgInRoom(self.auth, str(room["room_id"]), content)
