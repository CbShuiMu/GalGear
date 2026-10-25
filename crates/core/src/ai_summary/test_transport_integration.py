"""Offline integration of request accounting, transport errors and host phases."""
import importlib.util
import io
import json
import pathlib
import socket
import tempfile
import unittest
import urllib.error
from unittest import mock


spec = importlib.util.spec_from_file_location('transport_integration_host', pathlib.Path(__file__).with_name('agent_host.py'))
host = importlib.util.module_from_spec(spec)
spec.loader.exec_module(host)


class TransportIntegrationTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = pathlib.Path(temporary.name)
        (self.root / 'game.exe').write_bytes(b'fixture')
        self.scene = self.root / 'scene.txt'
        self.scene.write_bytes(b'Current')
        self.saves = self.root / 'saves'
        self.saves.mkdir()
        for name in ('one.sav', 'two.sav'):
            (self.saves / name).write_bytes(b'Current|scene|' + (0).to_bytes(4, 'little'))
        self.token = 'fixture-integration-secret-token'
        self.config = dict(game=str(self.root), save=str(self.saves), skill=str(self.root),
                           work=str(self.root / 'work'), title='Fixture', language='en-US',
                           provider='deepseek', model='test', token=self.token, max_turns=6,
                           model_capabilities=dict(status='unavailable', kind='offline_fixture'),
                           required_saves=[str(self.saves / 'two.sav')])
        self.agent = host.Agent(self.config)
        self.agent.read_skill = self.agent.read_contract = self.agent.used_skill = True
        self.agent.parts[str(self.scene)] = [dict(source='scene.txt', position=0, text='Current', speaker=None)]
        self.agent.speaker_review = dict(version=1, signature=self.agent.speaker_signature())
        self.agent.sources.add(str(self.scene))
        self.agent.programs = dict(
            save='def extract(data, metadata): return {"source":"scene.txt","line":0,"anchors":["Current"],"evidence":{"anchor":{"offset":0,"length":7,"encoding":"utf-8"},"source":{"offset":8,"length":5,"encoding":"ascii"},"position":{"offset":14,"width":4}}}',
            script='def extract(data, metadata): return [{"position":0,"text":"Current","speaker":None,"evidence":{"offset":0,"length":7,"encoding":"utf-8"}}]')
        for kind, source in self.agent.programs.items():
            (self.agent.work / (kind + '_extractor.py')).write_text(source, encoding='utf-8')
        (self.agent.work / 'dialogue.jsonl').write_text(json.dumps(self.agent.dialogue()[0]) + '\n', encoding='utf-8')
        self.confirm_save('one.sav')
        self.finish_args = dict(engine='Fixture', script_language='en-US', speaker_resolution='unavailable',
                                notes='Narration fixture with no speaker table.')
        self.usage = dict(prompt_tokens=120, completion_tokens=45, total_tokens=165,
                          prompt_cache_hit_tokens=100, prompt_cache_miss_tokens=20,
                          completion_tokens_details=dict(reasoning_tokens=31))
        self.messages = [dict(role='system', content='Fixed protocol'), dict(role='user', content='Prepare')]
        self.silence = mock.patch.object(host.sys, 'stderr', new=io.StringIO())
        self.silence.start()
        self.addCleanup(self.silence.stop)

    def confirm_save(self, name):
        path = self.saves / name
        self.agent.samples[str(path)] = 'progress'
        self.agent.tests[str(path)] = dict(matching_anchors=['Current'], coverage='node',
                                           save_verification_version=host.SAVE_VERIFICATION_VERSION,
                                           sha256=host.hashlib.sha256(path.read_bytes()).hexdigest())

    def response(self, name='list_files', args=None, *, reason='tool_calls', usage=None):
        if args is None:
            args = dict(root='save', path='.')
        arguments = args if isinstance(args, str) else json.dumps(args)
        message = dict(role='assistant', content=None, reasoning_content='Retain this model context only',
                       tool_calls=[dict(id='call-' + name, type='function',
                                        function=dict(name=name, arguments=arguments))])
        response = mock.MagicMock()
        response.status = 200
        response.headers = {'x-request-id': 'req-integration'}
        response.read.return_value = json.dumps(dict(id='chatcmpl-integration',
            choices=[dict(finish_reason=reason, message=message)], usage=self.usage if usage is None else usage)).encode()
        response.__enter__.return_value = response
        return response

    def network(self, outcomes):
        opener = mock.Mock()
        opener.open.side_effect = outcomes
        return opener, mock.patch.object(host.urllib.request, 'build_opener', return_value=opener)

    def requests(self):
        path = self.agent.session_dir / 'requests.jsonl'
        return [json.loads(line) for line in path.read_text(encoding='utf-8').splitlines()]

    def session(self):
        return json.loads((self.agent.work / 'session.json').read_text(encoding='utf-8'))

    def test_each_transient_attempt_consumes_budget_and_has_its_own_audit(self):
        response = self.response()
        opener, patcher = self.network([urllib.error.URLError(socket.timeout('private ' + self.token)), response])
        with patcher, mock.patch.object(host.time, 'sleep') as sleep:
            self.agent.run_turn(self.messages, host.TOOLS, 0, 'prepare')
        self.assertEqual(self.agent.requests_sent, 2)
        self.assertEqual(opener.open.call_count, 2)
        self.assertEqual([item['request_number'] for item in self.requests()], [1, 2])
        self.assertEqual(self.requests()[0]['kind'], 'timeout')
        self.assertEqual(self.requests()[1]['finish_reason'], 'tool_calls')
        self.assertEqual(self.requests()[1]['usage'], self.usage)
        self.assertEqual(self.agent.events[0]['session_turn'], 2)
        self.assertEqual(self.agent.events[0]['tool'], 'list_files')
        self.assertIsNone(self.agent.last_failure)
        sleep.assert_called_once_with(1)
        response.close.assert_called_once()
        logs = (self.agent.session_dir / 'requests.jsonl').read_text(encoding='utf-8')
        self.assertNotIn(self.token, logs)
        self.assertNotIn('Retain this model context only', logs)

    def test_running_session_records_real_attempts_and_clears_in_flight_between_retries(self):
        response = self.response()
        outcomes = iter((socket.timeout('temporary connection failure'), response))
        observed_attempts = []
        def read_live_pair():
            live = self.session()
            archived = json.loads((self.agent.session_dir / 'session.json').read_text(encoding='utf-8'))
            self.assertEqual(live, archived)
            self.assertEqual(live['status'], 'running')
            self.assertFalse(live['closed'])
            self.assertEqual(live['session_id'], self.agent.session_id)
            self.assertEqual(live['phase'], 'prepare')
            self.assertNotIn(self.token, json.dumps(live))
            return live
        def open_request(request, timeout):
            state = read_live_pair()
            self.assertTrue(state['in_flight'])
            observed_attempts.append(state['requests_sent'])
            outcome = next(outcomes)
            if isinstance(outcome, BaseException):
                raise outcome
            return outcome
        def retry_backoff(delay):
            state = read_live_pair()
            self.assertFalse(state['in_flight'])
            self.assertEqual(state['requests_sent'], 1)
        opener = mock.Mock()
        opener.open.side_effect = open_request
        with mock.patch.object(host.urllib.request, 'build_opener', return_value=opener), \
                mock.patch.object(host.time, 'sleep', side_effect=retry_backoff) as sleep:
            self.agent.run_turn(self.messages, host.TOOLS, 0, 'prepare')
        self.assertEqual(observed_attempts, [1, 2])
        self.assertEqual(opener.open.call_count, 2)
        self.assertEqual([item['request_number'] for item in self.requests()], [1, 2])
        state = read_live_pair()
        self.assertEqual(state['requests_sent'], 2)
        self.assertFalse(state['in_flight'])
        sleep.assert_called_once_with(1)

    def test_non_authoring_length_retries_once_and_executes_only_complete_reply(self):
        self.agent.workflow.stage = 'resources'
        original_program = (self.agent.work / 'script_extractor.py').read_bytes()
        truncated = self.response('write_program', '{"kind":"script","source":"partial', reason='length')
        complete = self.response()
        opener, patcher = self.network([truncated, complete])
        with patcher, mock.patch.object(self.agent, 'dispatch', wraps=self.agent.dispatch) as dispatch:
            self.agent.run_turn(self.messages, host.TOOLS, 0, 'prepare')
        payloads = [json.loads(call.args[0].data) for call in opener.open.call_args_list]
        self.assertEqual([item['max_tokens'] for item in payloads], [12000, 24000])
        self.assertEqual([item['reasoning_effort'] for item in payloads], ['max', 'max'])
        self.assertEqual(payloads[0]['messages'], payloads[1]['messages'][:len(payloads[0]['messages'])])
        retry = json.loads(payloads[1]['messages'][-1]['content'].split('\n')[0].removeprefix('Host progress: '))
        self.assertEqual(retry['remaining_work_requests'], 4)
        self.assertEqual(payloads[0]['tools'], host.TOOLS)
        self.assertEqual(payloads[1]['tools'], host.TOOLS)
        self.assertNotIn('tool_choice', payloads[0])
        self.assertEqual([call.args[0] for call in dispatch.call_args_list], ['list_files'])
        self.assertEqual((self.agent.work / 'script_extractor.py').read_bytes(), original_program)
        self.assertEqual([item['request_number'] for item in self.requests()], [1, 2])
        self.assertEqual([item['max_tokens'] for item in self.requests()], [12000, 24000])
        self.assertEqual(self.requests()[0]['finish_reason'], 'length')
        self.assertEqual(self.requests()[0]['usage'], self.usage)
        self.assertTrue(all(message.get('tool_calls', [{}])[0].get('function', {}).get('name') != 'write_program'
                            for message in self.messages))

    def test_non_authoring_repeated_length_stops_after_one_expansion_and_never_dispatches(self):
        self.agent.workflow.stage = 'prerequisites'
        opener, patcher = self.network([
            self.response('write_program', '{"source":"partial', reason='length'),
            self.response('write_program', '{"source":"still partial', reason='length')])
        with patcher, mock.patch.object(self.agent, 'dispatch') as dispatch:
            with self.assertRaises(host.TRANSPORT['CompletionError']) as caught:
                self.agent.run_turn(self.messages, host.TOOLS, 0, 'prepare')
        self.assertEqual(caught.exception.kind, 'length')
        self.assertEqual(self.agent.requests_sent, 2)
        self.assertEqual(opener.open.call_count, 2)
        self.assertEqual([item['max_tokens'] for item in self.requests()], [12000, 24000])
        self.assertTrue(all(item['reasoning_effort'] == 'max' for item in self.requests()))
        dispatch.assert_not_called()

    def test_non_authoring_reasoning_only_length_keeps_max_on_retry_and_following_turn(self):
        self.agent.workflow.stage = 'resources'
        exhausted = dict(completion_tokens=12000, completion_tokens_details=dict(reasoning_tokens=12000))
        opener, patcher = self.network([self.response(reason='length', usage=exhausted), self.response(), self.response()])
        with patcher:
            self.agent.run_turn(self.messages, host.TOOLS, 0, 'prepare')
            self.agent.run_turn(self.messages, host.TOOLS, 1, 'prepare')
        payloads = [json.loads(call.args[0].data) for call in opener.open.call_args_list]
        self.assertEqual([item['reasoning_effort'] for item in payloads], ['max', 'max', 'max'])
        self.assertEqual([item['max_tokens'] for item in payloads], [12000, 24000, 24000])

    def test_authoring_stages_start_at_32000_and_never_retry_truncated_program_arguments(self):
        original_program = (self.agent.work / 'script_extractor.py').read_bytes()
        stages = ('script_program', 'script_validation', 'speaker_review', 'save_program', 'save_validation')
        exhausted = dict(completion_tokens=32000, completion_tokens_details=dict(reasoning_tokens=32000))
        for turn, stage in enumerate(stages):
            with self.subTest(stage=stage):
                self.agent.workflow.stage = stage
                truncated = self.response('write_program', '{"kind":"script","source":"partial',
                                          reason='length', usage=exhausted)
                opener, patcher = self.network([truncated])
                with patcher, mock.patch.object(self.agent, 'dispatch') as dispatch:
                    with self.assertRaises(host.TRANSPORT['CompletionError']) as caught:
                        self.agent.run_turn(self.messages, host.TOOLS, turn, 'prepare')
                self.assertEqual(caught.exception.kind, 'length')
                self.assertEqual(caught.exception.max_tokens, 32000)
                opener.open.assert_called_once()
                payload = json.loads(opener.open.call_args.args[0].data)
                self.assertEqual((payload['max_tokens'], payload['reasoning_effort']), (32000, 'max'))
                self.assertEqual(self.requests()[-1]['max_tokens'], 32000)
                self.assertEqual(self.requests()[-1]['reasoning_effort'], 'max')
                dispatch.assert_not_called()
        self.assertEqual(self.agent.requests_sent, len(stages))
        self.assertEqual((self.agent.work / 'script_extractor.py').read_bytes(), original_program)
        self.assertEqual(self.agent.events, [])
        self.assertTrue(all('tool_calls' not in message for message in self.messages))

    def test_all_preparation_stages_keep_max_after_repeated_inspection(self):
        self.agent.request_limit = self.agent.request_ceiling = 150
        self.agent.config['reasoning_effort'] = 'low'
        stages = ('prerequisites', 'resources', 'script_program', 'script_validation',
                  'speaker_review', 'save_program', 'save_validation', 'finish')
        for turn, stage in enumerate(stages):
            with self.subTest(stage=stage):
                self.agent.workflow.stage = stage
                self.agent.workflow.inspection_turns.update(range(8))
                opener, patcher = self.network([self.response()])
                with patcher:
                    self.agent.run_turn(self.messages, host.TOOLS, turn,
                                        'finish' if stage == 'finish' else 'prepare')
                payload = json.loads(opener.open.call_args.args[0].data)
                self.assertEqual(payload['thinking'], dict(type='enabled'))
                self.assertEqual(payload['reasoning_effort'], 'max')
                self.assertEqual(self.requests()[-1]['reasoning_effort'], 'max')
                expected_cap = 32000 if stage in ('script_program', 'script_validation', 'speaker_review',
                                                  'save_program', 'save_validation') else 12000
                self.assertEqual(payload['max_tokens'], expected_cap)
                self.assertEqual(self.requests()[-1]['max_tokens'], expected_cap)

    def test_401_does_not_retry_and_persisted_failure_and_usage_exclude_secrets(self):
        usage = dict(self.usage, provider_extension={'echo': self.token})
        body = dict(id='req-denied', usage=usage, choices=[dict(finish_reason='stop')],
                    error=dict(message='private provider error ' + self.token))
        error = urllib.error.HTTPError('https://api.deepseek.com/chat/completions', 401,
                                       'private ' + self.token, {}, io.BytesIO(json.dumps(body).encode()))
        opener, patcher = self.network([error])
        with patcher, mock.patch.object(host.time, 'sleep') as sleep:
            with self.assertRaises(host.TRANSPORT['CompletionError']) as caught:
                self.agent.run()
        self.assertEqual(caught.exception.kind, 'http')
        self.assertEqual(caught.exception.http_status, 401)
        self.assertEqual(self.agent.requests_sent, 1)
        opener.open.assert_called_once()
        sleep.assert_not_called()
        session = self.session()
        self.assertEqual(session['status'], 'failed')
        self.assertTrue(session['closed'])
        self.assertEqual(session['failure']['http_status'], 401)
        self.assertFalse(session['failure']['retryable'])
        self.assertEqual(session['failure']['usage']['provider_extension']['echo'], '[redacted]')
        self.assertEqual(self.requests()[0]['usage'], session['failure']['usage'])
        for path in (self.agent.work / 'session.json', self.agent.session_dir / 'session.json',
                     self.agent.session_dir / 'requests.jsonl'):
            logged = path.read_text(encoding='utf-8')
            self.assertNotIn(self.token, logged)
            self.assertNotIn('private provider error', logged)
            self.assertNotIn('Retain this model context only', logged)

    def test_prepare_retry_can_validate_then_use_the_reserved_finish_request(self):
        self.agent.request_limit = self.config['max_turns'] = 3
        opener, patcher = self.network([
            socket.timeout('private timeout'),
            self.response('test_program', dict(kind='save', root='save', path='two.sav')),
            self.response('finish', self.finish_args)])
        with patcher, mock.patch.object(host.time, 'sleep'):
            result = self.agent.run()
        self.assertEqual(set(result['validated_saves']), {str(self.saves / 'two.sav')})
        self.assertEqual(self.agent.requests_sent, 3)
        self.assertEqual(opener.open.call_count, 3)
        self.assertEqual([item['request_number'] for item in self.requests()], [1, 2, 3])
        self.assertEqual([(event['tool'], event['phase'], event['session_turn']) for event in self.agent.events],
                         [('test_program', 'prepare', 2), ('finish', 'finish', 3)])
        self.assertEqual(self.session()['status'], 'completed')
        self.assertIsNone(self.session()['failure'])

    def test_prepare_retries_stop_before_spending_finish_reserve(self):
        self.agent.request_limit = self.config['max_turns'] = 3
        opener, patcher = self.network([socket.timeout('first'), socket.timeout('second')])
        with patcher, mock.patch.object(host.time, 'sleep'):
            with self.assertRaises(host.TRANSPORT['CompletionError']):
                self.agent.run()
        self.assertEqual(self.agent.request_ceiling, 2)
        self.assertEqual(self.agent.requests_sent, 2)
        self.assertEqual(opener.open.call_count, 2)
        self.assertEqual([item['request_number'] for item in self.requests()], [1, 2])
        self.assertEqual(self.agent.events, [])
        self.assertEqual(self.session()['requests_sent'], 2)
        self.assertEqual(self.session()['request_limit'], 3)
        self.assertFalse((self.agent.work / 'result.json').exists())

    def test_finish_phase_can_retry_and_clear_the_previous_failure(self):
        self.confirm_save('two.sav')
        body = io.BytesIO(json.dumps(dict(usage=self.usage)).encode())
        error = urllib.error.HTTPError('https://api.deepseek.com/chat/completions', 503, 'unavailable', {}, body)
        opener, patcher = self.network([error, self.response('finish', self.finish_args)])
        with patcher, mock.patch.object(host.time, 'sleep'):
            result = self.agent.run()
        self.assertEqual(set(result['validated_saves']), {str(self.saves / 'two.sav')})
        self.assertEqual(self.agent.requests_sent, 2)
        self.assertEqual(self.agent.events[0]['phase'], 'finish')
        self.assertEqual([item['request_number'] for item in self.requests()], [1, 2])
        self.assertEqual(self.requests()[0]['usage'], self.usage)
        self.assertIsNone(self.session()['failure'])

    def test_finish_retry_still_rejects_program_changes_from_the_full_schema(self):
        self.confirm_save('two.sav')
        program_path = self.agent.work / 'script_extractor.py'
        original_program = program_path.read_bytes()
        opener, patcher = self.network([
            socket.timeout('temporary'),
            self.response('write_program', dict(kind='script', source='invalid replacement')),
            self.response('finish', self.finish_args)])
        with patcher, mock.patch.object(host.time, 'sleep'):
            self.agent.run()
        self.assertEqual(opener.open.call_count, 3)
        self.assertEqual(program_path.read_bytes(), original_program)
        self.assertEqual(self.agent.events[0]['tool'], 'write_program')
        self.assertEqual(self.agent.events[0]['phase'], 'finish')
        self.assertIn('Only the currently offered tools', self.agent.events[0]['error'])
        self.assertEqual(self.agent.events[1]['tool'], 'finish')
        payloads = [json.loads(call.args[0].data) for call in opener.open.call_args_list]
        self.assertTrue(all(payload['tools'] == host.TOOLS for payload in payloads))

    def test_recent_validation_unlocks_the_default_120_to_150_extension(self):
        self.assertEqual((host.BASE_TURNS, host.MAX_TURNS), (120, 150))
        self.agent.request_limit = self.config['max_turns'] = host.MAX_TURNS
        self.agent.requests_sent = host.BASE_TURNS - 2
        self.agent.workflow.verified_at = self.agent.requests_sent
        opener, patcher = self.network([
            self.response(),
            self.response('test_program', dict(kind='save', root='save', path='two.sav')),
            self.response('finish', self.finish_args)])
        ceilings = []
        real_completion = self.agent.request_completion
        def record_ceiling(messages, tools):
            ceilings.append(self.agent.request_ceiling)
            return real_completion(messages, tools)
        with patcher, mock.patch.object(self.agent, 'request_completion', side_effect=record_ceiling):
            result = self.agent.run()
        self.assertEqual(set(result['validated_saves']), {str(self.saves / 'two.sav')})
        self.assertEqual(opener.open.call_count, 3)
        self.assertTrue(self.agent.extended)
        self.assertEqual(ceilings, [119, 149, 150])
        self.assertEqual([item['request_number'] for item in self.requests()], [119, 120, 121])
        self.assertEqual(self.session()['request_limit'], 150)

    def test_retry_at_119_unlocks_extension_after_recent_validation(self):
        self.agent.request_limit = self.config['max_turns'] = host.MAX_TURNS
        self.agent.requests_sent = host.BASE_TURNS - 2
        self.agent.workflow.verified_at = self.agent.requests_sent
        opener, patcher = self.network([
            socket.timeout('temporary failure at baseline boundary'),
            self.response('test_program', dict(kind='save', root='save', path='two.sav')),
            self.response('finish', self.finish_args)])
        ceilings = []
        real_completion = self.agent.request_completion
        def record_ceiling(messages, tools):
            ceilings.append(self.agent.request_ceiling)
            return real_completion(messages, tools)
        with patcher, mock.patch.object(host.time, 'sleep') as sleep, \
                mock.patch.object(self.agent, 'request_completion', side_effect=record_ceiling), \
                mock.patch.object(self.agent.workflow, 'extension_allowed',
                                  wraps=self.agent.workflow.extension_allowed) as extension:
            result = self.agent.run()
        self.assertEqual(set(result['validated_saves']), {str(self.saves / 'two.sav')})
        self.assertEqual(opener.open.call_count, 3)
        self.assertEqual(self.agent.requests_sent, 121)
        self.assertTrue(self.agent.extended)
        self.assertEqual(ceilings, [119, 149, 150])
        extension.assert_called_once()
        self.assertEqual(extension.call_args.args[0], 119)
        self.assertEqual([item['request_number'] for item in self.requests()], [119, 120, 121])
        self.assertEqual(self.requests()[0]['kind'], 'timeout')
        self.assertEqual([(event['tool'], event['phase'], event['session_turn']) for event in self.agent.events],
                         [('test_program', 'prepare', 120), ('finish', 'finish', 121)])
        payloads = [json.loads(call.args[0].data) for call in opener.open.call_args_list]
        original = json.loads(payloads[0]['messages'][-1]['content'].split('\n')[0].removeprefix('Host progress: '))
        retry = json.loads(payloads[1]['messages'][-1]['content'].split('\n')[0].removeprefix('Host progress: '))
        self.assertEqual((original['remaining_work_requests'], original['request_limit']), (1, 120))
        self.assertEqual((retry['remaining_work_requests'], retry['remaining_requests'], retry['request_limit']),
                         (30, 31, 150))
        self.assertEqual(self.session()['status'], 'completed')
        self.assertTrue(self.session()['extension_used'])
        sleep.assert_called_once_with(1)

    def test_retry_at_hard_work_ceiling_still_preserves_request_150_for_finish(self):
        self.agent.request_limit = self.config['max_turns'] = host.MAX_TURNS
        self.agent.requests_sent = host.MAX_TURNS - 2
        self.agent.workflow.verified_at = self.agent.requests_sent
        self.agent.extended = True
        opener, patcher = self.network([socket.timeout('failure at extended work boundary')])
        with patcher, mock.patch.object(host.time, 'sleep') as sleep:
            with self.assertRaises(host.TRANSPORT['CompletionError']) as caught:
                self.agent.run()
        self.assertEqual(caught.exception.kind, 'timeout')
        self.assertEqual(self.agent.request_ceiling, 149)
        self.assertEqual(self.agent.requests_sent, 149)
        opener.open.assert_called_once()
        sleep.assert_not_called()
        self.assertEqual(self.agent.events, [])
        self.assertEqual(self.session()['requests_sent'], 149)
        self.assertEqual(self.session()['request_limit'], 150)
        self.assertFalse((self.agent.work / 'result.json').exists())


    def test_length_expansion_is_retained_until_the_stage_changes(self):
        self.agent.workflow.stage = 'resources'
        opener, patcher = self.network([self.response(reason='length'), self.response(), self.response(),
                                       self.response(), self.response()])
        with patcher:
            self.agent.run_turn(self.messages, host.TOOLS, 0, 'prepare')
            self.agent.run_turn(self.messages, host.TOOLS, 1, 'prepare')
            self.agent.workflow.stage = 'save_program'
            self.agent.run_turn(self.messages, host.TOOLS, 2, 'prepare')
            self.agent.workflow.stage = 'resources'
            self.agent.run_turn(self.messages, host.TOOLS, 3, 'prepare')
        payloads = [json.loads(call.args[0].data) for call in opener.open.call_args_list]
        self.assertEqual([payload['max_tokens'] for payload in payloads], [12000, 24000, 24000, 32000, 12000])
        self.assertTrue(all(payload['reasoning_effort'] == 'max' for payload in payloads))
        self.assertEqual(self.agent.requests_sent, 5)

    def test_expanded_output_state_survives_compatible_checkpoint_only_with_allowed_caps(self):
        self.agent.work_guidance()
        self.agent.output_limit_stage, self.agent.output_limit = self.agent.workflow.stage, 24000
        self.agent.checkpoint(self.messages)
        saved = json.loads((self.agent.work / 'checkpoint.json').read_text('utf-8'))
        saved['output_limit_state']['reasoning_effort'] = 'low'
        restored = host.Agent(self.config)
        restored.programs = dict(self.agent.programs)
        self.assertTrue(restored.restore_checkpoint(saved))
        self.assertEqual((restored.output_limit_stage, restored.output_limit), ('save_validation', 24000))
        self.assertEqual(restored.reasoning_effort, 'max')
        opener, patcher = self.network([self.response()])
        with patcher:
            restored.run_turn(self.messages, host.TOOLS, 0, 'prepare')
        resumed_payload = json.loads(opener.open.call_args.args[0].data)
        self.assertEqual((resumed_payload['max_tokens'], resumed_payload['reasoning_effort']), (32000, 'max'))
        for unsafe in (64000, True, '32000'):
            saved['output_limit_state']['max_tokens'] = unsafe
            restored.restore_checkpoint(saved)
            self.assertEqual((restored.output_limit_stage, restored.output_limit), (None, 12000))

    def test_resume_uses_bounded_failed_length_cap_behind_rust_start_marker(self):
        self.agent.work_guidance()
        self.agent.checkpoint(self.messages)
        saved = json.loads((self.agent.work / 'checkpoint.json').read_text('utf-8'))
        saved.pop('output_limit_state', None)
        host.atomic_json(self.agent.work / 'session.json', dict(status='running', closed=False))
        host.atomic_json(self.agent.work / 'previous-session.json', dict(session_id='failed-session', closed=True,
                         stage='save_validation', failure=dict(kind='length', max_tokens=24000,
                         usage=dict(completion_tokens=24000, completion_tokens_details=dict(reasoning_tokens=24000)))))
        restored = host.Agent(self.config)
        restored.programs = dict(self.agent.programs)
        self.assertTrue(restored.restore_checkpoint(saved))
        self.assertEqual((restored.output_limit_stage, restored.output_limit), ('save_validation', 32000))
        self.assertEqual(restored.reasoning_effort, 'max')

    def test_maximum_output_limit_does_not_retry_the_identical_length_cap(self):
        self.agent.workflow.stage = 'resources'
        self.agent.output_limit_stage, self.agent.output_limit = self.agent.workflow.stage, 32000
        opener, patcher = self.network([self.response(reason='length')])
        with patcher:
            with self.assertRaises(host.TRANSPORT['CompletionError']):
                self.agent.run_turn(self.messages, host.TOOLS, 0, 'prepare')
        opener.open.assert_called_once()
        payload = json.loads(opener.open.call_args.args[0].data)
        self.assertEqual((payload['max_tokens'], payload['reasoning_effort']), (32000, 'max'))
        self.assertEqual(self.agent.events, [])


if __name__ == '__main__':
    unittest.main()
