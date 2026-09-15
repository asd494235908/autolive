"""Offline protocol/lifecycle tests; no account or chat sends to Douyin."""
import gzip
import io
import json
from pathlib import Path
import threading
import unittest

from sidecar import Sidecar, MAX_LINE_BYTES
from upstream_adapter import AuthenticationExpired, Cancelled, UpstreamAdapter


class FakeAdapter:
    def __init__(self):
        self.login_started = threading.Event()
        self.listen_started = threading.Event()
        self.sent = []
        self.logged_out = 0
        self.auth = object()

    def login(self, stop, timeout, qr, diagnostic=None):
        self.login_started.set()
        qr("iVBORw0KGgo=", 1234)
        if stop.wait(2):
            raise Cancelled()

    def resolve(self, room, stop):
        return dict(room_id="internal-room", room_title="直播间")

    def listen(self, room, info, stop, state, chat):
        state("connected")
        for index in range(3):
            chat(dict(msg_id=str(index + 1), author_id="12", nickname="小明",
                      content="你好🙂", received_at_unix_ms=1, is_self=False, is_replay=False))
        self.listen_started.set()
        stop.wait()

    def close(self):
        pass

    def logout(self):
        self.logged_out += 1
        self.auth = None

    def send(self, room, content, stop):
        self.sent.append(content)
        return {"status_code": 0}


def command(op, payload=None, rid="request"):
    return dict(v=1, id=rid, op=op, payload=payload or {})


class ProtocolTests(unittest.TestCase):
    def setUp(self):
        self.output = io.StringIO()
        self.adapter = FakeAdapter()
        self.sidecar = Sidecar(self.adapter, self.output)

    def tearDown(self):
        self.sidecar.stop_auth()
        self.sidecar.stop_live()

    def events(self):
        return [json.loads(line) for line in self.output.getvalue().splitlines()]

    def connect(self):
        self.sidecar.handle(command("live.open", dict(web_rid="123", generation=1)))
        self.assertTrue(self.adapter.listen_started.wait(2))

    def test_watch_keeps_receiving_and_never_sends(self):
        self.connect()
        messages = [e for e in self.events() if e.get("event") == "live.chat"]
        self.assertEqual(3, len(messages))
        self.assertEqual("小明", messages[-1]["payload"]["nickname"])
        self.assertEqual("你好🙂", messages[-1]["payload"]["content"])
        self.assertEqual("123", messages[-1]["payload"]["room_id"])
        self.assertEqual([], self.adapter.sent)
        self.assertTrue(self.sidecar.live_thread.is_alive())
        self.sidecar.handle(command("live.close", dict(session_id=self.sidecar.session, generation=1)))
        self.assertIsNone(self.sidecar.live_thread)

    def test_qr_can_be_cancelled_while_stdin_is_available(self):
        self.sidecar.handle(command("auth.qr.start", dict(timeout_ms=300000)))
        self.assertTrue(self.adapter.login_started.wait(2))
        self.sidecar.handle(command("auth.cancel"))
        self.assertIsNone(self.sidecar.auth_thread)
        states = [e["payload"]["state"] for e in self.events() if e.get("event") == "auth.state"]
        self.assertEqual(["waiting", "cancelled"], states)

    def test_login_diagnostic_is_emitted_before_failure_without_raw_error(self):
        def fail(stop, timeout, qr, diagnostic):
            diagnostic(dict(stage='login_finalize', code='network_error', exception_type='timeout'))
            raise TimeoutError('SECRET credential')
        self.adapter.login = fail
        self.sidecar.login(300)
        events = self.events()
        self.assertEqual('auth.diagnostic', events[0]['event'])
        self.assertEqual('login_finalize', events[0]['payload']['stage'])
        self.assertEqual('expired', events[-1]['payload']['state'])
        self.assertNotIn('SECRET', json.dumps(events))

    def test_send_only_on_explicit_request_once(self):
        self.connect()
        payload = dict(session_id=self.sidecar.session, generation=1,
                       client_action_id="action-1", content="明确发送")
        self.sidecar.handle(command("chat.send", payload, "action-1"))
        self.sidecar.handle(command("chat.send", payload, "action-1"))
        self.assertEqual(["明确发送"], self.adapter.sent)
        self.assertEqual("transport_before_dispatch", self.events()[-1]["error"]["code"])
        payload["generation"] = 2
        with self.assertRaises(ValueError):
            self.sidecar.handle(command("chat.send", payload, "action-1"))

    def test_protocol_bounds_shutdown_and_invalid_input(self):
        stream = io.BytesIO((json.dumps(command("shutdown")) + "\n").encode())
        self.assertEqual(0, self.sidecar.run(stream))
        self.assertEqual(2, self.sidecar.run(io.BytesIO(b"x" * (MAX_LINE_BYTES + 1) + b"\n")))
        with self.assertRaises(ValueError):
            self.sidecar.handle(command("live.open", dict(web_rid="https://other/123", generation=1)))

    def test_long_session_send_evicts_old_id_and_matches_protocol_limit(self):
        self.connect()
        self.sidecar.actions.update((f"old-{index}", None) for index in range(5000))
        payload = dict(session_id=self.sidecar.session, generation=1,
                       client_action_id="new-action", content="🙂" * 100)
        self.sidecar.handle(command("chat.send", payload, "new-action"))
        self.assertEqual(["🙂" * 100], self.adapter.sent)
        self.assertEqual(5000, len(self.sidecar.actions))
        self.assertNotIn("old-0", self.sidecar.actions)
        self.assertIn("new-action", self.sidecar.actions)
        self.sidecar.handle(command("chat.send", payload, "new-action"))
        self.assertEqual(1, len(self.adapter.sent))
        payload.update(client_action_id="too-long", content="🙂" * 101)
        with self.assertRaises(ValueError):
            self.sidecar.handle(command("chat.send", payload, "too-long"))

    def test_unknown_send_is_not_retried(self):
        self.connect()

        def unknown(room, content, stop):
            self.adapter.sent.append(content)
            raise OSError("fake network failure after dispatch")

        self.adapter.send = unknown
        payload = dict(session_id=self.sidecar.session, generation=1,
                       client_action_id="unknown-action", content="一次")
        self.sidecar.handle(command("chat.send", payload, "unknown-action"))
        self.assertEqual("unknown", self.events()[-1]["error"]["outcome"])
        self.sidecar.handle(command("chat.send", payload, "unknown-action"))
        self.assertEqual(["一次"], self.adapter.sent)

    def test_close_reopen_reuses_auth_and_shutdown_clears_it(self):
        self.connect()
        auth = self.adapter.auth
        old_session = self.sidecar.session
        self.sidecar.handle(command("live.close", dict(session_id=old_session, generation=1)))
        self.assertIs(self.adapter.auth, auth)
        self.assertEqual(0, self.adapter.logged_out)
        self.adapter.listen_started.clear()
        self.sidecar.handle(command("live.open", dict(web_rid="456", generation=2)))
        self.assertTrue(self.adapter.listen_started.wait(2))
        self.assertIs(self.adapter.auth, auth)
        self.assertNotEqual(old_session, self.sidecar.session)
        self.assertEqual(2, self.events()[-1]["generation"])
        self.assertFalse(any(e.get("event", "").startswith("auth.") for e in self.events()))
        self.sidecar.handle(command("shutdown"))
        self.assertIsNone(self.adapter.auth)
        self.assertIsNone(self.sidecar.live_thread)

    def test_explicit_auth_expiry_is_not_room_or_network_failure(self):
        def expired(room, stop):
            raise AuthenticationExpired()

        self.adapter.resolve = expired
        self.sidecar.handle(command("live.open", dict(web_rid="123", generation=1)))
        self.sidecar.live_thread.join(2)
        self.assertEqual("auth_expired", self.events()[-1]["error"]["code"])
        self.assertIsNone(self.adapter.auth)

    def test_unrecognized_send_response_is_unknown_not_rejected(self):
        self.connect()
        self.adapter.send = lambda room, content, stop: {"unexpected": "shape"}
        payload = dict(session_id=self.sidecar.session, generation=1,
                       client_action_id="shape-action", content="大家好")
        self.sidecar.handle(command("chat.send", payload, "shape-action"))
        self.assertEqual("unknown", self.events()[-1]["error"]["outcome"])


class PinnedProtobufTests(unittest.TestCase):
    def test_real_pinned_protobuf_chat_ack_and_replay(self):
        root = Path(__file__).resolve().parents[2] / ".tools" / "douyin-upstream"
        if not root.is_dir():
            self.skipTest("pinned upstream checkout is not provisioned")
        adapter = UpstreamAdapter(root)
        adapter.uid = "99"
        pb = adapter.pb
        response = pb.LiveResponse(needAck=True, internalExt="ack-token")
        for mid, uid in [(101, 12), (102, 99)]:
            chat = pb.ChatMessage(content="你好🙂", user=pb.User(id=uid, nickname="真实字段"))
            response.messagesList.add(method="WebcastChatMessage", msgId=mid, payload=chat.SerializeToString())
        response.messagesList.add(method="WebcastLikeMessage", msgId=103)
        frame = pb.PushFrame(logId=9, payload=gzip.compress(response.SerializeToString()))
        calls = []
        ws = type("Socket", (), {"send": lambda self, raw, opcode: calls.append((raw, opcode))})()
        received = []
        adapter.consume_frame(ws, frame.SerializeToString(), received.append)
        self.assertEqual(2, len(received))
        self.assertEqual("真实字段", received[0]["nickname"])
        self.assertTrue(received[1]["is_self"])
        self.assertFalse(received[0]["is_replay"])
        adapter.consume_frame(ws, frame.SerializeToString(), received.append)
        self.assertTrue(received[-1]["is_replay"])
        ack = pb.PushFrame.FromString(calls[0][0])
        self.assertEqual("ack", ack.payloadType)
        self.assertEqual(b"ack-token", ack.payload)


if __name__ == "__main__":
    unittest.main()
