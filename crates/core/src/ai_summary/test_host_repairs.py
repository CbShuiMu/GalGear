"""Toy host integrations for atomic parser edits and actionable test failures."""
import copy
import hashlib
import importlib.util
import json
import pathlib
import struct
import tempfile
import unittest
from unittest import mock


spec = importlib.util.spec_from_file_location(
    'repair_speaker_fixtures', pathlib.Path(__file__).with_name('test_speaker_scopes.py'))
fixtures = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fixtures)
host, PARSER, HELPER = fixtures.host, fixtures.PARSER, fixtures.HELPER


class HostRepairTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = pathlib.Path(temporary.name)
        (self.root / 'game.exe').write_bytes(b'fixture')
        skill = self.root / 'skill'
        skill.mkdir()
        self.config = dict(game=str(self.root), save=str(self.root), skill=str(skill),
                           work=str(self.root / 'work'), title='Repair fixture', language='en-US',
                           provider='deepseek', max_turns=0)
        self.agent = host.Agent(self.config)
        self.agent.read_contract = self.agent.read_skill = True
        self.agent.dispatch('write_program', dict(kind='script', source=PARSER))
        for name in ('first', 'second'):
            text = ('Message ' + name).encode('ascii')
            (self.root / (name + '.lng')).write_bytes(struct.pack('<II', 1, len(text)) + text)
            (self.root / (name + '.ws2')).write_bytes(fixtures.companion_bytes(0, 'Name'))

    def run_script(self, name='first', agent=None):
        return (agent or self.agent).dispatch('test_program', dict(kind='script', root='game', path=name + '.lng'))

    def snapshot(self):
        return dict(source=self.agent.programs['script'],
                    file=(self.agent.work / 'script_extractor.py').read_bytes(),
                    parts=copy.deepcopy(self.agent.parts),
                    tests=copy.deepcopy(self.agent.tests),
                    dialogue=(self.agent.work / 'dialogue.jsonl').read_bytes(),
                    history={path.name: path.read_bytes() for path in (self.agent.work / 'program-history').glob('*.py')})

    def checkpoint(self):
        messages = [dict(role='system', content='Fixture system'),
                    dict(role='user', content=json.dumps(dict(byte_evidence_contract=host.BYTE_EVIDENCE_CONTRACT)))]
        self.agent.checkpoint(messages)
        return json.loads((self.agent.work / 'checkpoint.json').read_text('utf-8'))

    def resume_without_model(self):
        # Exercise production run_session's parser-file loading, then stop at
        # its zero request budget. No generated test response or model is used.
        restored = host.Agent(self.config)
        with mock.patch.object(restored, 'request_completion', side_effect=AssertionError('Unexpected model request')) as request:
            with self.assertRaisesRegex(RuntimeError, 'budget exhausted'):
                restored.run()
        request.assert_not_called()
        self.assertEqual(restored.requests_sent, 0)
        return restored

    def assert_failure(self, category, source, path='first.lng', agent=None):
        failure = (agent or self.agent).preparation_status()['last_script_failure']
        self.assertEqual(failure['kind'], 'script')
        self.assertEqual(failure['category'], category)
        self.assertEqual(failure['root'], 'game')
        self.assertEqual(failure['path'], path)
        self.assertEqual(failure['source_sha256'], hashlib.sha256(source.encode('utf-8')).hexdigest())
        self.assertTrue(failure['error'])
        return failure

    def test_unique_exact_edit_preserves_other_source_and_requires_real_revalidation(self):
        self.assertEqual(self.run_script()['count'], 1)
        accepted = copy.deepcopy(self.agent.parts)
        old, new = '    rows = []\n', '    rows = []  # local repair\n'
        self.assertEqual(PARSER.count(old), 1)
        result = self.agent.dispatch('write_program', dict(kind='script', edits=[dict(old=old, new=new)]))
        expected = PARSER.replace(old, new, 1)
        self.assertEqual(self.agent.programs['script'], expected)
        self.assertEqual((self.agent.work / result['saved']).read_bytes(), expected.encode('utf-8'))
        self.assertEqual(result['sha256'], hashlib.sha256(expected.encode('utf-8')).hexdigest())
        self.assertEqual(result['invalidated_dialogue_rows'], 1)
        self.assertEqual(result['requires_test'], 'script')
        self.assertEqual(self.agent.parts, {})
        self.assertEqual(self.run_script()['count'], 1)
        self.assertEqual(self.agent.parts, accepted)

    def test_missing_or_ambiguous_exact_span_never_changes_the_validated_program(self):
        self.run_script()
        before = self.snapshot()
        for old in ('not present in this source', 'data'):
            with self.subTest(old=old), self.assertRaisesRegex(ValueError, 'must match exactly once'):
                self.agent.dispatch('write_program', dict(kind='script', edits=[dict(old=old, new='replacement')]))
            self.assertEqual(self.snapshot(), before)

    def test_later_failed_edit_rolls_back_all_prior_edits_and_accepted_rows(self):
        self.run_script()
        before = self.snapshot()
        edits = [dict(old='    rows = []\n', new='    rows = []  # would be valid alone\n'),
                 dict(old='this second span does not exist', new='replacement')]
        with self.assertRaisesRegex(ValueError, r'edits\[1\]'):
            self.agent.dispatch('write_program', dict(kind='script', edits=edits))
        self.assertEqual(self.snapshot(), before)

    def test_invalid_syntax_and_forbidden_ast_do_not_reach_program_or_history_files(self):
        self.run_script()
        before = self.snapshot()
        requests = [dict(kind='script', source='def extract(data, metadata):\n    return (\n'),
                    dict(kind='script', edits=[dict(old='    rows = []\n', new='    rows = [\n')]),
                    dict(kind='script', source='import os\ndef extract(data, metadata): return []\n')]
        for args in requests:
            with self.subTest(arguments=set(args)), self.assertRaises((SyntaxError, ValueError)):
                self.agent.dispatch('write_program', args)
            self.assertEqual(self.snapshot(), before)

    def test_crlf_unicode_source_is_stored_and_resumed_byte_for_byte(self):
        source = '# 中文局部修复与恢复 🌙\r\n' + PARSER.replace('\n', '\r\n')
        result = self.agent.dispatch('write_program', dict(kind='script', source=source))
        encoded = source.encode('utf-8')
        path = self.agent.work / 'script_extractor.py'
        history = self.agent.work / 'program-history' / (result['sha256'] + '.py')
        self.assertEqual(path.read_bytes(), encoded)
        self.assertEqual(history.read_bytes(), encoded)
        self.assertEqual(self.agent.programs['script'], source)
        self.assertTrue(self.agent.dispatch('write_program', dict(kind='script', source=source))['reused'])
        self.run_script()
        accepted = copy.deepcopy(self.agent.parts)
        self.checkpoint()
        restored = self.resume_without_model()
        self.assertEqual(restored.programs['script'], source)
        self.assertEqual(path.read_bytes(), encoded)
        self.assertEqual(restored.parts, accepted)
        edited = restored.dispatch('write_program', dict(kind='script', edits=[
            dict(old='# 中文局部修复与恢复 🌙\r\n', new='# Unicode 恢复后的精确修改 🌙\r\n')]))
        expected = source.replace('# 中文局部修复与恢复 🌙\r\n', '# Unicode 恢复后的精确修改 🌙\r\n', 1)
        self.assertEqual(path.read_bytes(), expected.encode('utf-8'))
        self.assertEqual(edited['requires_test'], 'script')

    def test_actual_schema_failure_survives_checkpoint_until_success_on_the_same_target(self):
        broken = PARSER.replace("'text':text,", "'text':text + ' forged',")
        self.agent.dispatch('write_program', dict(kind='script', source=broken))
        with self.assertRaises(ValueError) as caught:
            self.run_script()
        failure = copy.deepcopy(self.assert_failure('script', broken))
        self.assertEqual(failure['error'], str(caught.exception)[:5000])
        saved = self.checkpoint()
        self.assertEqual(saved['last_script_failure'], failure)
        restored = self.resume_without_model()
        self.assertEqual(self.assert_failure('script', broken, agent=restored), failure)
        restored.dispatch('write_program', dict(kind='script', source=PARSER))
        self.assertEqual(restored.last_script_failure, failure)
        self.assertEqual(self.run_script('second', agent=restored)['count'], 1)
        self.assertEqual(restored.last_script_failure, failure)
        self.assertEqual(self.run_script(agent=restored)['count'], 1)
        self.assertIsNone(restored.preparation_status()['last_script_failure'])

    def test_stale_companion_failure_retains_the_speaker_metadata_category(self):
        self.agent.dispatch('load_script_metadata', dict(root='game', path='first.ws2', key='own-companion',
            source=HELPER, target_script=dict(root='game', path='first.lng')))
        (self.root / 'first.ws2').write_bytes(fixtures.companion_bytes(0, 'Changed'))
        with self.assertRaisesRegex(ValueError, 'Companion speaker resource changed'):
            self.run_script()
        failure = self.assert_failure('speaker_metadata', PARSER)
        self.assertEqual(failure['stale_metadata_keys'], ['own-companion'])

    def test_stale_archive_failure_retains_the_resource_category(self):
        archive = self.root / 'chapters.arc'
        archive.write_bytes(b'Original archive fixture')
        target = str(self.root)
        self.agent.extracted_from[target] = str(archive)
        self.agent.extraction_stamps[target] = self.agent.file_fingerprint(archive)
        archive.write_bytes(b'Archive contents changed after extraction')
        with self.assertRaisesRegex(ValueError, 'Source archive changed'):
            self.run_script()
        self.assert_failure('resource', PARSER)


if __name__ == '__main__':
    unittest.main()
