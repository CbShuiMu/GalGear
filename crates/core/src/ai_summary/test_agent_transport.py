"""Offline transport tests; every network operation is replaced with a mock."""
import http.client
import importlib.util
import io
import json
import pathlib
import socket
import ssl
import unittest
import urllib.error
from unittest import mock


spec = importlib.util.spec_from_file_location('agent_transport', pathlib.Path(__file__).with_name('agent_transport.py'))
transport = importlib.util.module_from_spec(spec)
spec.loader.exec_module(transport)


class CompletionTransportTests(unittest.TestCase):
    def setUp(self):
        self.config = dict(provider='deepseek', model='deepseek-chat', token='fixture-secret-token')
        self.messages = [dict(role='user', content='Prepare')]
        self.tools = [dict(type='function', function=dict(name='finish', parameters=dict(type='object')))]
        self.usage = dict(prompt_tokens=120, completion_tokens=45, total_tokens=165,
                          prompt_cache_hit_tokens=100, prompt_cache_miss_tokens=20,
                          completion_tokens_details=dict(reasoning_tokens=31))
        self.message = dict(role='assistant', content=None, reasoning_content='Retained only in message',
                            tool_calls=[dict(id='call_1', type='function',
                                             function=dict(name='finish', arguments='{}'))])

    def response(self, *, body=None, status=200):
        value = dict(id='chatcmpl-fixture', choices=[dict(finish_reason='tool_calls', message=self.message)], usage=self.usage)
        response = mock.Mock()
        response.status = status
        response.headers = {'x-request-id': 'req-fixture'}
        response.read.return_value = json.dumps(value if body is None else body).encode('utf-8')
        opener = mock.Mock()
        # Preserve the established host test convention: __enter__ returns the
        # actual response, rather than using the opener return value directly.
        opener.open.return_value.__enter__ = mock.Mock(return_value=response)
        opener.open.return_value.__exit__ = mock.Mock(return_value=False)
        return opener, response

    def complete(self, opener, **kwargs):
        with mock.patch.object(transport.urllib.request, 'build_opener', return_value=opener):
            return transport.complete(self.config, self.messages, self.tools, **kwargs)

    def test_deepseek_protocol_and_json_diagnostics_preserve_usage(self):
        opener, response = self.response()
        result = self.complete(opener)
        request = opener.open.call_args.args[0]
        payload = json.loads(request.data)
        self.assertEqual(request.full_url, 'https://api.deepseek.com/chat/completions')
        self.assertEqual(payload, dict(model='deepseek-chat', messages=self.messages, tools=self.tools,
                                       max_tokens=12000, thinking=dict(type='enabled'), reasoning_effort='max'))
        self.assertEqual(request.get_header('Authorization'), 'Bearer fixture-secret-token')
        self.assertEqual(request.get_header('Connection'), 'close')
        self.assertEqual(opener.open.call_args.kwargs['timeout'], 300)
        self.assertEqual(result['message'], self.message)
        self.assertEqual(result['usage'], self.usage)
        self.assertEqual(result['finish_reason'], 'tool_calls')
        self.assertEqual(result['http_status'], 200)
        self.assertEqual(result['request_id'], 'chatcmpl-fixture')
        self.assertEqual(result['max_tokens'], 12000)
        self.assertGreaterEqual(result['elapsed_ms'], 0)
        diagnostics = {key: item for key, item in result.items() if key != 'message'}
        encoded = json.dumps(diagnostics, allow_nan=False)
        self.assertNotIn(self.config['token'], encoded)
        self.assertNotIn('Retained only in message', encoded)
        self.assertNotIn('headers', diagnostics)
        response.read.assert_called_once_with(transport.MAX_RESPONSE_BYTES + 1)
        response.close.assert_called_once()

    def test_preparation_ignores_legacy_reasoning_overrides(self):
        for effort in ('low', 'high'):
            with self.subTest(effort=effort):
                self.config['reasoning_effort'] = effort
                opener, _ = self.response()
                self.complete(opener)
                payload = json.loads(opener.open.call_args.args[0].data)
                self.assertEqual(payload['reasoning_effort'], 'max')
                self.assertEqual(payload['thinking'], dict(type='enabled'))

    def test_openai_protocol_uses_completion_budget_and_auto_tools(self):
        self.config['provider'] = 'openai'
        opener, _ = self.response()
        self.complete(opener, max_tokens=24000, timeout=123)
        request = opener.open.call_args.args[0]
        payload = json.loads(request.data)
        self.assertEqual(request.full_url, 'https://api.openai.com/v1/chat/completions')
        self.assertEqual(payload['max_completion_tokens'], 24000)
        self.assertEqual(payload['tool_choice'], 'auto')
        self.assertNotIn('max_tokens', payload)
        self.assertNotIn('thinking', payload)

    def test_plain_stop_response_can_omit_usage_and_use_header_request_id(self):
        message = dict(role='assistant', content='Ready')
        opener, response = self.response(body=dict(choices=[dict(finish_reason='stop', message=message)]))
        result = self.complete(opener)
        self.assertEqual(result['message'], message)
        self.assertEqual(result['usage'], {})
        self.assertEqual(result['finish_reason'], 'stop')
        self.assertEqual(result['request_id'], 'req-fixture')
        response.close.assert_called_once()

    def test_tools_and_input_context_are_transmitted_unchanged(self):
        self.messages.append(self.message)
        self.messages.append(dict(role='tool', tool_call_id='call_1', content='{}'))
        opener, _ = self.response()
        self.complete(opener)
        payload = json.loads(opener.open.call_args.args[0].data)
        self.assertEqual(payload['tools'], self.tools)
        self.assertEqual(payload['messages'], self.messages)
        self.assertEqual(payload['messages'][1]['reasoning_content'], self.message['reasoning_content'])

    def test_length_never_returns_partial_tool_arguments_and_does_not_retry(self):
        value = dict(id='truncated', usage=self.usage, choices=[dict(finish_reason='length', message=dict(
            role='assistant', reasoning_content='Unlogged reasoning', tool_calls=[dict(
                id='call_1', function=dict(name='write_program', arguments='{"source":"partial'))]))])
        opener, response = self.response(body=value)
        with self.assertRaises(transport.CompletionError) as caught:
            self.complete(opener)
        error = caught.exception
        self.assertEqual(error.kind, 'length')
        self.assertTrue(error.retryable)
        self.assertEqual(error.finish_reason, 'length')
        self.assertEqual(error.usage, self.usage)
        self.assertEqual(error.request_id, 'truncated')
        self.assertEqual(error.http_status, 200)
        self.assertEqual(error.max_tokens, 12000)
        self.assertNotIn('partial', json.dumps(error.diagnostic))
        self.assertNotIn('Unlogged reasoning', json.dumps(error.diagnostic))
        opener.open.assert_called_once()
        response.close.assert_called_once()

    def test_http_status_classification_preserves_selected_metadata_only(self):
        for status in (301, 302, 400, 401, 402, 403, 404, 408, 422, 429, 500, 502, 503, 504):
            with self.subTest(status=status):
                body = dict(id='req-failed', usage=self.usage, choices=[dict(finish_reason='length')],
                            error=dict(message='Private provider body ' + self.config['token']))
                failure = urllib.error.HTTPError('https://api.deepseek.com/chat/completions', status,
                                                'Private HTTP reason ' + self.config['token'],
                                                {'x-request-id': 'req-header'},
                                                io.BytesIO(json.dumps(body).encode()))
                opener = mock.Mock()
                opener.open.side_effect = failure
                with mock.patch.object(failure, 'close', wraps=failure.close) as close:
                    with self.assertRaises(transport.CompletionError) as caught:
                        self.complete(opener)
                    close.assert_called_once()
                error = caught.exception
                self.assertEqual(error.kind, 'http')
                self.assertEqual(error.http_status, status)
                self.assertEqual(error.retryable, status in transport.RETRYABLE_HTTP_STATUSES)
                self.assertEqual(error.usage, self.usage)
                self.assertEqual(error.finish_reason, 'length')
                self.assertEqual(error.request_id, 'req-failed')
                encoded = json.dumps(error.diagnostic) + str(error)
                self.assertNotIn('Private', encoded)
                self.assertNotIn(self.config['token'], encoded)
                opener.open.assert_called_once()

    def test_http_error_with_non_json_body_is_still_closed_and_safe(self):
        failure = urllib.error.HTTPError('https://api.deepseek.com/chat/completions', 503,
                                        'Private reason', {}, io.BytesIO(b'private non-JSON body'))
        opener = mock.Mock()
        opener.open.side_effect = failure
        with mock.patch.object(failure, 'close', wraps=failure.close) as close:
            with self.assertRaises(transport.CompletionError) as caught:
                self.complete(opener)
            close.assert_called_once()
        self.assertEqual(caught.exception.kind, 'http')
        self.assertEqual(caught.exception.usage, {})
        self.assertTrue(caught.exception.retryable)

    def test_non_raising_http_adapter_preserves_http_failure_classification(self):
        for raw in (b'Private gateway error', json.dumps(dict(usage=self.usage)).encode()):
            with self.subTest(raw=raw):
                opener, response = self.response(status=503)
                response.read.return_value = raw
                with self.assertRaises(transport.CompletionError) as caught:
                    self.complete(opener)
                self.assertEqual(caught.exception.kind, 'http')
                self.assertEqual(caught.exception.http_status, 503)
                self.assertTrue(caught.exception.retryable)
                self.assertNotIn('Private', str(caught.exception) + json.dumps(caught.exception.diagnostic))
                response.close.assert_called_once()

    def test_network_and_timeout_failures_are_classified_without_private_messages(self):
        cases = [
            (socket.timeout('Private timeout'), 'timeout', True),
            (urllib.error.URLError(socket.timeout('Private timeout')), 'timeout', True),
            (urllib.error.URLError('Private timed out connection'), 'timeout', True),
            (urllib.error.URLError('Private DNS detail'), 'network', True),
            (ConnectionResetError('Private reset'), 'network', True),
            (http.client.RemoteDisconnected('Private remote disconnect'), 'network', True),
            (http.client.IncompleteRead(b'Private partial bytes', 200), 'network', True),
            (ssl.SSLCertVerificationError('Private certificate detail'), 'certificate', False),
            (urllib.error.URLError(ssl.SSLCertVerificationError('Private certificate detail')), 'certificate', False),
        ]
        for failure, kind, retryable in cases:
            with self.subTest(kind=kind, failure=type(failure).__name__):
                opener = mock.Mock()
                opener.open.side_effect = failure
                with self.assertRaises(transport.CompletionError) as caught:
                    self.complete(opener)
                self.assertEqual(caught.exception.kind, kind)
                self.assertEqual(caught.exception.retryable, retryable)
                self.assertNotIn('Private', str(caught.exception) + json.dumps(caught.exception.diagnostic))
                opener.open.assert_called_once()

    def test_read_timeout_closes_response_and_preserves_header_request_id(self):
        opener, response = self.response()
        response.read.side_effect = socket.timeout('Private')
        with self.assertRaises(transport.CompletionError) as caught:
            self.complete(opener)
        self.assertEqual(caught.exception.kind, 'timeout')
        self.assertEqual(caught.exception.http_status, 200)
        self.assertEqual(caught.exception.request_id, 'req-fixture')
        response.close.assert_called_once()

    def test_redirects_are_explicitly_rejected(self):
        self.assertIsNone(transport.NoRedirect().redirect_request(None, None, 302, 'redirect', {}, 'https://other.test/'))
        opener, _ = self.response()
        with mock.patch.object(transport.urllib.request, 'build_opener', return_value=opener) as build:
            transport.complete(self.config, self.messages, self.tools)
        self.assertIsInstance(build.call_args.args[0], transport.NoRedirect)

    def test_oversized_body_is_rejected_and_closed(self):
        opener, response = self.response()
        response.read.return_value = b'x' * (transport.MAX_RESPONSE_BYTES + 1)
        with self.assertRaises(transport.CompletionError) as caught:
            self.complete(opener)
        self.assertEqual(caught.exception.kind, 'response_too_large')
        self.assertFalse(caught.exception.retryable)
        response.close.assert_called_once()

    def test_invalid_json_and_message_shapes_are_safe_protocol_errors(self):
        values = [None, [], {}, dict(choices=[]), dict(choices=[dict(finish_reason='stop')]),
                  dict(choices=[dict(finish_reason='stop', message=dict(role='user', content='Private'))]),
                  dict(choices=[dict(finish_reason='tool_calls', message=dict(role='assistant', tool_calls=[]))]),
                  dict(choices=[dict(finish_reason='tool_calls', message=dict(role='assistant', tool_calls=[dict(id='x')]))])]
        for value in values:
            opener, response = self.response()
            response.read.return_value = json.dumps(value).encode()
            with self.subTest(value=value), self.assertRaises(transport.CompletionError) as caught:
                self.complete(opener)
            self.assertEqual(caught.exception.kind, 'protocol')
            self.assertFalse(caught.exception.retryable)
            response.close.assert_called_once()
        for raw in (b'{"secret":"private', b'{"usage":{"tokens":NaN}}', b'\xff'):
            opener, response = self.response()
            response.read.return_value = raw
            with self.subTest(raw=raw), self.assertRaises(transport.CompletionError) as caught:
                self.complete(opener)
            self.assertEqual(caught.exception.kind, 'protocol')
            response.close.assert_called_once()

    def test_other_finish_reasons_are_not_retried_and_keep_usage(self):
        opener, _ = self.response(body=dict(usage=self.usage, choices=[dict(finish_reason='content_filter')]))
        with self.assertRaises(transport.CompletionError) as caught:
            self.complete(opener)
        self.assertEqual(caught.exception.kind, 'finish_reason')
        self.assertEqual(caught.exception.finish_reason, 'content_filter')
        self.assertEqual(caught.exception.usage, self.usage)
        self.assertFalse(caught.exception.retryable)

    def test_json_safe_usage_extensions_survive_and_secret_cannot_enter_diagnostics(self):
        self.usage['provider_extension'] = dict(reasoning=[1, None, True, 'count'], label=self.config['token'])
        opener, _ = self.response()
        result = self.complete(opener)
        self.assertEqual(result['usage']['provider_extension']['reasoning'], [1, None, True, 'count'])
        self.assertEqual(result['usage']['provider_extension']['label'], '[redacted]')
        json.dumps(result['usage'], allow_nan=False)

    def test_invalid_configuration_does_not_start_a_network_request(self):
        for change, kwargs in [(dict(provider='other'), {}), (dict(token=''), {}),
                               (dict(model=''), {}), ({}, dict(max_tokens=0)), ({}, dict(timeout=-1)),
                               ({}, dict(timeout=float('nan'))), ({}, dict(max_tokens=object()))]:
            with self.subTest(change=change, kwargs=kwargs):
                config = dict(self.config, **change)
                with mock.patch.object(transport.urllib.request, 'build_opener') as build:
                    with self.assertRaises(transport.CompletionError) as caught:
                        transport.complete(config, self.messages, self.tools, **kwargs)
                    self.assertEqual(caught.exception.kind, 'configuration')
                    json.dumps(caught.exception.diagnostic, allow_nan=False)
                    build.assert_not_called()

    def test_interruption_closes_an_active_response(self):
        opener, response = self.response()
        response.read.side_effect = KeyboardInterrupt()
        with self.assertRaises(KeyboardInterrupt):
            self.complete(opener)
        response.close.assert_called_once()


if __name__ == '__main__':
    unittest.main()
