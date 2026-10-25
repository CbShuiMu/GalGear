"""Read bounded model metadata from an official provider endpoint.

Only normalized capability fields leave this module. Provider bodies, headers,
free-form error messages and credentials are never returned or persisted.
https://api-docs.deepseek.com/api/list-models/
"""
import http.client
import json
import math
import re
import socket
import ssl
import time
import urllib.error
import urllib.request


DEEPSEEK_MODELS = 'https://api.deepseek.com/models'
MAX_RESPONSE_BYTES = 1024 * 1024
MAX_MODELS = 1024
MAX_TOKEN_CAPACITY = 2**31-1
_IDENTIFIER = re.compile(r'[A-Za-z0-9][A-Za-z0-9_.:/@+\-]{0,159}\Z', re.ASCII)
_ENUM = re.compile(r'[a-z][a-z0-9_\-]{0,63}\Z', re.ASCII)


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *args, **kwargs):
        return None


def _safe_identifier(value, secret='', *, enumeration=False):
    pattern = _ENUM if enumeration else _IDENTIFIER
    return (isinstance(value, str) and pattern.fullmatch(value) is not None
            and not (secret and secret in value))


def _capacity(value):
    return value if type(value) is int and 1 <= value <= MAX_TOKEN_CAPACITY else None


def _levels(value, secret):
    if (not isinstance(value, list) or not value or len(value) > 32
            or any(not _safe_identifier(item, secret, enumeration=True) for item in value)
            or len(set(value)) != len(value)):
        return None
    return list(value)


def normalize_models(value, model, *, secret=''):
    """Select one exact model ID without alias guesses or baked-in limits.

    Missing optional fields remain unknown. Invalid numeric limits are not
    converted, clamped or treated as zero. Duplicate IDs fail closed.
    """
    base = dict(provider='deepseek', model=model if _safe_identifier(model, secret) else None,
                source=DEEPSEEK_MODELS, status='unavailable', kind='protocol')
    if (not _safe_identifier(model, secret) or not isinstance(value, dict)
            or value.get('object') != 'list' or not isinstance(value.get('data'), list)
            or len(value['data']) > MAX_MODELS or any(not isinstance(item, dict) for item in value['data'])):
        return base
    matches = [item for item in value['data'] if item.get('id') == model and item.get('object') == 'model']
    if not matches:
        return dict(base, kind='model_not_listed')
    if len(matches) != 1:
        return dict(base, kind='ambiguous_model')
    selected = matches[0]
    context, maximum = (_capacity(selected.get(key)) for key in ('context_window', 'max_output_tokens'))
    # Contradictory or malformed advertised numeric fields cannot become
    # authority for a larger request. An omitted field is merely unknown.
    if (any(key in selected and _capacity(selected[key]) is None
            for key in ('context_window', 'max_output_tokens'))
            or context is not None and maximum is not None and maximum > context):
        return dict(base, kind='invalid_limits')
    result = dict(base)
    if context is not None:
        result['context_window'] = context
    if maximum is not None:
        result['max_output_tokens'] = maximum
    for key, allowed in (('input_modalities', {'text', 'image', 'audio', 'video'}),
                         ('output_modalities', {'text', 'image', 'audio', 'video'})):
        values = selected.get(key)
        if (isinstance(values, list) and len(values) <= 8 and len(set(item for item in values if isinstance(item, str))) == len(values)
                and all(isinstance(item, str) and item in allowed and not (secret and secret in item) for item in values)):
            result[key] = list(values)
    effort = selected.get('effort')
    if isinstance(effort, dict):
        levels = _levels(effort.get('supported_levels'), secret)
        if levels is not None:
            result['effort'] = dict(supported_levels=levels)
            default = effort.get('default_level')
            if default in levels:
                result['effort']['default_level'] = default
    protocols = selected.get('api_capabilities')
    if isinstance(protocols, dict):
        # This is the currently documented per-protocol field. Unknown future
        # descriptions/free-form data are deliberately not copied to logs.
        messages = protocols.get('anthropic_messages')
        if (isinstance(messages, dict) and messages.get('system_prompt_update') in ('leading-only', 'in-history')
                and not (secret and secret in messages['system_prompt_update'])):
            result['api_capabilities'] = dict(anthropic_messages=dict(system_prompt_update=messages['system_prompt_update']))
    if context is not None or maximum is not None:
        result.update(status='available', kind='model_metadata')
    else:
        result.update(kind='limits_unavailable')
    return result


def read_capabilities(config, timeout=20):
    """Perform at most one official GET and return safe nonfatal diagnostics.

    There is no endpoint override, redirect, automatic retry, model-name alias
    inference or raw-response logging. This does not issue a model completion.
    """
    started = time.monotonic()
    provider = config.get('provider') if isinstance(config, dict) else None
    model = config.get('model') if isinstance(config, dict) else None
    secret = config.get('token') if isinstance(config, dict) else None
    secret = secret if isinstance(secret, str) else ''
    safe_model = model if _safe_identifier(model, secret) else None
    status = None
    base = dict(provider=provider if provider in ('deepseek', 'openai') else None,
                model=safe_model, status='unavailable', kind='configuration')
    if provider == 'deepseek':
        base['source'] = DEEPSEEK_MODELS
    def result(kind, *, retryable=False, payload=None):
        value = dict(base if payload is None else payload)
        value.update(kind=kind, http_status=status, retryable=bool(retryable),
                     elapsed_ms=max(0, round((time.monotonic()-started)*1000)), retrieved_at=int(time.time()))
        return value
    if provider != 'deepseek':
        return result('unsupported_provider')
    if (safe_model is None or not secret or '\r' in secret or '\n' in secret
            or isinstance(timeout, bool) or not isinstance(timeout, (int, float))
            or not math.isfinite(timeout) or not 0 < timeout <= 60):
        return result('configuration')
    try:
        request = urllib.request.Request(DEEPSEEK_MODELS, method='GET', headers={
            'Authorization': 'Bearer '+secret, 'Accept': 'application/json', 'Connection': 'close'})
        try:
            with urllib.request.build_opener(NoRedirect()).open(request, timeout=timeout) as response:
                try:
                    candidate_status = getattr(response, 'status', None)
                    if type(candidate_status) is not int:
                        candidate_status = response.getcode()
                    status = candidate_status if type(candidate_status) is int else 200
                    raw = response.read(MAX_RESPONSE_BYTES+1)
                finally:
                    response.close()
        except urllib.error.HTTPError as error:
            status = error.code
            # Do not even decode a free-form provider error body.
            error.close()
            return result('http', retryable=status in (429, 500, 502, 503, 504))
        if not 200 <= status < 300:
            return result('http', retryable=status in (429, 500, 502, 503, 504))
        if not isinstance(raw, bytes):
            return result('protocol')
        if len(raw) > MAX_RESPONSE_BYTES:
            return result('response_too_large')
        def invalid_constant(_value):
            raise ValueError('Non-JSON number')
        try:
            value = json.loads(raw, parse_constant=invalid_constant)
        except (ValueError, UnicodeError):
            return result('protocol')
        normalized = normalize_models(value, model, secret=secret)
        return result(normalized['kind'], payload=normalized)
    except (TimeoutError, socket.timeout):
        return result('timeout', retryable=True)
    except ssl.SSLCertVerificationError:
        return result('certificate')
    except urllib.error.URLError as error:
        reason = error.reason
        if isinstance(reason, ssl.SSLCertVerificationError):
            return result('certificate')
        if isinstance(reason, (TimeoutError, socket.timeout)):
            return result('timeout', retryable=True)
        return result('network', retryable=True)
    except (ConnectionError, OSError, http.client.HTTPException):
        return result('network', retryable=True)
    except Exception:
        return result('transport')
