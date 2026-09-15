"""Explicit network smoke: prepare then cancel QR; never scan or send chat."""
import json
from pathlib import Path
import subprocess
import sys
import threading


def main():
    entry = Path(__file__).with_name("sidecar.py")
    root = Path(__file__).resolve().parents[2] / ".tools" / "douyin-upstream"
    process = subprocess.Popen(
        [sys.executable, "-u", str(entry), "--upstream-root", str(root)],
        stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        text=True, encoding="utf-8", creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
    ready = threading.Event()
    outcome = {"qr_prepared": False, "forced_exit": False}

    def read():
        try:
            for line in process.stdout:
                value = json.loads(line)
                if value.get("event") == "auth.qr":
                    outcome["qr_prepared"] = True
                    ready.set()
                elif value.get("event") == "auth.state" and value["payload"]["state"] == "failed":
                    ready.set()
        finally:
            ready.set()

    reader = threading.Thread(target=read)
    reader.start()
    try:
        process.stdin.write(json.dumps(dict(v=1, id="qr-check", op="auth.qr.start", payload=dict(timeout_ms=300000))) + "\n")
        process.stdin.flush()
        ready.wait(60)
        if process.poll() is None:
            process.stdin.write(json.dumps(dict(v=1, id="stop-check", op="shutdown", payload={})) + "\n")
            process.stdin.flush()
        try:
            process.wait(timeout=20)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
            outcome["forced_exit"] = True
    finally:
        if process.poll() is None:
            process.kill()
            process.wait()
        reader.join(2)
        process.stdin.close()
        process.stdout.close()
    outcome["exit_code"] = process.returncode
    outcome["diagnostic"] = process.stderr.read(512)
    process.stderr.close()
    print(json.dumps(outcome))
    return 0 if outcome["qr_prepared"] and process.returncode == 0 and not outcome["forced_exit"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
