"""One bounded Chat Completions request with safe, structured diagnostics.

Retries and their request budget belong to the host. A truncated response never
returns a message, so a partial program or tool argument cannot be executed.
"""
import http.client
import json
import math
import re
import socket
import ssl
import time
import pathlib
import runpy
import urllib.error
import urllib.request


MAX_RESPONSE_BYTES = 8 * 1024 * 1024
MAX_EXPANDED_RESPONSE_BYTES = 32 * 1024 * 1024
MODEL_CAPABILITIES = runpy.run_path(str(pathlib.Path(__file__).with_name('model_capabilities.py')))
RETRYABLE_HTTP_STATUSES = frozenset((429, 500, 502, 503, 504))
# Preparation uses MAX even when resuming a checkpoint made under an older policy.
PREPARATION_REASONING_EFFORT = 'max'


def read_capabilities(config, timeout=20):
    return MODEL_CAPABILITIES['read_capabilities'](config, timeout=timeout)


def response_byte_limit(max_tokens):
    """Bound wide/escaped JSON output without assuming a model's token limit."""
    if type(max_tokens) is not int or max_tokens < 1:
        return MAX_RESPONSE_BYTES
    return min(MAX_EXPANDED_RESPONSE_BYTES, max(MAX_RESPONSE_BYTES, max_tokens*32 + 1024*1024))


class CompletionError(RuntimeError):
    """A safe failure description; never contains a provider body or headers."""

    def __init__(self, kind, retryable=False, *, http_status=None,
                 finish_reason=None, usage=None, elapsed_ms=0, request_id=None,
                 max_tokens=None):
        self.kind = kind
        self.retryable = bool(retryable)
        self.http_status = http_status
        self.finish_reason = finish_reason
        self.usage = {} if usage is None else usage
        self.elapsed_ms = elapsed_ms
        self.request_id = request_id
        self.max_tokens = max_tokens
        self.diagnostic = dict(kind=kind, retryable=self.retryable,
                               http_status=http_status, finish_reason=finish_reason,
                               usage=self.usage, elapsed_ms=elapsed_ms,
                               request_id=request_id, max_tokens=max_tokens)
        descriptions = {
            'configuration': 'Invalid AI request configuration',
            'length': 'Incomplete AI response: length',
            'timeout': 'AI request timed out',
            'network': 'AI network connection failed',
            'certificate': 'AI TLS certificate verification failed',
            'protocol': 'Invalid AI completion response',
            'response_too_large': 'AI response exceeded the response body limit',
            'finish_reason': 'AI response did not complete successfully',
            'transport': 'AI request failed',
        }
        super().__init__('AI HTTP status ' + str(http_status) if kind == 'http'
                         else descriptions.get(kind, 'AI request failed'))


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *args, **kwargs):
        return None


def _safe_json(value, secret):
    """Preserve JSON usage extensions, while excluding the configured secret."""
    if isinstance(value, dict):
        return {key.replace(secret, '[redacted]') if secret else key:
                _safe_json(item, secret) for key, item in value.items()}
    if isinstance(value, list):
        return [_safe_json(item, secret) for item in value]
    if isinstance(value, str) and secret:
        return value.replace(secret, '[redacted]')
    return value


def _request_id(value, secret):
    if not isinstance(value, str) or not value or (secret and secret in value):
        return None
    return value if len(value) <= 160 and re.fullmatch(r'[A-Za-z0-9_.:/-]+', value) else None


def _decode(raw):
    # Python's default decoder also accepts NaN and Infinity, which cannot be
    # persisted as interoperable JSON diagnostics.
    def invalid_constant(value):
        raise ValueError('Non-JSON number')
    return json.loads(raw, parse_constant=invalid_constant)


def complete(config, messages, tools, max_tokens=12000, timeout=300):
    """Perform exactly one official API request and return its accepted message.

    The caller supplies the complete, stable tool schema in every phase. The
    result includes usage (including provider cache/reasoning fields), transport
    metadata, and the message needed to preserve the current reasoning context.
    Only the message contains model content; diagnostics never contain it.
    """
    started = time.monotonic()
    status = None
    finish_reason = None
    usage = {}
    request_id = None
    secret = config.get('token', '') if isinstance(config, dict) else ''
    secret = secret if isinstance(secret, str) else ''
    response_limit = response_byte_limit(max_tokens)

    def failure(kind, retryable=False):
        return CompletionError(kind, retryable, http_status=status,
                               finish_reason=finish_reason, usage=usage,
                               elapsed_ms=max(0, round((time.monotonic() - started) * 1000)),
                               request_id=request_id,
                               max_tokens=max_tokens if type(max_tokens) is int else None)

    def metadata(value):
        nonlocal finish_reason, usage, request_id
        if not isinstance(value, dict):
            return
        if isinstance(value.get('usage'), dict):
            usage = _safe_json(value['usage'], secret)
        request_id = _request_id(value.get('id'), secret) or request_id
        choices = value.get('choices')
        if isinstance(choices, list) and choices and isinstance(choices[0], dict):
            reason = choices[0].get('finish_reason')
            # Finish reasons are protocol identifiers, not free-form error text.
            if (isinstance(reason, str) and re.fullmatch(r'[a-z_]{1,64}', reason)
                    and not (secret and secret in reason)):
                finish_reason = reason

    try:
        if (not isinstance(config, dict) or config.get('provider') not in ('deepseek', 'openai')
                or not isinstance(config.get('model'), str) or not config['model'].strip()
                or not secret or type(max_tokens) is not int or max_tokens < 1
                or isinstance(timeout, bool) or not isinstance(timeout, (int, float))
                or not math.isfinite(timeout) or timeout <= 0):
            raise failure('configuration')
        payload = dict(model=config['model'], messages=messages, tools=tools)
        if config['provider'] == 'deepseek':
            endpoint = 'https://api.deepseek.com/chat/completions'
            payload.update(max_tokens=max_tokens, thinking={'type': 'enabled'},
                           reasoning_effort=PREPARATION_REASONING_EFFORT)
        else:
            endpoint = 'https://api.openai.com/v1/chat/completions'
            payload.update(max_completion_tokens=max_tokens, tool_choice='auto')
        try:
            encoded = json.dumps(payload, ensure_ascii=False, allow_nan=False).encode('utf-8')
            request = urllib.request.Request(endpoint, data=encoded, headers={
                'Authorization': 'Bearer ' + secret, 'Content-Type': 'application/json',
                'Connection': 'close'})
        except (TypeError, ValueError, UnicodeError):
            raise failure('configuration') from None
        try:
            with urllib.request.build_opener(NoRedirect()).open(request, timeout=timeout) as response:
                try:
                    candidate_status = getattr(response, 'status', None)
                    if type(candidate_status) is not int:
                        candidate_status = response.getcode()
                    # A few embedded test/opening adapters do not expose status.
                    status = candidate_status if type(candidate_status) is int else 200
                    headers = getattr(response, 'headers', None)
                    if headers is not None:
                        request_id = _request_id(headers.get('x-request-id'), secret)
                    raw = response.read(response_limit + 1)
                finally:
                    response.close()
        except urllib.error.HTTPError as error:
            status = error.code
            try:
                # Selected JSON metadata is useful for billed/truncated failures;
                # the provider's free-form error body is never retained or shown.
                request_id = _request_id(error.headers.get('x-request-id'), secret) if error.headers else None
                try:
                    raw_error = error.read(response_limit + 1)
                    if len(raw_error) <= response_limit:
                        metadata(_decode(raw_error))
                except Exception:
                    pass
            finally:
                error.close()
            raise failure('http', status in RETRYABLE_HTTP_STATUSES) from None
        if not 200 <= status < 300:
            try:
                if isinstance(raw, bytes) and len(raw) <= response_limit:
                    metadata(_decode(raw))
            except Exception:
                pass
            raise failure('http', status in RETRYABLE_HTTP_STATUSES)
        if not isinstance(raw, bytes):
            raise failure('protocol')
        if len(raw) > response_limit:
            raise failure('response_too_large')
        try:
            value = _decode(raw)
        except (ValueError, UnicodeError):
            raise failure('protocol') from None
        metadata(value)
        # Check truncation before inspecting any tool arguments. The host may
        # retry this same request once with more output space.
        if finish_reason == 'length':
            raise failure('length', True)
        if finish_reason not in ('stop', 'tool_calls'):
            raise failure('finish_reason' if finish_reason else 'protocol')
        try:
            message = value['choices'][0]['message']
            if not isinstance(message, dict) or message.get('role') != 'assistant':
                raise ValueError('Invalid role')
            if message.get('content') is not None and not isinstance(message['content'], str):
                raise ValueError('Invalid content')
            if message.get('reasoning_content') is not None and not isinstance(message['reasoning_content'], str):
                raise ValueError('Invalid reasoning')
            calls = message.get('tool_calls')
            if calls is not None:
                if not isinstance(calls, list):
                    raise ValueError('Invalid tools')
                for call in calls:
                    if (not isinstance(call, dict) or not isinstance(call.get('id'), str) or not call['id']
                            or call.get('type', 'function') != 'function' or not isinstance(call.get('function'), dict)
                            or not isinstance(call['function'].get('name'), str) or not call['function']['name']
                            or not isinstance(call['function'].get('arguments'), str)):
                        raise ValueError('Invalid function')
            if finish_reason == 'tool_calls' and not calls:
                raise ValueError('Missing tools')
            message = {key: item for key, item in message.items()
                       if key in ('role', 'content', 'tool_calls', 'reasoning_content')}
        except (KeyError, IndexError, TypeError, ValueError):
            raise failure('protocol') from None
        return dict(message=message, usage=usage, finish_reason=finish_reason,
                    http_status=status, elapsed_ms=max(0, round((time.monotonic() - started) * 1000)),
                    request_id=request_id, max_tokens=max_tokens)
    except CompletionError:
        raise
    except (TimeoutError, socket.timeout):
        raise failure('timeout', True) from None
    except ssl.SSLCertVerificationError:
        raise failure('certificate') from None
    except urllib.error.URLError as error:
        reason = error.reason
        if isinstance(reason, ssl.SSLCertVerificationError):
            raise failure('certificate') from None
        if isinstance(reason, (TimeoutError, socket.timeout)) or 'timed out' in str(reason).lower():
            raise failure('timeout', True) from None
        raise failure('network', True) from None
    except (ConnectionError, OSError, http.client.HTTPException):
        raise failure('network', True) from None
    except Exception:
        raise failure('transport') from None
