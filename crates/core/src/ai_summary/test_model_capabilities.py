"""Offline capability discovery: no actual credentials or model requests."""
import copy
import io
import json
import pathlib
import runpy
import socket
import ssl
import unittest
import urllib.error
from unittest import mock

import model_capabilities as capabilities


class ModelCapabilitiesTests(unittest.TestCase):
    def setUp(self):
        self.config = dict(provider='deepseek', model='future-model-x', token='private-capability-token')
        self.entry = dict(id='future-model-x', object='model', owned_by='provider', name='Ignored display text',
                          context_window=2**20, max_output_tokens=3*2**17,
                          input_modalities=['text', 'image'], output_modalities=['text'],
                          effort=dict(supported_levels=['low', 'high', 'max'], default_level='high'),
                          api_capabilities=dict(anthropic_messages=dict(system_prompt_update='in-history')))

    def body(self, entry=None):
        return dict(object='list', data=[self.entry if entry is None else entry])

    def opener(self, body=None, status=200):
        response = mock.Mock()
        response.status = status
        response.read.return_value = json.dumps(self.body() if body is None else body).encode()
        opener = mock.Mock()
        opener.open.return_value.__enter__ = mock.Mock(return_value=response)
        opener.open.return_value.__exit__ = mock.Mock(return_value=False)
        return opener, response

    def read(self, opener, config=None, **kwargs):
        with mock.patch.object(capabilities.urllib.request, 'build_opener', return_value=opener):
            return capabilities.read_capabilities(self.config if config is None else config, **kwargs)

    def test_exact_official_get_normalizes_limits_effort_and_protocol_metadata(self):
        opener, response = self.opener()
        value = self.read(opener, timeout=17)
        request = opener.open.call_args.args[0]
        self.assertEqual(request.full_url, 'https://api.deepseek.com/models')
        self.assertEqual(request.get_method(), 'GET')
        self.assertIsNone(request.data)
        self.assertEqual(request.get_header('Authorization'), 'Bearer '+self.config['token'])
        self.assertEqual(opener.open.call_args.kwargs['timeout'], 17)
        self.assertEqual(value['status'], 'available')
        self.assertEqual(value['model'], self.config['model'])
        self.assertEqual(value['context_window'], 2**20)
        self.assertEqual(value['max_output_tokens'], 3*2**17)
        self.assertEqual(value['effort']['supported_levels'], ['low', 'high', 'max'])
        self.assertEqual(value['api_capabilities'], self.entry['api_capabilities'])
        self.assertNotIn('name', value)
        self.assertNotIn('owned_by', value)
        self.assertNotIn(self.config['token'], json.dumps(value))
        response.read.assert_called_once_with(capabilities.MAX_RESPONSE_BYTES+1)
        response.close.assert_called_once()

    def test_no_fixed_model_names_or_numeric_limit_aliases(self):
        entry = dict(self.entry, id='new-model-revision', context_window=777777, max_output_tokens=135791)
        value = capabilities.normalize_models(self.body(entry), 'new-model-revision')
        self.assertEqual(value['max_output_tokens'], 135791)
        self.assertEqual(value['context_window'], 777777)
        absent = capabilities.normalize_models(self.body(entry), 'legacy-model-alias')
        self.assertEqual(absent['kind'], 'model_not_listed')
        self.assertNotIn('max_output_tokens', absent)

    def test_missing_optional_limits_remain_unknown_instead_of_zero(self):
        old = dict(id=self.config['model'], object='model', owned_by='provider')
        value = capabilities.normalize_models(self.body(old), self.config['model'])
        self.assertEqual(value['status'], 'unavailable')
        self.assertEqual(value['kind'], 'limits_unavailable')
        self.assertNotIn('max_output_tokens', value)
        self.assertNotIn('context_window', value)
        partial = dict(old, max_output_tokens=8191)
        value = capabilities.normalize_models(self.body(partial), self.config['model'])
        self.assertEqual(value['status'], 'available')
        self.assertEqual(value['max_output_tokens'], 8191)
        self.assertNotIn('context_window', value)

    def test_malformed_or_contradictory_limits_do_not_authorize_large_requests(self):
        for value in (0, -1, True, '393216', 13.5, 2**40, None):
            with self.subTest(value=value):
                result = capabilities.normalize_models(self.body(dict(self.entry, max_output_tokens=value)), self.config['model'])
                self.assertEqual(result['kind'], 'invalid_limits')
                self.assertNotIn('max_output_tokens', result)
        result = capabilities.normalize_models(self.body(dict(self.entry, context_window=4096, max_output_tokens=8192)), self.config['model'])
        self.assertEqual(result['kind'], 'invalid_limits')

    def test_duplicate_ids_and_invalid_document_shapes_fail_closed(self):
        body = self.body()
        body['data'].append(copy.deepcopy(self.entry))
        self.assertEqual(capabilities.normalize_models(body, self.config['model'])['kind'], 'ambiguous_model')
        for value in (None, [], {}, dict(object='list', data='wrong'), dict(object='list', data=[None])):
            with self.subTest(value=value):
                self.assertEqual(capabilities.normalize_models(value, self.config['model'])['kind'], 'protocol')

    def test_unknown_provider_does_not_contact_an_arbitrary_endpoint(self):
        for change in (dict(provider='openai'), dict(provider='other'), dict(token=''), dict(model=''),
                       dict(token='bad\nheader'), dict(model=self.config['token'])):
            with self.subTest(fields=list(change)), mock.patch.object(capabilities.urllib.request, 'build_opener') as build:
                value = capabilities.read_capabilities(dict(self.config, **change))
                self.assertEqual(value['status'], 'unavailable')
                self.assertNotIn(self.config['token'], json.dumps(value))
                build.assert_not_called()
        opener, _ = self.opener()
        self.read(opener, dict(self.config, base_url='https://untrusted.example/models'))
        self.assertEqual(opener.open.call_args.args[0].full_url, capabilities.DEEPSEEK_MODELS)

    def test_http_error_body_and_headers_are_never_read_or_logged(self):
        for status in (301, 401, 403, 429, 503):
            failure = urllib.error.HTTPError('https://api.deepseek.com/models', status,
                'Private '+self.config['token'], {'Private': self.config['token']},
                io.BytesIO(('Private '+self.config['token']).encode()))
            opener = mock.Mock()
            opener.open.side_effect = failure
            with mock.patch.object(failure, 'read', wraps=failure.read) as read:
                value = self.read(opener)
                read.assert_not_called()
            self.assertEqual(value['http_status'], status)
            self.assertEqual(value['kind'], 'http')
            self.assertNotIn('Private', json.dumps(value))
            self.assertNotIn(self.config['token'], json.dumps(value))

    def test_response_size_json_and_network_errors_are_safe_nonfatal_results(self):
        for raw, kind in ((b'x'*(capabilities.MAX_RESPONSE_BYTES+1), 'response_too_large'),
                          (b'{', 'protocol'), (b'{"data":NaN}', 'protocol'), (b'\xff', 'protocol')):
            opener, response = self.opener()
            response.read.return_value = raw
            value = self.read(opener)
            self.assertEqual(value['kind'], kind)
            response.close.assert_called_once()
        for failure, kind in ((socket.timeout('Private'), 'timeout'),
                              (urllib.error.URLError('Private'), 'network'),
                              (ssl.SSLCertVerificationError('Private'), 'certificate')):
            opener = mock.Mock()
            opener.open.side_effect = failure
            value = self.read(opener)
            self.assertEqual(value['kind'], kind)
            self.assertNotIn('Private', json.dumps(value))

    def test_free_form_and_secret_bearing_extensions_are_not_retained(self):
        entry = copy.deepcopy(self.entry)
        entry.update(error=self.config['token'], name=self.config['token'])
        entry['effort']['supported_levels'].append(self.config['token'])
        entry['api_capabilities']['custom'] = dict(message=self.config['token'])
        value = capabilities.normalize_models(self.body(entry), self.config['model'], secret=self.config['token'])
        self.assertNotIn('effort', value)
        self.assertNotIn('custom', value.get('api_capabilities', {}))
        self.assertNotIn(self.config['token'], json.dumps(value))

    def test_invalid_timeouts_and_redirects_never_forward_credentials(self):
        self.assertIsNone(capabilities.NoRedirect().redirect_request(None, None, 302, 'Private', {}, 'https://other/'))
        for timeout in (0, -1, True, float('nan'), 61, None):
            with self.subTest(timeout=timeout), mock.patch.object(capabilities.urllib.request, 'build_opener') as build:
                value = capabilities.read_capabilities(self.config, timeout=timeout)
                self.assertEqual(value['kind'], 'configuration')
                build.assert_not_called()

    def test_transport_response_read_limit_grows_boundedly_with_output_budget(self):
        transport = runpy.run_path(str(pathlib.Path(__file__).with_name('agent_transport.py')))
        limits = transport['response_byte_limit']
        self.assertEqual(limits(12000), 8*1024*1024)
        self.assertGreater(limits(393216), limits(32000))
        self.assertEqual(limits(2**31), 32*1024*1024)
        self.assertEqual(limits(True), 8*1024*1024)


if __name__ == '__main__':
    unittest.main()
