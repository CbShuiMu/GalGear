"""Offline host regressions for research provenance and upgrade/restart behavior."""
import hashlib
import importlib.util
import json
import os
import pathlib
import tempfile
import unittest
from unittest import mock

spec = importlib.util.spec_from_file_location('research_integration_host', pathlib.Path(__file__).with_name('agent_host.py'))
host = importlib.util.module_from_spec(spec)
spec.loader.exec_module(host)


class ResearchIntegrationTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = pathlib.Path(temporary.name)
        (self.root / 'fixture.exe').write_bytes(b'fixture')
        (self.root / 'scene.csx').write_bytes(b'CSX_fixture')
        skill, saves = self.root / 'skill', self.root / 'saves'
        skill.mkdir()
        saves.mkdir()
        (saves / 'current.sav').write_bytes(b'SAVE_fixture')
        self.config = dict(game=str(self.root), save=str(saves), skill=str(skill),
                           work=str(self.root / 'work'), title='Fixture', language='en-US',
                           provider='deepseek', model='fixture', max_turns=10)
        self.serial = 0

    def agent(self):
        agent = host.Agent(self.config)
        agent.audit_messages = []
        agent.context_system = 'Resume validated prerequisites and model research.'
        agent.context_task = json.dumps(dict(task='Fixture research'))
        return agent

    def read(self, agent, value=None, root='game', path='scene.csx'):
        self.serial += 1
        call_id = 'real-file-inspection-' + str(self.serial)
        function = dict(name='read_bytes', arguments=json.dumps(dict(
            root=root, path=path, offset=0, length=8)))
        reply = dict(role='assistant', content='', reasoning_content='fixture private reasoning',
                     tool_calls=[dict(id=call_id, type='function', function=function)])
        messages = []
        if value is None:
            with mock.patch.object(agent, 'request_completion', return_value=reply):
                agent.run_turn(messages, host.TOOLS, self.serial, 'prepare')
        else:
            with mock.patch.object(agent, 'request_completion', return_value=reply), mock.patch.object(agent, 'dispatch', return_value=value):
                agent.run_turn(messages, host.TOOLS, self.serial, 'prepare')
        return call_id, json.loads(messages[-1]['content'])

    @staticmethod
    def finding(identity, ref):
        return dict(id=identity, status='observed', summary='The inspected member starts at input byte zero.',
                    evidence_refs=[ref], coordinates={'member': {'base': 'input_file_byte_zero', 'offset': 0}})

    def test_bounded_api_provenance_does_not_modify_raw_audit_or_full_record(self):
        agent = self.agent()
        raw = dict(offset=0, length=8, content='EXCERPT_' * 3000)
        call_id, api = self.read(agent, raw)
        self.assertTrue(api['truncated'])
        self.assertEqual(api['observation_ref'], 'O1')
        self.assertEqual(api['source_context']['coordinate_base'], 'input_file_byte_zero')
        self.assertEqual(api['source_context']['sha256'], hashlib.sha256((self.root / 'scene.csx').read_bytes()).hexdigest())
        actual = json.loads(next(message['content'] for message in agent.audit_messages
                                 if message.get('role') == 'tool' and message['tool_call_id'] == call_id))
        self.assertEqual(actual, raw)
        observation = agent.research.resolve('O1')
        full_record = json.loads((agent.work / observation['full_record']).read_text('utf-8'))
        self.assertEqual(full_record['result'], raw)
        self.assertNotIn('observation_ref', actual)
        self.assertNotIn('source_context', actual)
        ledger_text = (agent.work / 'analysis-ledger.json').read_text('utf-8')
        self.assertNotIn('fixture private reasoning', ledger_text)
        self.assertNotIn('EXCERPT_', ledger_text)

    def test_one_mistyped_legacy_call_id_saves_other_four_findings(self):
        agent = self.agent()
        call_id, api = self.read(agent)
        items = [self.finding('finding-' + str(index), call_id) for index in range(5)]
        items[2]['evidence_refs'] = [call_id[:-1] + 'TYPO']
        # Older clients used this key and sometimes supplied a very long ID.
        for item in items:
            item['evidence_call_ids'] = item.pop('evidence_refs')
        result = agent.dispatch('record_findings', dict(findings=items, next_steps=['Write the minimal script parser.']))
        self.assertEqual(len(result['accepted']), 4)
        self.assertEqual(len(result['errors']), 1)
        self.assertEqual(result['errors'][0]['id'], 'finding-2')
        self.assertIn(api['observation_ref'], result['errors'][0]['references'][0]['closest_refs'])
        self.assertEqual(len(agent.analysis_notes['findings']), 4)
        saved = json.loads((agent.work / 'analysis-notes.json').read_text('utf-8'))
        self.assertEqual(len(saved['findings']), 4)
        self.assertEqual(saved['next_steps'], ['Write the minimal script parser.'])
        self.assertFalse(agent.preparation_status()['ready_for_finish'])

    def test_first_upgrade_preserves_old_notes_before_host_loads_audit(self):
        work = self.root / 'work'
        work.mkdir()
        legacy_function = dict(name='inspect_data', arguments='{"root":"game","path":"scene.csx"}')
        old_audit = [dict(role='assistant', reasoning_content='DO NOT COPY PRIVATE REASONING',
                          tool_calls=[dict(id='old-real-call', function=legacy_function)]),
                     dict(role='tool', tool_call_id='old-real-call', content='{"offset":0,"count":9}')]
        old_notes = dict(model_authored=True, needs_recheck=False,
                         findings={'legacy-layout': dict(status='observed', summary='Member offset is zero.',
                             evidence=[dict(tool_call_id='old-real-call', full_record='old.json')])},
                         next_steps=['Implement the parser.'])
        (work / 'conversation.json').write_text(json.dumps(old_audit), encoding='utf-8')
        (work / 'analysis-notes.json').write_text(json.dumps(old_notes), encoding='utf-8')
        # Agent.__init__ has not loaded conversation.json into audit_messages yet.
        agent = host.Agent(self.config)
        self.assertIn('legacy-layout', agent.analysis_notes['findings'])
        self.assertEqual(agent.analysis_notes['findings']['legacy-layout']['evidence_refs'], ['O1'])
        preserved = json.loads((work / 'analysis-notes-legacy.json').read_text('utf-8'))
        self.assertEqual(preserved, old_notes)
        self.assertNotIn('DO NOT COPY PRIVATE REASONING', (work / 'analysis-ledger.json').read_text('utf-8'))

    def test_legacy_notes_without_audit_remain_preserved_and_unverified(self):
        work = self.root / 'work'
        work.mkdir()
        old_notes = dict(model_authored=True, findings={'missing-proof': dict(
            status='observed', summary='Unresolved old structural claim.', evidence=[dict(tool_call_id='unavailable')])},
            next_steps=['Locate archived input evidence.'])
        (work / 'analysis-notes.json').write_text(json.dumps(old_notes), encoding='utf-8')
        agent = host.Agent(self.config)
        self.assertEqual(json.loads((work / 'analysis-notes-legacy.json').read_text('utf-8')), old_notes)
        self.assertNotIn('missing-proof', agent.analysis_notes['findings'])
        self.assertEqual(agent.research.snapshot()['legacy_notes_file'], 'analysis-notes-legacy.json')
        self.assertFalse(agent.preparation_status()['ready_for_finish'])

    def test_restore_rechecks_input_seen_only_by_inspection(self):
        agent = self.agent()
        call_id, _ = self.read(agent, root='save', path='current.sav')
        agent.dispatch('record_findings', dict(findings=[self.finding('member-layout', call_id)], next_steps=[]))
        agent.checkpoint([])
        saved = json.loads((agent.work / 'checkpoint.json').read_text('utf-8'))
        self.assertEqual(saved['source_stamps'], {})
        (self.root / 'saves' / 'current.sav').write_bytes(b'Different current save bytes')
        restored = host.Agent(self.config)
        restored.audit_messages = list(agent.audit_messages)
        restored.restore_checkpoint(saved)
        self.assertTrue(restored.research.resolve('O1')['stale'])
        self.assertTrue(restored.analysis_notes['findings']['member-layout']['stale'])
        self.assertTrue(restored.analysis_needs_recheck)

    def test_compaction_keeps_short_ref_coordinate_base_and_actual_input_hash(self):
        agent = self.agent()
        call_id, api = self.read(agent)
        agent.dispatch('record_findings', dict(findings=[self.finding('member-layout', 'latest')], next_steps=[]))
        agent.reset_context = True
        messages = []
        agent.prepare_request(messages, dict(phase='prepare', remaining_work_requests=3), 'Continue')
        checkpoint = json.loads(messages[2]['content'])
        entry = checkpoint['structural_ledger']['observations'][0]
        self.assertEqual(entry['ref'], api['observation_ref'])
        self.assertEqual(entry['tool_call_id'], call_id)
        self.assertEqual(entry['inputs'][0]['coordinate_base'], 'input_file_byte_zero')
        self.assertEqual(entry['inputs'][0]['sha256'], api['source_context']['sha256'])
        self.assertEqual(checkpoint['model_analysis']['findings']['member-layout']['evidence_refs'], ['O1'])

    def test_input_hash_updates_after_same_length_same_mtime_mutation(self):
        agent = self.agent()
        args = dict(root='game', path='scene.csx')
        first = agent.input_description('inspect_data', args)
        path = self.root / 'scene.csx'
        info = path.stat()
        path.write_bytes(b'CHANGED_NOW')
        self.assertEqual(path.stat().st_size, info.st_size)
        os.utime(path, ns=(info.st_atime_ns, info.st_mtime_ns))
        second = agent.input_description('inspect_data', args)
        self.assertNotEqual(first['sha256'], second['sha256'])

    def test_newer_durable_notes_survive_a_crash_before_host_checkpoint(self):
        agent = self.agent()
        self.read(agent)
        original = self.finding('layout', 'O1')
        agent.dispatch('record_findings', dict(findings=[original], next_steps=['Inspect the table.']))
        agent.checkpoint([])
        old_checkpoint = json.loads((agent.work / 'checkpoint.json').read_text('utf-8'))
        corrected = dict(original, summary='The table payload offset was corrected after checking the boundary.')
        agent.dispatch('record_findings', dict(findings=[corrected], next_steps=['Write and test the parser.']))
        # Simulate a crash after the tool persisted research, before checkpoint().
        restored = host.Agent(self.config)
        restored.audit_messages = list(agent.audit_messages)
        restored.restore_checkpoint(old_checkpoint)
        self.assertEqual(restored.analysis_notes['findings']['layout']['summary'], corrected['summary'])
        self.assertEqual(restored.analysis_notes['next_steps'], ['Write and test the parser.'])


if __name__ == '__main__':
    unittest.main()
