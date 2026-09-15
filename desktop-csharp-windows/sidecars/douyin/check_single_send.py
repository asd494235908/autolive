"""Explicit live acceptance: one QR login, reconnect, one greeting, self echo."""
import argparse
import base64
import json
from pathlib import Path
import queue
import subprocess
import sys
import threading
import time
import uuid


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--room-id", required=True)
    parser.add_argument("--qr-output", required=True, type=Path)
    parser.add_argument("--send-once", required=True, choices=["大家好"])
    args = parser.parse_args()
    if not args.room_id.isascii() or not args.room_id.isdigit() or not 1 <= len(args.room_id) <= 20:
        parser.error("room-id must contain 1 to 20 ASCII digits")
    qr_path = args.qr_output.resolve()
    if qr_path.exists() or qr_path.suffix.lower() != ".png" or not qr_path.parent.is_dir():
        parser.error("qr-output must be a new PNG in an existing directory")
    entry = Path(__file__).with_name("sidecar.py")
    root = Path(__file__).resolve().parents[2] / ".tools" / "douyin-upstream"
    events = queue.Queue(maxsize=500)
    stop = threading.Event()
    created_qr = False
    generation = 1
    report = dict(first_chat_count=0, second_chat_count=0, qr_count=0,
                  reconnect_qr_count=0, sent_once=False, send_state="not_sent", self_echo=False,
                  forced_exit=False)
    sent_at = 0
    process = subprocess.Popen(
        [sys.executable, "-u", str(entry), "--upstream-root", str(root)],
        stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
        text=True, encoding="utf-8", creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))

    def read():
        for line in process.stdout:
            value = json.loads(line)
            while not stop.is_set():
                try:
                    events.put(value, timeout=0.1)
                    break
                except queue.Full:
                    continue

    reader = threading.Thread(target=read, name="single-send-events")
    reader.start()

    def progress(stage, **values):
        print(json.dumps(dict(stage=stage, **values), ensure_ascii=False), flush=True)

    def send(op, payload=None):
        rid = uuid.uuid4().hex
        if op == "chat.send":
            payload["client_action_id"] = rid
        process.stdin.write(json.dumps(dict(v=1, id=rid, op=op, payload=payload or {}), ensure_ascii=False) + "\n")
        process.stdin.flush()
        return rid

    def wait_for(predicate, seconds, optional=False):
        nonlocal created_qr
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            try:
                value = events.get(timeout=min(0.25, max(0.01, deadline - time.monotonic())))
            except queue.Empty:
                if process.poll() is not None:
                    raise RuntimeError("sidecar_exited")
                continue
            event, payload = value.get("event"), value.get("payload", {})
            if event == "auth.qr":
                png = base64.b64decode(payload["png_base64"], validate=True)
                if not png.startswith(b"\x89PNG\r\n\x1a\n") or len(png) > 65536:
                    raise RuntimeError("invalid_qr")
                with qr_path.open("wb" if created_qr else "xb") as output:
                    created_qr = True
                    output.write(png)
                report["qr_count"] += 1
                if generation == 2:
                    report["reconnect_qr_count"] += 1
                progress("qr_ready", path=str(qr_path))
            elif event == "live.chat":
                key = "first_chat_count" if value.get("generation") == 1 else "second_chat_count"
                report[key] += 1
                if report["sent_once"] and value.get("generation") == 2 and value.get("session_id") == session and payload.get("is_self") and not payload.get("is_replay") and payload.get("content") == "大家好" and payload.get("received_at_unix_ms", 0) >= sent_at:
                    report["self_echo"] = True
            elif event == "auth.state" and payload.get("state") in {"failed", "expired", "cancelled"}:
                raise RuntimeError("authentication_unavailable")
            elif event == "live.state" and payload.get("state") in {"failed", "auth_expired", "risk_controlled", "room_ended"}:
                raise RuntimeError("live_unavailable")
            if predicate(value):
                return value
        if not optional:
            raise TimeoutError("acceptance_timeout")
        return None

    def response(rid, seconds=30):
        return wait_for(lambda value: value.get("request_id") == rid, seconds)

    def open_room():
        rid = send("live.open", dict(web_rid=args.room_id, generation=generation))
        result = response(rid)
        if not result.get("ok"):
            raise RuntimeError("live_open_rejected")
        session = result["result"]["session_id"]
        wait_for(lambda value: value.get("event") == "live.state" and value.get("session_id") == session and value["payload"]["state"] == "connected", 30)
        return session

    try:
        progress("waiting_for_login")
        rid = send("auth.qr.start", dict(timeout_ms=300000))
        if not response(rid).get("ok"):
            raise RuntimeError("auth_start_rejected")
        wait_for(lambda value: value.get("event") == "auth.state" and value["payload"]["state"] == "confirmed", 310)
        progress("login_confirmed")
        session = open_room()
        progress("first_connected")
        wait_for(lambda value: value.get("event") == "live.chat", 60, optional=True)
        rid = send("live.close", dict(session_id=session, generation=1))
        if not response(rid).get("ok"):
            raise RuntimeError("live_close_rejected")
        generation = 2
        session = open_room()
        progress("reconnected_without_login", observed_count=report["first_chat_count"])
        sent_at = int(time.time() * 1000)
        # Set before writing: an uncertain write/response is never retried.
        report["sent_once"] = True
        rid = send("chat.send", dict(session_id=session, generation=2, content="大家好"))
        value = response(rid, 30)
        report["send_state"] = value.get("result", {}).get("state") if value.get("ok") else value.get("error", {}).get("outcome", "unknown")
        progress("send_response", state=report["send_state"])
        if not report["self_echo"]:
            wait_for(lambda value: report["self_echo"], 30, optional=True)
    except Exception as error:
        if report["sent_once"] and report["send_state"] == "not_sent":
            report["send_state"] = "unknown"
        report["failure_type"] = type(error).__name__
    finally:
        if process.poll() is None:
            try:
                send("shutdown")
                process.wait(timeout=20)
            except (OSError, subprocess.TimeoutExpired):
                process.kill()
                process.wait()
                report["forced_exit"] = True
        stop.set()
        reader.join(2)
        report["reader_stopped"] = not reader.is_alive()
        process.stdin.close()
        process.stdout.close()
        if created_qr:
            qr_path.unlink(missing_ok=True)
        report["exit_code"] = process.returncode
        progress("completed", **report)
    return 0 if report["send_state"] == "accepted" and report["self_echo"] and report["reconnect_qr_count"] == 0 and process.returncode == 0 and not report["forced_exit"] and report["reader_stopped"] and "failure_type" not in report else 1


if __name__ == "__main__":
    raise SystemExit(main())
