"""Canonical NDJSON v1 process; watch mode never originates chat messages."""
import argparse
from collections import OrderedDict
import json
import os
import re
import sys
import threading
import traceback
import uuid

from upstream_adapter import AuthenticationExpired, Cancelled, UpstreamAdapter, clean_text

MAX_LINE_BYTES = 64 * 1024


def valid_id(value):
    return isinstance(value, str) and 1 <= len(value) <= 128 and all(33 <= ord(c) <= 126 for c in value)


class Sidecar:
    def __init__(self, adapter, output):
        self.adapter, self.output = adapter, output
        self.write_lock = threading.Lock()
        self.auth_stop, self.live_stop = threading.Event(), threading.Event()
        self.auth_thread = self.live_thread = None
        self.session = None
        self.room = None
        self.generation = 0
        self.connected = False
        self.actions = OrderedDict()

    def emit(self, value):
        line = json.dumps(dict(v=1, **value), ensure_ascii=False, separators=(",", ":"))
        if len(line.encode("utf-8")) > MAX_LINE_BYTES:
            return False
        with self.write_lock:
            self.output.write(line + "\n")
            self.output.flush()
        return True

    def response(self, request_id, result=None, code=None, outcome="not_sent"):
        value = dict(type="response", request_id=request_id, ok=code is None)
        if code is None:
            value["result"] = result or {}
        else:
            value["error"] = dict(code=code, message=code, retryable=False, outcome=outcome)
        self.emit(value)

    def event(self, name, payload, live=False):
        value = dict(type="event", event=name, payload=payload)
        if live:
            value.update(session_id=self.session, generation=self.generation)
        self.emit(value)

    def stop_auth(self):
        self.auth_stop.set()
        if self.auth_thread is not None:
            self.auth_thread.join(12)
            if self.auth_thread.is_alive():
                raise RuntimeError("sidecar_exited")
            self.auth_thread = None

    def stop_live(self):
        self.live_stop.set()
        self.adapter.close()
        if self.live_thread is not None:
            self.live_thread.join(12)
            if self.live_thread.is_alive():
                raise RuntimeError("sidecar_exited")
            self.live_thread = None
        self.connected = False
        self.room = None

    def login(self, timeout):
        try:
            self.adapter.login(self.auth_stop, timeout,
                               lambda png, expiry: self.event("auth.qr", dict(png_base64=png, expires_at_unix_ms=expiry)),
                               lambda payload: self.event("auth.diagnostic", payload))
            if not self.auth_stop.is_set():
                self.event("auth.state", dict(state="confirmed"))
        except Cancelled:
            pass
        except TimeoutError:
            self.event("auth.state", dict(state="expired"))
        except Exception as error:
            if not self.auth_stop.is_set():
                sys.stderr.write("douyin_auth_failed:" + type(error).__name__ + "\n")
                sys.stderr.write(" ".join(f"{frame.name}:{frame.lineno}" for frame in traceback.extract_tb(error.__traceback__)) + "\n")
                self.event("auth.state", dict(state="failed"))

    def listen(self, request_id, web_rid):
        responded = False
        try:
            self.room = self.adapter.resolve(web_rid, self.live_stop)
            if self.live_stop.is_set():
                return
            self.response(request_id, dict(session_id=self.session,
                          title=clean_text(self.room.get("room_title"), 128) or "抖音直播间", live_status="connecting"))
            responded = True

            def state(value):
                if not self.live_stop.is_set():
                    self.connected = value == "connected"
                    self.event("live.state", dict(state=value), live=True)

            def chat(value):
                if not self.live_stop.is_set():
                    self.event("live.chat", dict(value, room_id=web_rid), live=True)

            self.adapter.listen(web_rid, self.room, self.live_stop, state, chat)
            state("closed")
        except Cancelled:
            pass
        except AuthenticationExpired:
            if not self.live_stop.is_set():
                self.adapter.logout()
                if responded:
                    self.event("live.state", dict(state="auth_expired"), live=True)
                else:
                    self.response(request_id, code="auth_expired")
        except Exception as error:
            if not self.live_stop.is_set():
                if responded:
                    self.event("live.state", dict(state="failed"), live=True)
                else:
                    code = str(error) if str(error) in {"auth_expired", "room_not_live", "room_resolve_failed"} else "room_resolve_failed"
                    self.response(request_id, code=code)
        finally:
            self.connected = False

    def handle(self, request):
        if not isinstance(request, dict) or set(request) != {"v", "id", "op", "payload"} or type(request.get("v")) is not int or request.get("v") != 1 or not valid_id(request.get("id")) or not isinstance(request.get("payload"), dict):
            raise ValueError("protocol_invalid")
        rid, op, payload = request["id"], request["op"], request["payload"]
        if op == "auth.qr.start":
            if payload != {"timeout_ms": 300000}:
                raise ValueError("protocol_invalid")
            self.stop_auth()
            self.stop_live()
            self.adapter.logout()
            self.auth_stop = threading.Event()
            self.response(rid)
            self.event("auth.state", dict(state="waiting"))
            self.auth_thread = threading.Thread(target=self.login, args=(300,), name="douyin-auth")
            self.auth_thread.start()
        elif op in {"auth.cancel", "auth.logout", "shutdown"}:
            if payload:
                raise ValueError("protocol_invalid")
            self.stop_auth()
            if op != "auth.cancel":
                self.stop_live()
                self.adapter.logout()
            self.response(rid)
            if op == "auth.cancel":
                self.event("auth.state", dict(state="cancelled"))
            return op != "shutdown"
        elif op == "live.open":
            if set(payload) != {"web_rid", "generation"} or not isinstance(payload["web_rid"], str) or not re.fullmatch(r"[0-9]{1,20}", payload["web_rid"]) or type(payload["generation"]) is not int or not 0 < payload["generation"] < 2**64:
                raise ValueError("protocol_invalid")
            self.stop_live()
            self.session, self.generation = uuid.uuid4().hex, payload["generation"]
            self.actions.clear()
            self.live_stop = threading.Event()
            self.live_thread = threading.Thread(target=self.listen, args=(rid, payload["web_rid"]), name="douyin-live")
            self.live_thread.start()
        elif op in {"live.close", "chat.send"}:
            expected = {"session_id", "generation"} | ({"client_action_id", "content"} if op == "chat.send" else set())
            if set(payload) != expected or not valid_id(payload["session_id"]) or payload["session_id"] != self.session or type(payload["generation"]) is not int or payload["generation"] != self.generation:
                raise ValueError("protocol_invalid")
            if op == "live.close":
                self.stop_live()
                self.event("live.state", dict(state="closed"), live=True)
                self.response(rid)
            else:
                content, action = payload["content"], payload["client_action_id"]
                if not valid_id(action) or rid != action or not isinstance(content, str) or not 1 <= len(content.strip()) <= 100 or len(content.encode("utf-8")) > 400 or not all(c.isprintable() for c in content):
                    raise ValueError("protocol_invalid")
                if not self.connected or self.room is None or action in self.actions:
                    self.response(rid, code="transport_before_dispatch")
                else:
                    self.actions[action] = None
                    if len(self.actions) > 5000:
                        self.actions.popitem(last=False)
                    try:
                        result = self.adapter.send(self.room, content, self.live_stop)
                        status = result.get("status_code") if isinstance(result, dict) else None
                        if type(status) is int and status == 0:
                            self.response(rid, dict(state="accepted", client_action_id=action, platform_status_code=0))
                        elif type(status) is int:
                            self.response(rid, code="send_rejected", outcome="rejected")
                        else:
                            self.response(rid, code="transport_after_dispatch", outcome="unknown")
                    except AuthenticationExpired:
                        self.stop_live()
                        self.adapter.logout()
                        self.response(rid, code="auth_expired", outcome="rejected")
                        self.event("live.state", dict(state="auth_expired"), live=True)
                    except Exception:
                        self.response(rid, code="transport_after_dispatch", outcome="unknown")
        else:
            raise ValueError("protocol_invalid")
        return True

    def run(self, source):
        try:
            while True:
                raw = source.readline(MAX_LINE_BYTES + 2)
                if not raw:
                    return 0
                if len(raw.rstrip(b"\r\n")) > MAX_LINE_BYTES or not raw.endswith(b"\n"):
                    return 2
                request = None
                try:
                    request = json.loads(raw)
                    if not self.handle(request):
                        return 0
                except (ValueError, TypeError, KeyError):
                    if isinstance(request, dict) and valid_id(request.get("id")):
                        self.response(request["id"], code="protocol_invalid")
                    else:
                        return 2
        finally:
            self.stop_auth()
            self.stop_live()
            self.adapter.logout()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--upstream-root", required=True)
    args = parser.parse_args()
    # Keep all incidental upstream print/debug output away from protocol and logs.
    protocol = sys.stdout
    protocol.reconfigure(encoding="utf-8", newline="\n")
    with open(os.devnull, "w", encoding="utf-8") as quiet:
        sys.stdout = quiet
        try:
            adapter = UpstreamAdapter(args.upstream_root)
            return Sidecar(adapter, protocol).run(sys.stdin.buffer)
        except Exception as error:
            sys.stderr.write("douyin_sidecar_failed:" + type(error).__name__ + "\n")
            return 1
        finally:
            sys.stdout = protocol


if __name__ == "__main__":
    raise SystemExit(main())
