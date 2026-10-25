"""Actual advertised limits drive bounded MAX-thinking work requests."""
import json
import unittest
from unittest import mock

import test_transport_integration as fixtures

host = fixtures.host


class OutputMetadataBudgetTests(unittest.TestCase):
    setUp = fixtures.TransportIntegrationTests.setUp
    confirm_save = fixtures.TransportIntegrationTests.confirm_save
    response = fixtures.TransportIntegrationTests.response
    network = fixtures.TransportIntegrationTests.network
    requests = fixtures.TransportIntegrationTests.requests

    def capabilities(self, maximum=393216):
        value = dict(provider='deepseek', model='test', source='https://api.deepseek.com/models',
                     status='available', max_output_tokens=maximum, context_window=1048576,
                     effort=dict(supported_levels=['low', 'high', 'max']))
        self.config['model_capabilities'] = value
        self.agent.model_capabilities = value
        self.agent.capabilities_checked = True
        return value

    def test_actual_provider_metadata_expands_research_and_authoring_baselines(self):
        self.capabilities()
        opener, patcher = self.network([self.response(), self.response()])
        with patcher:
            self.agent.workflow.stage = 'resources'
            self.agent.run_turn(self.messages, host.TOOLS, 0, 'prepare')
            self.agent.workflow.stage = 'script_program'
            self.agent.run_turn(self.messages, host.TOOLS, 1, 'prepare')
        payloads = [json.loads(call.args[0].data) for call in opener.open.call_args_list]
        self.assertEqual([item['max_tokens'] for item in payloads], [65536, 131072])
        self.assertTrue(all(item['reasoning_effort'] == 'max' for item in payloads))
        self.assertEqual([call.kwargs['timeout'] for call in opener.open.call_args_list], [600, 900])

    def test_two_length_expansions_use_remaining_advertised_capacity_without_dispatching_partial_code(self):
        self.capabilities()
        self.agent.workflow.stage = 'script_program'
        original = dict(self.agent.programs)
        responses = [self.response('write_program', '{"source":"partial', reason='length'),
                     self.response('write_program', '{"source":"still partial', reason='length'), self.response()]
        opener, patcher = self.network(responses)
        with patcher:
            self.agent.run_turn(self.messages, host.TOOLS, 0, 'prepare')
        payloads = [json.loads(call.args[0].data) for call in opener.open.call_args_list]
        self.assertEqual([item['max_tokens'] for item in payloads], [131072, 262144, 393216])
        self.assertTrue(all(item['reasoning_effort'] == 'max' for item in payloads))
        self.assertTrue(all(call.kwargs['timeout'] == 900 for call in opener.open.call_args_list))
        self.assertEqual(self.agent.programs, original)
        self.assertEqual([event['tool'] for event in self.agent.events], ['list_files'])
        self.assertEqual(self.agent.requests_sent, 3)

    def test_lower_actual_limit_is_respected_and_never_retried_at_the_same_ceiling(self):
        self.capabilities(16384)
        self.agent.workflow.stage = 'save_program'
        opener, patcher = self.network([self.response(reason='length')])
        with patcher, self.assertRaises(host.TRANSPORT['CompletionError']):
            self.agent.run_turn(self.messages, host.TOOLS, 0, 'prepare')
        opener.open.assert_called_once()
        self.assertEqual(json.loads(opener.open.call_args.args[0].data)['max_tokens'], 16384)

    def test_metadata_is_queried_once_only_when_a_real_work_turn_is_needed(self):
        metadata = self.capabilities()
        self.agent.capabilities_checked = False
        self.agent.model_capabilities = {}
        fetch = mock.Mock(return_value=dict(metadata, token=self.token))
        opener, patcher = self.network([self.response(), self.response()])
        with mock.patch.dict(host.TRANSPORT, read_capabilities=fetch), patcher:
            fetch.assert_not_called()
            self.agent.run_turn(self.messages, host.TOOLS, 0, 'prepare')
            self.agent.run_turn(self.messages, host.TOOLS, 1, 'prepare')
        fetch.assert_called_once_with(self.agent.config)
        persisted = (self.agent.work / 'model-capabilities.json').read_text('utf-8')
        self.assertNotIn(self.token, persisted)
        self.assertNotIn('token', json.loads(persisted))
        self.assertEqual(opener.open.call_count, 2)

    def test_zero_request_probe_does_not_query_metadata_or_inference(self):
        self.agent.config['max_turns'] = 0
        self.agent.capabilities_checked = False
        self.agent.model_capabilities = {}
        with mock.patch.dict(host.TRANSPORT, read_capabilities=mock.Mock(side_effect=AssertionError('Metadata query forbidden'))):
            with mock.patch.object(self.agent, 'request_completion', side_effect=AssertionError('Inference forbidden')):
                with self.assertRaisesRegex(RuntimeError, 'budget exhausted'):
                    self.agent.run()
        self.assertEqual(self.agent.requests_sent, 0)

    def test_advertised_absence_of_max_never_silently_downgrades_thinking(self):
        value = self.capabilities()
        value['effort'] = dict(supported_levels=['low', 'high'])
        with mock.patch.object(self.agent, 'request_completion') as completion:
            with self.assertRaisesRegex(RuntimeError, 'reasoning_effort=max'):
                self.agent.run_turn(self.messages, host.TOOLS, 0, 'prepare')
        completion.assert_not_called()
        self.assertEqual(self.agent.requests_sent, 0)

    def test_wide_output_budget_survives_a_checkpoint_without_changing_reasoning_effort(self):
        self.capabilities()
        self.agent.work_guidance()
        self.agent.output_limit_stage = self.agent.workflow.stage
        self.agent.output_limit = 262144
        self.agent.checkpoint(self.messages)
        saved = json.loads((self.agent.work / 'checkpoint.json').read_text('utf-8'))
        restored = host.Agent(self.config)
        restored.programs = dict(self.agent.programs)
        self.assertTrue(restored.restore_checkpoint(saved))
        self.assertEqual(restored.output_limit, 262144)
        self.assertEqual(restored.reasoning_effort, 'max')

    def test_missing_limit_and_boolean_limit_remain_unknown_rather_than_provider_zero(self):
        for metadata in ({'status':'available'}, {'status':'available','max_output_tokens':True},
                         {'status':'unavailable','max_output_tokens':393216}):
            with self.subTest(metadata=metadata):
                policy = host.output_token_policy(metadata, 'script_program')
                self.assertEqual(policy['source'], 'bounded_fallback')
                self.assertIsNone(policy['advertised_max_output_tokens'])

    def test_request_timeout_respects_the_parent_runtime_remaining_budget(self):
        self.capabilities()
        self.agent.workflow.stage = 'save_program'
        self.agent.runtime_deadline = 1100.75
        opener, patcher = self.network([self.response()])
        with patcher, mock.patch.object(host.time, 'monotonic', return_value=1000):
            self.agent.run_turn(self.messages, host.TOOLS, 0, 'prepare')
        self.assertEqual(opener.open.call_args.kwargs['timeout'], 100)
        payload = json.loads(opener.open.call_args.args[0].data)
        self.assertEqual(payload['max_tokens'], 131072)
        self.assertEqual(payload['reasoning_effort'], 'max')

    def test_expired_runtime_never_starts_an_inference_request(self):
        self.agent.runtime_deadline = 100
        with mock.patch.object(host.time, 'monotonic', return_value=101):
            with mock.patch.dict(host.TRANSPORT, complete=mock.Mock(side_effect=AssertionError('No request after deadline'))):
                with self.assertRaisesRegex(RuntimeError, 'runtime budget exhausted'):
                    self.agent.request_completion(self.messages, host.TOOLS)

    def test_expired_parent_budget_stops_before_capability_lookup_and_attempt_accounting(self):
        self.agent.runtime_deadline = 100
        self.agent.capabilities_checked = False
        with mock.patch.object(host.time, 'monotonic', return_value=101):
            with mock.patch.dict(host.TRANSPORT, read_capabilities=mock.Mock(side_effect=AssertionError('No metadata after deadline'))):
                with self.assertRaisesRegex(RuntimeError, 'runtime budget exhausted'):
                    self.agent.run_turn(self.messages, host.TOOLS, 0, 'prepare')
        self.assertEqual(self.agent.requests_sent, 0)


if __name__ == '__main__':
    unittest.main()
