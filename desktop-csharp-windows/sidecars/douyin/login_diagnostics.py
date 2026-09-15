"""Bounded allowlisted login telemetry; never serialize upstream error text."""
import ssl
from functools import wraps

from curl_cffi.requests import exceptions as http_errors

STAGES = frozenset(('auth_start', 'qr_fetch', 'qr_poll', 'login_finalize', 'self_identity'))
CODES = frozenset(('started', 'qr_issued', 'scanned', 'confirmed', 'cancelled', 'expired',
                   'http_error', 'platform_error', 'invalid_response', 'network_error',
                   'internal_error', 'verification_required', 'rate_limited', 'identity_missing'))


class LoginDiagnostics:
    def __init__(self, emit):
        self.emit = emit
        self.stage = 'auth_start'
        self.seen = set()

    def record(self, code, *, exception_type=None, http_status=None, platform_code=None, final=False):
        if code not in CODES or self.stage not in STAGES:
            raise ValueError('diagnostic_invalid')
        payload = dict(stage=self.stage, code=code)
        if exception_type in {'timeout', 'connect', 'tls', 'http', 'protocol', 'cancelled', 'other'}:
            payload['exception_type'] = exception_type
        if type(http_status) is int and 100 <= http_status <= 599:
            payload['http_status'] = http_status
        if type(platform_code) is int and -(2**31) <= platform_code < 2**31:
            payload['platform_code'] = platform_code
        key = tuple(payload.items())
        if key in self.seen or len(self.seen) >= 64 or (len(self.seen) >= 63 and not final):
            return
        self.seen.add(key)
        self.emit(payload)

    def enter(self, stage):
        if stage not in STAGES:
            raise ValueError('diagnostic_invalid')
        self.stage = stage
        self.record('started')

    def poll(self, result):
        data = result.get('data') if isinstance(result, dict) else None
        status = data.get('status') if isinstance(data, dict) else None
        if status in ('scanned', 'confirmed', 'expired'):
            self.record(status)

    def response(self, response):
        status = response.status_code
        headers = {key.lower() for key in response.headers if isinstance(key, str)}
        if headers & {'x-vc-bdturing-parameters', 'x-tt-verify-passport-decision'}:
            self.record('verification_required', http_status=status)
        elif status == 429:
            self.record('rate_limited', http_status=status)
        elif type(status) is int and status >= 400:
            self.record('http_error', http_status=status)
        # Login redirects legitimately return HTML. Only inspect bounded JSON,
        # and select numeric error fields; never copy its description or payload.
        if len(response.content) > 64 * 1024:
            return
        try:
            result = response.json()
        except (ValueError, TypeError):
            if self.stage in ('qr_fetch', 'qr_poll', 'self_identity'):
                self.record('invalid_response', http_status=status)
            return
        if not isinstance(result, dict):
            return
        data = result.get('data')
        for source in (result, data if isinstance(data, dict) else {}):
            for field in ('error_code', 'status_code'):
                code = source.get(field)
                if type(code) is int and -(2**31) <= code < 2**31 and code != 0:
                    self.record('rate_limited' if code == 7 else 'platform_error',
                                http_status=status, platform_code=code)

    def failure(self, error):
        if isinstance(error, (TimeoutError, http_errors.Timeout)):
            category = 'timeout'
        elif isinstance(error, (ssl.SSLError, http_errors.SSLError)):
            category = 'tls'
        elif isinstance(error, (ConnectionError, http_errors.ConnectionError, http_errors.ProxyError)):
            category = 'connect'
        elif isinstance(error, http_errors.HTTPError):
            category = 'http'
        elif isinstance(error, (ValueError, TypeError, KeyError)):
            category = 'protocol'
        else:
            category = 'other'
        code = 'expired' if type(error) is TimeoutError else (
            'network_error' if category in ('timeout', 'tls', 'connect', 'http') else 'internal_error')
        self.record(code, exception_type=category, final=True)


def observe_login_stages(login_api, operation):
    """Observe the pinned upstream's existing calls, preserving arguments/results."""
    for name, stage in (('get_qrcode', 'qr_fetch'), ('check_qrcode', 'qr_poll'),
                        ('_follow_login_redirect', 'login_finalize')):
        original = getattr(login_api, name)

        @wraps(original)
        def observed(api, *args, _original=original, _stage=stage, **kwargs):
            diagnostic = getattr(operation, 'diagnostic', None)
            if diagnostic is not None:
                diagnostic.enter(_stage)
            result = _original(api, *args, **kwargs)
            if diagnostic is not None and _stage == 'qr_poll':
                diagnostic.poll(result)
            return result

        setattr(login_api, name, observed)
