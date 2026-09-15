"""Login telemetry tests: no network or account access."""
import json
import ssl
import threading
import types
import unittest

from login_diagnostics import LoginDiagnostics, observe_login_stages
from upstream_adapter import UpstreamAdapter, _operation


class Response:
    status_code = 200
    headers = {}
    content = b'{}'

    def __init__(self, body=None, status=200, headers=None):
        self.body = body
        self.status_code = status
        self.headers = headers or {}

    def json(self):
        return self.body


class LoginDiagnosticsTests(unittest.TestCase):
    def setUp(self):
        self.events = []
        self.diagnostic = LoginDiagnostics(self.events.append)

    def test_poll_records_changes_without_repeating_success_or_secrets(self):
        self.diagnostic.enter('qr_poll')
        for _ in range(100):
            self.diagnostic.poll({'data': {'status': 'scanned', 'token': 'SECRET'}})
        self.diagnostic.poll({'data': {'status': 'confirmed', 'redirect_url': 'SECRET'}})
        self.assertEqual(['started', 'scanned', 'confirmed'], [x['code'] for x in self.events])
        self.assertNotIn('SECRET', json.dumps(self.events))

    def test_response_retains_only_numeric_codes_and_verification_presence(self):
        self.diagnostic.enter('qr_poll')
        self.diagnostic.response(Response({'data': {'error_code': 7, 'description': 'SECRET'}}))
        self.assertEqual('rate_limited', self.events[-1]['code'])
        self.assertEqual(7, self.events[-1]['platform_code'])
        self.diagnostic.response(Response(headers={'X-Tt-Verify-Passport-Decision': 'SECRET'}))
        self.assertEqual('verification_required', self.events[-1]['code'])
        self.assertNotIn('SECRET', json.dumps(self.events))

    def test_invalid_codes_and_raw_exception_are_not_logged(self):
        self.diagnostic.enter('self_identity')
        for code in (True, 'SECRET', 2**40):
            self.diagnostic.response(Response({'error_code': code}))
        self.diagnostic.failure(ValueError('cookie=SECRET'))
        self.assertEqual('protocol', self.events[-1]['exception_type'])
        self.assertEqual('self_identity', self.events[-1]['stage'])
        self.assertNotIn('SECRET', json.dumps(self.events))
        self.assertTrue(all('platform_code' not in event for event in self.events))

    def test_timeout_and_tls_have_fixed_categories_and_bounded_events(self):
        self.diagnostic.enter('login_finalize')
        self.diagnostic.failure(ssl.SSLError('SECRET'))
        self.assertEqual('tls', self.events[-1]['exception_type'])
        self.diagnostic.failure(TimeoutError('SECRET'))
        self.assertEqual('timeout', self.events[-1]['exception_type'])
        for index in range(1000):
            self.diagnostic.enter('qr_poll' if index % 2 else 'qr_fetch')
        self.assertLessEqual(len(self.events), 64)

    def test_observer_preserves_existing_call_results_and_failure_stage(self):
        class LoginApi:
            def get_qrcode(self, value):
                return value

            def check_qrcode(self, value):
                return value

            def _follow_login_redirect(self, value):
                raise ConnectionError('SECRET')

        operation = types.SimpleNamespace(diagnostic=self.diagnostic)
        observe_login_stages(LoginApi, operation)
        api = LoginApi()
        result = {'data': {'status': 'confirmed', 'token': 'SECRET'}}
        self.assertIs(result, api.get_qrcode(result))
        self.assertIs(result, api.check_qrcode(result))
        with self.assertRaises(ConnectionError):
            api._follow_login_redirect('SECRET')
        self.assertEqual('login_finalize', self.diagnostic.stage)
        self.assertNotIn('SECRET', json.dumps(self.events))

    def test_confirmed_phone_but_missing_identity_records_exact_stage(self):
        class Auth:
            @staticmethod
            def from_qrcode_login(**kwargs):
                return types.SimpleNamespace(get_uid=lambda: None)

        adapter = UpstreamAdapter.__new__(UpstreamAdapter)
        adapter.Auth = Auth
        adapter.auth = None
        with self.assertRaises(ValueError):
            adapter.login(threading.Event(), 300, lambda *args: None, self.events.append)
        self.assertIn({'stage': 'self_identity', 'code': 'identity_missing'}, self.events)
        self.assertIsNone(_operation.diagnostic)

    def test_final_failure_is_retained_after_telemetry_budget(self):
        for code in range(100):
            self.diagnostic.response(Response({'error_code': code}))
        self.diagnostic.failure(ValueError('SECRET'))
        self.assertEqual(64, len(self.events))
        self.assertEqual('internal_error', self.events[-1]['code'])


if __name__ == '__main__':
    unittest.main()
