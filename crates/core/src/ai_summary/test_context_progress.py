"""Progress projection and reset headroom, using synthetic messages only."""
import copy
import importlib.util
import json
import pathlib
import tempfile
import types
import unittest
import urllib.error
from unittest import mock

import test_transport_integration as transport_fixtures


SPEC = importlib.util.spec_from_file_location('progress_host', pathlib.Path(__file__).with_name('agent_host.py'))
host = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(host)
context = host.CONTEXT


def large_progress():
    goal = 'Repair the current byte-evidence schema on the exact failed target; preserve current code. ' * 20
    return dict(dialogue_count=0, validated_saves=0, last_script_failure=dict(
        kind='script', category='script', root='work', path='unpacked/sample.lng',
        error='Actual synthetic validation failure: ' + 'field detail ' * 50),
        last_save_failure=None, phase='prepare', remaining_requests=47, remaining_work_requests=46,
        request_limit=120, hard_request_limit=150,
        guidance=dict(stage='script_validation', goal=goal, gate_instruction=goal, next_action=goal),
        speaker_metadata_progress=dict(next_step='retest_sample',
            sample=dict(root='work', path='unpacked/sample.lng', rows=0, named_rows=0),
            global_resources=[], empty_resources=[],
            scoped_resources=[dict(key='messages:chapter_' + str(i), records=100,
                helper_file='metadata-helpers/' + 'a' * 64 + '.py',
                input_access='metadata["speaker_resources"]["messages:chapter_' + str(i) + '"]',
                target=dict(root='work', path='unpacked/0123456789ab/chapter_' + str(i) + '.lng'),
                sample_entry=dict(entry='message_0', id=0, text='Synthetic name')) for i in range(43)]),
        transcript_coverage=dict(expected_members=45, covered_members=0,
            pending_members=[dict(archive='F:/' + 'synthetic-directory/' * 8 + 'Script.arc',
                                  member='chapter_' + str(i) + '.lng') for i in range(45)]))


class ContextProgressTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.work = pathlib.Path(temporary.name)
        (self.work / 'tool-results').mkdir()

    def value(self, message):
        return json.JSONDecoder().raw_decode(message['content'][len('Host progress: '):])[0]

    def stub(self, status):
        source = '# exact CRLF\r\ndef extract(data, metadata): return []\r\n'
        inventory = dict(version=1, root='game', provided=False, supplied_entries=0,
                         excluded_entries=0, total_entries=0, entries=[])
        host.atomic_json(self.work / 'source-inventory.json', inventory)
        stub = types.SimpleNamespace(work=self.work, reset_context=True, context_resets=0,
            context_system='S' * 7224, context_task=json.dumps(dict(task='Synthetic only')),
            audit_messages=[], tests={}, programs={'script': source},
            read_skill=True, read_contract=True, used_skill=True, resource_selections={}, samples={},
            speaker_resource_targets={}, speaker_resources={}, metadata_helpers={}, required_script_sets={},
            extracted_from={}, documents={}, analysis_needs_recheck=True,
            analysis_notes=dict(findings={}, next_steps=[]), last_script_failure=status['last_script_failure'],
            last_save_failure=None, config={}, source_inventory=inventory,
            workflow=types.SimpleNamespace(snapshot=lambda: {}),
            research=types.SimpleNamespace(snapshot=lambda: {}),
            current_script_roles=lambda: {},
            model_capabilities=dict(status='unavailable', kind='offline_fixture'),
            helper_sources=lambda: {}, native_link_repair=lambda: None,
            preflight_status=lambda: {},
            preparation_status=lambda: copy.deepcopy(status),
            speaker_metadata_progress=lambda: copy.deepcopy(status.get('speaker_metadata_progress', {})),
            artifact_hashes=lambda: {}, checkpoint=lambda messages: None)

        def append(messages, message):
            messages.append(message)
            stub.audit_messages.append(message)
        stub.append_message = append
        return stub

    def test_large_inventory_update_is_bounded_and_guidance_has_one_canonical_copy(self):
        progress = large_progress()
        raw = dict(role='user', content='Host progress: ' + json.dumps(progress, ensure_ascii=False) + '\nBudget')
        self.assertGreater(len(context['encode'](raw)), 30000)
        message = context['progress_message'](self.work, progress, 'Budget')
        self.assertLessEqual(len(context['encode'](message)), context['MAX_PROGRESS_MESSAGE_BYTES'])
        value = self.value(message)
        self.assertEqual(value['guidance']['next_action'], progress['guidance']['next_action'])
        self.assertNotIn('goal', value['guidance'])
        self.assertNotIn('gate_instruction', value['guidance'])
        self.assertEqual(value['last_script_failure'], progress['last_script_failure'])
        archived = json.loads((self.work / value['full_progress']).read_text('utf-8'))
        self.assertEqual(archived, progress)
        self.assertEqual(value['speaker_metadata_progress']['sample'], progress['speaker_metadata_progress']['sample'])

    def test_small_updates_preserve_counts_and_distinct_gate_instruction(self):
        progress = dict(dialogue_count=42, remaining_work_requests=11,
                        guidance=dict(goal='Do the work', next_action='Do the work', gate_instruction='Test first'))
        before = copy.deepcopy(progress)
        value = self.value(context['progress_message'](self.work, progress, 'Exact instruction'))
        self.assertEqual(value['dialogue_count'], 42)
        self.assertEqual(value['remaining_work_requests'], 11)
        self.assertEqual(value['guidance']['next_action'], 'Do the work')
        self.assertEqual(value['guidance']['gate_instruction'], 'Test first')
        self.assertNotIn('goal', value['guidance'])
        self.assertEqual(progress, before)

    def test_reset_avoids_duplicate_update_and_next_compact_update_fits_after_full_reply(self):
        progress = large_progress()
        status = {key: value for key, value in progress.items() if key not in
                  ('guidance', 'phase', 'remaining_requests', 'remaining_work_requests', 'request_limit', 'hard_request_limit')}
        stub = self.stub(status)
        original_reset = context['reset_messages']
        captured = {}

        def realistic_baseline(work, system, task, facts, audit, structural_preview=None):
            captured['facts'] = facts
            captured['structural_preview'] = structural_preview
            messages = original_reset(work, system, task, facts, audit, structural_preview=structural_preview)
            checkpoint = json.loads(messages[-1]['content'])
            # Match the observed 102KB baseline shape without loading any real
            # checkpoint, helper, parser, response or game content in a test.
            padding = max(0, 102340 - len(context['encode'](messages)) - 30)
            checkpoint['synthetic_fixture_padding'] = 'P' * padding
            messages[-1]['content'] = json.dumps(checkpoint, ensure_ascii=False)
            return messages

        messages = [dict(role='user', content='old state')]
        with mock.patch.dict(context, reset_messages=realistic_baseline):
            host.Agent.prepare_request(stub, messages, progress, 'Budget')
            self.assertEqual(stub.context_resets, 1)
            self.assertEqual(len(messages), 3)
            self.assertFalse(any(message.get('content', '').startswith('Host progress: ') for message in messages))
            self.assertEqual(captured['facts']['program_sources']['script']['source'], stub.programs['script'])
            self.assertEqual(captured['facts']['last_script_failure'], status['last_script_failure'])
            self.assertEqual(captured['facts']['current_request'], progress)
            self.assertEqual(captured['facts']['request_budget_instruction'], 'Budget')
            inventory_ref = captured['facts']['source_inventory']['read_json']
            self.assertEqual(inventory_ref, dict(root='work', path='source-inventory.json', pointer='/entries', offset=0, limit=20))
            self.assertEqual(json.loads((self.work / inventory_ref['path']).read_text('utf-8')), stub.source_inventory)
            self.assertIs(captured['structural_preview'], host.RESEARCH['structural_preview'])
            # Approximately the actual 20,783B completed assistant/tool growth.
            messages += [dict(role='assistant', reasoning_content='R' * 16131,
                              tool_calls=[dict(id='synthetic', function=dict(name='verification_contract', arguments='{}'))]),
                         dict(role='tool', tool_call_id='synthetic', content='T' * 4012)]
            host.Agent.prepare_request(stub, messages, progress, 'Budget')
        self.assertEqual(stub.context_resets, 1)
        self.assertLessEqual(len(context['encode'](messages)), context['MAX_CONTEXT_BYTES'])
        self.assertEqual(sum(message.get('content', '').startswith('Host progress: ') for message in messages), 1)

    def test_small_followup_does_not_reset_a_context_with_available_capacity(self):
        progress = large_progress()
        stub = self.stub(progress)
        stub.reset_context = False
        messages = [dict(role='system', content='Small system')]
        host.Agent.prepare_request(stub, messages, dict(remaining_work_requests=3), 'Finish checks')
        self.assertEqual(stub.context_resets, 0)
        self.assertEqual(self.value(messages[-1])['remaining_work_requests'], 3)

    def test_history_above_the_previous_limit_remains_exact_with_the_larger_budget(self):
        progress = large_progress()
        stub = self.stub(progress)
        stub.reset_context = False
        messages = [dict(role='system', content='Synthetic system'),
                    dict(role='assistant', content='H' * 240000),
                    dict(role='tool', content='Synthetic completed tool result')]
        previous = copy.deepcopy(messages)
        self.assertGreater(len(context['encode'](messages)), 160000)
        host.Agent.prepare_request(stub, messages, progress, 'Continue the exact current work')
        self.assertEqual(stub.context_resets, 0)
        self.assertEqual(messages[:-1], previous)
        self.assertEqual(self.value(messages[-1])['last_script_failure'], progress['last_script_failure'])
        self.assertLessEqual(len(context['encode'](messages)), context['MAX_CONTEXT_BYTES'])

    def test_exact_capacity_is_retained_and_the_next_update_resets_without_duplication(self):
        progress = large_progress()
        stub = self.stub(progress)
        stub.reset_context = False
        update = context['progress_message'](self.work, progress, 'Exact boundary')
        message = dict(role='user', content='')
        padding = context['MAX_CONTEXT_BYTES'] - len(context['encode']([message, update]))
        message['content'] = 'X' * padding
        messages = [message]
        host.Agent.prepare_request(stub, messages, progress, 'Exact boundary')
        self.assertEqual(len(context['encode'](messages)), context['MAX_CONTEXT_BYTES'])
        self.assertEqual(stub.context_resets, 0)
        host.Agent.prepare_request(stub, messages, progress, 'Exact boundary')
        self.assertEqual(stub.context_resets, 1)
        self.assertEqual(len(messages), 3)
        self.assertFalse(any(m.get('content', '').startswith('Host progress: ') for m in messages))
        checkpoint = json.loads(messages[-1]['content'])
        self.assertEqual(checkpoint['state']['last_script_failure'], progress['last_script_failure'])
        self.assertEqual(checkpoint['state']['program_sources']['script']['source'], stub.programs['script'])
        self.assertLessEqual(len(context['encode'](messages)), context['MAX_CONTEXT_BYTES'])

    def test_budget_overflow_never_truncates_current_failure(self):
        failure = dict(kind='script', error='E' * (context['MAX_PROGRESS_MESSAGE_BYTES'] + 1),
                       evidence=dict(offset=4, width=4))
        with self.assertRaisesRegex(RuntimeError, 'progress message exceeds'):
            context['progress_message'](self.work, dict(last_script_failure=failure), 'Budget')
        archived = [json.loads(path.read_text('utf-8')) for path in (self.work / 'tool-results').glob('*.json')]
        self.assertTrue(any(value == dict(last_script_failure=failure) for value in archived))

    def test_reset_still_rejects_an_unavoidably_oversized_request(self):
        progress = large_progress()
        stub = self.stub(progress)
        messages = []
        with mock.patch.dict(context, reset_messages=lambda *args, **kwargs:
                             [dict(role='user', content='X' * (context['MAX_CONTEXT_BYTES'] + 1))]):
            with self.assertRaisesRegex(RuntimeError, 'request context exceeds'):
                host.Agent.prepare_request(stub, messages, progress, 'Budget')
        self.assertEqual(context['MAX_CONTEXT_BYTES'], 480000)
        self.assertEqual(context['VERSION'], 17)


class ContextProgressTransportTests(unittest.TestCase):
    setUp = transport_fixtures.TransportIntegrationTests.setUp
    confirm_save = transport_fixtures.TransportIntegrationTests.confirm_save
    response = transport_fixtures.TransportIntegrationTests.response
    network = transport_fixtures.TransportIntegrationTests.network
    requests = transport_fixtures.TransportIntegrationTests.requests
    session = transport_fixtures.TransportIntegrationTests.session

    def retry_case(self, first):
        protocol = transport_fixtures.host
        self.agent.workflow.stage = 'resources'
        self.agent.context_system = 'Synthetic system'
        self.agent.context_task = json.dumps(dict(task='Synthetic retry'))
        self.messages = [dict(role='system', content='Synthetic system'),
                         dict(role='user', content='X' * (protocol.CONTEXT['MAX_CONTEXT_BYTES'] - 3000))]
        response = self.response()
        opener, patcher = self.network([first, response])
        before_program = (self.agent.work / 'script_extractor.py').read_bytes()
        with patcher, mock.patch.object(protocol.time, 'sleep'), \
                mock.patch.object(self.agent, 'preparation_status', return_value=large_progress()), \
                mock.patch.object(self.agent, 'dispatch', wraps=self.agent.dispatch) as dispatch:
            self.agent.run_turn(self.messages, protocol.TOOLS, 0, 'prepare')
        payloads = [json.loads(call.args[0].data) for call in opener.open.call_args_list]
        self.assertEqual(len(payloads), 2)
        self.assertTrue(all(len(protocol.CONTEXT['encode'](payload['messages'])) <= protocol.CONTEXT['MAX_CONTEXT_BYTES'] for payload in payloads))
        self.assertEqual(self.agent.context_resets, 1)
        initial = json.loads(payloads[1]['messages'][-1]['content'])
        self.assertEqual(initial['kind'], 'verified_host_checkpoint')
        self.assertEqual(initial['state']['current_request']['remaining_work_requests'], 4)
        self.assertIn('transport attempt failed before tool execution', initial['state']['request_budget_instruction'])
        self.assertEqual([call.args[0] for call in dispatch.call_args_list], ['list_files'])
        self.assertEqual((self.agent.work / 'script_extractor.py').read_bytes(), before_program)
        self.assertEqual(self.agent.requests_sent, 2)
        self.assertEqual(len(self.requests()), 2)
        self.assertEqual(payloads[0]['tools'], payloads[1]['tools'])
        return payloads

    def test_retryable_http_uses_compact_progress_and_checks_capacity_before_retry(self):
        self.retry_case(urllib.error.HTTPError('https://fixture.invalid', 503, 'temporary', {}, None))

    def test_length_retry_keeps_budget_and_never_executes_partial_program(self):
        payloads = self.retry_case(self.response('write_program', '{"kind":"script","source":"partial', reason='length'))
        self.assertEqual([payload['max_tokens'] for payload in payloads], [12000, 24000])


if __name__ == '__main__':
    unittest.main()
