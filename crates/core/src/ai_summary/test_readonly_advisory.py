"""Synthetic integration of advisory investigation and independent proof gates.

All model responses and CLI output are local fixtures. No game data, credentials,
network request, unsupported role upgrade or global-family claim is involved.
"""
import io
import json
import pathlib
import tempfile
import unittest
from unittest import mock

import test_agent_host as fixtures
import test_archive_evidence as archives


host = fixtures.host


class ReadOnlyAdvisoryIntegrationTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = pathlib.Path(temporary.name)
        self.game, self.saves, self.skill = (self.root / name for name in ('game', 'saves', 'skill'))
        for path in (self.game, self.saves, self.skill):
            path.mkdir()
        (self.game / 'game.exe').write_bytes(b'Independent fixture executable')
        (self.game / 'chapter.txt').write_bytes(b'Hello')
        (self.game / 'tree.dat').write_bytes(b'(const) %["value" => "Independent tree"]')
        self.save = self.saves / 'current.sav'
        self.save.write_bytes(b'Unclassified synthetic native state')
        self.agent = host.Agent(dict(game=str(self.game), save=str(self.saves), skill=str(self.skill),
            work=str(self.game / '.galgear-ai-summary' / 'fresh'), cold_start=True,
            title='Independent fixture', language='en-US', provider='deepseek', model='offline-fixture',
            max_turns=80, required_saves=[str(self.save)],
            model_capabilities=dict(status='unavailable', kind='offline_fixture')))
        self.agent.read_skill = self.agent.read_contract = self.agent.used_skill = True
        source = 'def extract(data, metadata): return []'
        self.agent.dispatch('write_program', dict(kind='script', source=source))
        self.agent.work_guidance()
        self.agent.workflow.inspection_turns = set(range(-8, 0))
        self.agent.workflow.pending_validation = {'script'}
        self.messages = [dict(role='system', content='Offline synthetic protocol'),
                         dict(role='user', content='Exercise only local tools')]
        self.call_number = 0
        self.no_generation = mock.patch.dict(host.TRANSPORT, {
            'complete': mock.Mock(side_effect=AssertionError('No real completion is permitted')),
            'read_capabilities': mock.Mock(side_effect=AssertionError('No metadata request is permitted'))})
        self.no_generation.start()
        self.addCleanup(self.no_generation.stop)
        silence = mock.patch.object(host.sys, 'stderr', new=io.StringIO())
        silence.start()
        self.addCleanup(silence.stop)

    def model_call(self, name, arguments):
        """Run the actual host tool-call pipeline with a fixed offline response."""
        self.call_number += 1
        call_id = 'offline-' + str(self.call_number)
        message = dict(role='assistant', content=None, tool_calls=[dict(id=call_id, type='function',
            function=dict(name=name, arguments=json.dumps(arguments)))])
        with mock.patch.object(self.agent, 'request_completion', return_value=message):
            self.agent.run_turn(self.messages, host.TOOLS, self.call_number, 'prepare')
        return json.loads(next(item['content'] for item in reversed(self.messages)
                               if item.get('role') == 'tool' and item.get('tool_call_id') == call_id))

    def test_same_run_result_remains_readable_and_rereads_are_advisory_during_repair(self):
        result = self.model_call('inspect_data', dict(root='game', path='chapter.txt',
            source='def extract(data, metadata): return {"size":len(data)}'))
        self.assertNotIn('error', result)
        record = next(path for path in (self.agent.work / 'tool-results').glob('*.json')
                      if json.loads(path.read_text('utf-8')).get('function', {}).get('name') == 'inspect_data')
        args = dict(root='work', path=record.relative_to(self.agent.work).as_posix(), pointer='/result')
        for _ in range(5):
            recovered = self.model_call('read_json', args)
            self.assertNotIn('error', recovered)
            self.assertEqual(recovered['value']['size'], 5)
        guidance = self.agent.work_guidance()
        self.assertGreaterEqual(guidance['exploratory_requests'], 14)
        self.assertGreaterEqual(guidance['repeated_read_requests'], 4)
        self.assertIn('Repeated read requests', guidance['read_only_research_advisory'])
        self.assertEqual(self.agent.workflow.pending_validation, {'script'})
        self.assertFalse(self.agent.preparation_status()['ready_for_finish'])
        self.assertIsNone(self.agent.workflow.verified_at)
        self.assertFalse(self.agent.extended)

    def test_complete_structured_observation_is_available_without_granting_a_node(self):
        result = self.model_call('inspect_structured', dict(root='game', path='tree.dat',
            document=dict(syntax='tjs_const', encoding='ascii', view=[]), field_path=['value']))
        self.assertNotIn('error', result)
        self.assertFalse(result['host_verified_current_node'])
        self.assertEqual(self.agent.preparation_status()['dialogue_count'], 0)
        self.assertEqual(self.agent.preparation_status()['validated_saves'], 0)

    def test_advisory_reads_still_reject_traversal_absolute_paths_and_other_cold_cache(self):
        previous = self.game / '.galgear-ai-summary' / 'old-run'
        previous.mkdir()
        (previous / 'note.txt').write_text('Untrusted prior fixture notes', encoding='utf-8')
        attempts = [('../outside.txt', 'Path outside allowed root'),
                    (str(self.game / 'chapter.txt'), 'relative path'),
                    ('.galgear-ai-summary/old-run/note.txt', 'cannot read another preparation cache')]
        for path, expected in attempts:
            with self.subTest(path=path):
                result = self.model_call('read_text', dict(root='game', path=path))
                self.assertIn(expected, result['error'])

    def test_inspection_sandbox_and_input_size_limit_remain_hard(self):
        result = self.model_call('inspect_data', dict(root='game', path='chapter.txt',
            source='import os\ndef extract(data, metadata): return {}'))
        self.assertIn('error', result)
        large = self.game / 'large.bin'
        with large.open('wb') as handle:
            handle.seek(32 * 1024 * 1024)
            handle.write(b'X')
        result = self.model_call('inspect_data', dict(root='game', path='large.bin',
            source='def extract(data, metadata): return {}'))
        self.assertIn('Input exceeds 32MB', result['error'])

    def test_only_exact_cli_info_list_commands_gain_advisory_scheduling(self):
        archive = self.game / 'resources.xp3'
        archive.write_bytes(archives.archive_bytes(b'Hello', name='chapter.scn'))
        cli = mock.Mock(return_value=mock.Mock(returncode=0, stdout=b'[00000000] 5 chapter.scn\n', stderr=b''))
        with mock.patch.object(host.subprocess, 'run', cli):
            for command in ('i', 'l'):
                result = self.model_call('run_skill', dict(command=command, archive=archive.name))
                self.assertNotIn('error', result)
        self.assertEqual(cli.call_count, 2)
        for call in cli.call_args_list:
            self.assertEqual(call.kwargs['timeout'], 120)
        # Listing cannot turn missing save preflight into an extraction grant.
        result = self.model_call('run_skill', dict(command='x', archive=archive.name, filter='*.scn'))
        self.assertIn('error', result)
        self.assertFalse(host.WORKFLOW['read_only_research']('run_skill', dict(command='x')))
        self.assertFalse(host.WORKFLOW['read_only_research']('run_skill', dict(command='unknown')))
        with self.assertRaisesRegex(ValueError, 'Parser changed'):
            self.agent.workflow.check('run_skill', dict(command='x', archive=archive.name))
        with self.assertRaisesRegex(ValueError, 'Parser changed'):
            self.agent.workflow.check('write_program', dict(kind='script'))

    def archived_member(self):
        archive = self.game / 'resources.xp3'
        archive.write_bytes(archives.archive_bytes(b'Hello', name='chapter.scn'))
        directory = self.agent.work / 'unpacked' / 'independent-fixture'
        directory.mkdir(parents=True)
        target = directory / 'chapter.scn'
        target.write_bytes(b'Hello')
        self.agent.extracted_from[str(directory)] = str(archive)
        self.agent.extraction_stamps[str(directory)] = self.agent.file_fingerprint(archive)
        self.agent.config['inventory'] = [dict(path=str(archive))]
        return archive, target

    def test_archive_observation_does_not_replace_selection_or_native_checksum_proof(self):
        archive, target = self.archived_member()
        result = self.model_call('inspect_archive_member', dict(root='work',
            path=target.relative_to(self.agent.work).as_posix(), archive=archive.name))
        self.assertNotIn('error', result)
        self.assertTrue(result['binding']['original_member_bound'])
        self.assertFalse(result['restored_member_verified'])
        self.assertFalse(result['host_verified_current_node'])
        test = dict(kind='script', root='work', path=target.relative_to(self.agent.work).as_posix())
        self.agent.dispatch('write_program', dict(kind='script', source='def extract(data, metadata): return {"rows":[]}'))
        with self.assertRaisesRegex(ValueError, 'effective archive/patch selection'):
            self.agent.dispatch('test_program', test)
        cli = mock.Mock(return_value=mock.Mock(returncode=0, stdout=b'[00000000] 5 chapter.scn\n', stderr=b''))
        with mock.patch.object(host.subprocess, 'run', cli):
            self.agent.dispatch('verify_resource_selection', dict(script_name=target.name, selected=archive.name))
        bad = 'def extract(data, metadata): return {"member_filter":[{"op":"xor","key":"01"}],"rows":[]}'
        self.agent.dispatch('write_program', dict(kind='script', source=bad))
        with self.assertRaisesRegex(ValueError, 'native size and Adler-32'):
            self.agent.dispatch('test_program', test)
        self.assertEqual(self.agent.preparation_status()['dialogue_count'], 0)

    def test_research_cannot_substitute_unproved_speaker_or_model_claimed_resume_role(self):
        speaker = ('def extract(data, metadata): return [{"position":0,"text":"Hello","speaker":"Alice",'
                   '"evidence":{"offset":0,"length":5,"encoding":"ascii"}}]')
        self.agent.dispatch('write_program', dict(kind='script', source=speaker))
        with self.assertRaises(ValueError):
            self.agent.dispatch('test_program', dict(kind='script', root='game', path='chapter.txt'))
        fake = ('def extract(data, metadata): return {"anchors":[],"resume":{"profile":"kag_parser_label_v1",'
                '"state_role":"resume_node","source":"chapter.txt","label":"*entry","position":0}}')
        self.agent.dispatch('write_program', dict(kind='save', source=fake))
        with self.assertRaises(ValueError):
            self.agent.dispatch('test_program', dict(kind='save', root='save', path=self.save.name))
        self.assertEqual(self.agent.preparation_status()['validated_saves'], 0)
        self.assertFalse(self.agent.preparation_status()['ready_for_finish'])
        with self.assertRaises(ValueError):
            self.agent.check_model_tool('finish', {})
        with self.assertRaisesRegex(ValueError, 'before finishing'):
            self.agent.dispatch('finish', dict(engine='Synthetic', script_language='en-US',
                speaker_resolution='unavailable', notes='Inspection alone is insufficient'))


if __name__ == '__main__':
    unittest.main()
