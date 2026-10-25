"""Offline regressions for avoiding duplicate work without reusing stale proof."""
import importlib.util
import json
import os
import pathlib
import tempfile
import unittest
from unittest import mock


spec = importlib.util.spec_from_file_location('reuse_host', pathlib.Path(__file__).with_name('agent_host.py'))
host = importlib.util.module_from_spec(spec)
spec.loader.exec_module(host)


class VerifiedReuseTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = pathlib.Path(temporary.name)
        (self.root / 'game.exe').write_bytes(b'fixture')
        self.scene = self.root / 'scene.txt'
        self.scene.write_bytes(b'Current')
        self.agent = host.Agent(dict(game=str(self.root), save=str(self.root), skill=str(self.root),
                                     work=str(self.root / 'work'), language='en-US'))
        self.agent.read_contract = self.agent.read_skill = True
        self.agent.programs['script'] = 'def extract(data, metadata): return []'
        self.script_args = dict(kind='script', root='game', path='scene.txt')

    def rows(self, source, data, metadata, **kwargs):
        return [dict(position=0, text=data.decode(), speaker=None,
                     evidence=dict(offset=0, length=len(data), encoding='utf-8'))]

    def test_identical_script_test_reuses_verified_rows_and_preserves_validated_saves(self):
        with mock.patch.object(host, 'run_parser', side_effect=self.rows) as parser:
            first = self.agent.dispatch('test_program', self.script_args)
            self.agent.tests['save'] = dict(matching_anchors=['Current'])
            second = self.agent.dispatch('test_program', self.script_args)
        self.assertEqual(parser.call_count, 1)
        self.assertEqual(first['preview'], second['preview'])
        self.assertTrue(second['reused'])
        self.assertIn('save', self.agent.tests)

    def test_selecting_another_script_does_not_repeat_an_already_verified_sample(self):
        with mock.patch.object(host, 'run_parser', side_effect=self.rows) as parser:
            self.agent.dispatch('test_program', self.script_args)
            self.agent.resource_selections['other.txt'] = dict(selected=str(self.root / 'other.arc'))
            self.agent.extracted_from[str(self.agent.work / 'other-unpacked')] = str(self.root / 'other.arc')
            result = self.agent.dispatch('test_program', self.script_args)
        self.assertTrue(result['reused'])
        self.assertEqual(parser.call_count, 1)

    def test_same_size_same_timestamp_input_change_invalidates_cached_test(self):
        with mock.patch.object(host, 'run_parser', side_effect=self.rows) as parser:
            self.agent.dispatch('test_program', self.script_args)
            original = self.scene.stat()
            self.scene.write_bytes(b'Changed')
            os.utime(self.scene, ns=(original.st_atime_ns, original.st_mtime_ns))
            self.agent.tests['save'] = dict(matching_anchors=['Current'])
            changed = self.agent.dispatch('test_program', self.script_args)
        self.assertEqual(parser.call_count, 2)
        self.assertEqual(changed['preview'][0]['text'], 'Changed')
        self.assertEqual(self.agent.tests, {})

    def test_changed_program_companion_or_accepted_rows_cannot_reuse_previous_proof(self):
        for change in ('program', 'metadata', 'rows', 'proof'):
            with self.subTest(change=change), mock.patch.object(host, 'run_parser', side_effect=self.rows) as parser:
                self.agent.program_test_cache.clear()
                self.agent.dispatch('test_program', self.script_args)
                if change == 'program':
                    self.agent.programs['script'] += '\n'
                elif change == 'metadata':
                    self.agent.script_resources['names'] = dict(seven='Speaker')
                elif change == 'proof':
                    self.agent.speaker_resource_inputs['names'] = dict(sha256='changed')
                    with self.assertRaisesRegex(ValueError, 'Companion speaker resource changed'):
                        self.agent.dispatch('test_program', self.script_args)
                    self.assertEqual(parser.call_count, 1)
                    continue
                else:
                    self.agent.parts[str(self.scene)][0]['text'] = 'Changed elsewhere'
                result = self.agent.dispatch('test_program', self.script_args)
                self.assertFalse(result.get('reused', False))
                self.assertEqual(parser.call_count, 2)

    def test_changed_companion_bytes_cannot_bypass_source_recheck_on_cache_hit(self):
        names = self.root / 'names.txt'
        names.write_bytes(b'Alice')
        self.agent.speaker_resource_inputs['names'] = self.agent.input_description(
            'load_script_metadata', dict(root='game', path='names.txt'))
        with mock.patch.object(host, 'run_parser', side_effect=self.rows) as parser:
            self.agent.dispatch('test_program', self.script_args)
            original = names.stat()
            names.write_bytes(b'Carol')
            os.utime(names, ns=(original.st_atime_ns, original.st_mtime_ns))
            with self.assertRaisesRegex(ValueError, 'Companion speaker resource changed'):
                self.agent.dispatch('test_program', self.script_args)
            self.assertEqual(parser.call_count, 1)

    def test_cleared_test_state_and_failed_tests_are_never_resurrected_from_cache(self):
        with mock.patch.object(host, 'run_parser', side_effect=self.rows) as parser:
            self.agent.dispatch('test_program', self.script_args)
            self.agent.parts.clear()
            self.agent.dispatch('test_program', self.script_args)
            self.assertEqual(parser.call_count, 2)
        self.agent.program_test_cache.clear()
        with mock.patch.object(host, 'run_parser', side_effect=ValueError('bad evidence')) as parser:
            for _ in range(2):
                with self.assertRaisesRegex(ValueError, 'bad evidence'):
                    self.agent.dispatch('test_program', self.script_args)
            self.assertEqual(parser.call_count, 2)

    def test_batch_skips_successful_sample_and_writes_complete_transcript_once(self):
        (self.root / 'second.txt').write_bytes(b'Next')
        with mock.patch.object(host, 'run_parser', side_effect=self.rows) as parser:
            self.agent.dispatch('test_program', self.script_args)
            with mock.patch.object(self.agent, 'write_dialogue', wraps=self.agent.write_dialogue) as write:
                result = self.agent.dispatch('test_program_batch', dict(root='game', path='.', filter='*.txt'))
                write.assert_called_once()
            self.assertEqual(parser.call_count, 2)
            self.assertEqual(sum(item['reused'] for item in result['files']), 1)
            self.agent.tests['save'] = dict(matching_anchors=['Current'])
            repeated = self.agent.dispatch('test_program_batch', dict(root='game', path='.', filter='*.txt'))
            self.assertTrue(repeated['reused'])
            self.assertEqual(parser.call_count, 2)
            self.assertIn('save', self.agent.tests)
        transcript = [json.loads(line) for line in (self.agent.work / 'dialogue.jsonl').read_text('utf-8').splitlines()]
        self.assertEqual([row['text'] for row in transcript], ['Current', 'Next'])

    def test_changed_or_missing_saved_test_artifact_requires_fresh_validation(self):
        with mock.patch.object(host, 'run_parser', side_effect=self.rows) as parser:
            first = self.agent.dispatch('test_program', self.script_args)
            (self.agent.work / first['full_output']).write_text('null', encoding='utf-8')
            self.agent.dispatch('test_program', self.script_args)
            self.assertEqual(parser.call_count, 2)

    def test_save_test_reuse_depends_on_current_transcript_and_accepted_sample_state(self):
        self.agent.parts[str(self.scene)] = [dict(source='scene.txt', position=0, text='Current', speaker=None)]
        self.agent.programs['save'] = 'def extract(data, metadata): return {}'
        slot = self.root / 'slot.sav'
        slot.write_bytes(b'Current')
        args = dict(kind='save', root='save', path=slot.name)
        facts = dict(source='scene.txt', line=0, anchors=['Current'])
        with mock.patch.object(host, 'run_parser', return_value=facts) as parser:
            self.agent.dispatch('test_program', args)
            self.assertTrue(self.agent.dispatch('test_program', args)['reused'])
            self.assertEqual(parser.call_count, 1)
            self.agent.parts[str(self.scene)][0]['speaker'] = 'Verified speaker'
            self.assertFalse(self.agent.dispatch('test_program', args).get('reused', False))
            self.assertEqual(parser.call_count, 2)
            self.agent.tests.clear()
            self.agent.dispatch('test_program', args)
            self.assertEqual(parser.call_count, 3)

    def test_archive_pagination_reuses_one_process_and_changes_invalidate_it(self):
        output = 'x' * 5000
        completed = mock.Mock(returncode=0, stdout=output.encode(), stderr=b'')
        with mock.patch.object(host.subprocess, 'run', return_value=completed) as process:
            first = self.agent.dispatch('run_skill', dict(command='l', archive='scene.txt'))
            second = self.agent.dispatch('run_skill', dict(command='l', archive='scene.txt', offset=first['next_offset']))
            self.assertEqual(first['stdout'] + second['stdout'], output[:4096])
            self.assertTrue(second['reused'])
            self.assertEqual(process.call_count, 1)
            original = self.scene.stat()
            self.scene.write_bytes(b'Changed')
            os.utime(self.scene, ns=(original.st_atime_ns, original.st_mtime_ns))
            self.agent.dispatch('run_skill', dict(command='l', archive='scene.txt'))
            self.assertEqual(process.call_count, 2)

    def test_default_document_page_delivers_up_to_eight_kilobytes_of_characters(self):
        (self.root / 'guide.md').write_text('a' * 10000, encoding='utf-8')
        page = self.agent.dispatch('read_text', dict(root='skill', path='guide.md'))
        self.assertEqual(len(page['text']), 8192)
        self.assertEqual(page['next_offset'], 8192)
        small = self.agent.dispatch('read_text', dict(root='skill', path='guide.md', length=100))
        self.assertEqual(len(small['text']), 100)

    def test_extraction_cache_rechecks_the_produced_member_bytes(self):
        def extract(command, **kwargs):
            directory = pathlib.Path(command[command.index('-o') + 1])
            (directory / 'scene.txt').write_bytes(b'Current')
            return mock.Mock(returncode=0, stdout=b'extracted', stderr=b'')
        args = dict(command='x', archive='scene.txt', filter='scene.txt')
        with mock.patch.object(host.subprocess, 'run', side_effect=extract) as process:
            first = self.agent.dispatch('run_skill', args)
            self.assertTrue(self.agent.dispatch('run_skill', args)['reused'])
            self.assertEqual(process.call_count, 1)
            member = self.agent.work / first['output'] / 'scene.txt'
            member.write_bytes(b'Changed')
            self.assertFalse(self.agent.dispatch('run_skill', args).get('reused', False))
            self.assertEqual(process.call_count, 2)
            self.assertEqual(member.read_bytes(), b'Current')

    def prepare_archived_script(self):
        archive = self.root / 'script.arc'
        archive.write_bytes(b'original archive')
        configuration = self.root / 'game.ini'
        configuration.write_bytes(b'archive=script.arc')
        self.agent.resource_selections['scene.txt'] = dict(selected=str(archive), configuration='game.ini',
            input_stamps={str(archive): self.agent.file_fingerprint(archive),
                          str(configuration): self.agent.file_fingerprint(configuration)})

        def extract(command, **kwargs):
            self.assertEqual(pathlib.Path(command[-1]), archive)
            if command[1] == 'l':
                listing = '\n'.join('[{:08X}] {} {}'.format(index * 32, len(data), name)
                                    for index, (name, data) in enumerate(extract.members.items()))
                return mock.Mock(returncode=0, stdout=listing.encode(), stderr=b'')
            self.assertEqual(command[1], 'x')
            directory = pathlib.Path(command[command.index('-o') + 1])
            for name, data in extract.members.items():
                (directory / name).write_bytes(data)
            return mock.Mock(returncode=0, stdout=b'extracted', stderr=b'')

        extract.members = {'scene.txt': b'Current'}
        # Keep the actual run_skill listing/cache/provenance path active while
        # replacing the fixture's absent GARbro executable with its CLI output.
        fixture_tools = mock.patch.object(host.subprocess, 'run', side_effect=extract)
        fixture_tools.start()
        self.addCleanup(fixture_tools.stop)
        extraction_args = dict(command='x', archive=archive.name, filter='scene.txt')
        result = self.agent.dispatch('run_skill', extraction_args)
        member = self.agent.work / result['output'] / 'scene.txt'
        test_args = dict(kind='script', root='work', path=str(member.relative_to(self.agent.work)))
        return archive, configuration, member, extraction_args, test_args, extract

    def test_changed_origin_archive_rejects_cached_or_bare_test_of_old_unpacked_member(self):
        archive, configuration, member, extraction_args, test_args, extract = self.prepare_archived_script()
        with mock.patch.object(host, 'run_parser', side_effect=self.rows) as parser:
            self.agent.dispatch('test_program', test_args)
            original = archive.stat()
            archive.write_bytes(b'modified archive')
            os.utime(archive, ns=(original.st_atime_ns, original.st_mtime_ns))
            for _ in range(2):
                with self.assertRaisesRegex(ValueError, 'Source archive changed'):
                    self.agent.dispatch('test_program', test_args)
            self.assertEqual(parser.call_count, 1)
            self.assertEqual(member.read_bytes(), b'Current')
            self.agent.tests['old-save'] = dict(matching_anchors=['Current'])
            self.assertEqual(self.agent.preparation_status()['dialogue_count'], 0)
            self.assertEqual(self.agent.tests, {})
            self.assertIn(str(member), self.agent.preparation_status()['pending_script_inputs'])
            selection = self.agent.resource_selections['scene.txt']
            selection['input_stamps'][str(archive)] = self.agent.file_fingerprint(archive)
            with mock.patch.object(host.subprocess, 'run', side_effect=extract):
                refreshed = self.agent.dispatch('run_skill', extraction_args)
            self.assertNotEqual(refreshed['output'], str(member.parent.relative_to(self.agent.work)))
            with self.assertRaisesRegex(ValueError, 'Source archive changed'):
                self.agent.dispatch('test_program', test_args)
            test_args['path'] = str(pathlib.Path(refreshed['output']) / 'scene.txt')
            self.agent.dispatch('test_program', test_args)
            self.assertEqual(parser.call_count, 2)
            self.assertEqual(self.agent.preparation_status()['pending_script_inputs'], [])

    def test_finish_without_retest_rejects_changed_resource_configuration_and_checkpoint(self):
        archive, configuration, member, extraction_args, test_args, extract = self.prepare_archived_script()
        with mock.patch.object(host, 'run_parser', side_effect=self.rows):
            self.agent.dispatch('test_program', test_args)
        self.agent.checkpoint([])
        saved = json.loads((self.agent.work / 'checkpoint.json').read_text('utf-8'))
        self.assertIn(str(member), saved['script_dependency_stamps'])
        self.assertTrue(saved['extraction_stamps'])
        original = configuration.stat()
        configuration.write_bytes(b'archive=changed.ar')
        os.utime(configuration, ns=(original.st_atime_ns, original.st_mtime_ns))
        self.agent.tests['old-save'] = dict(matching_anchors=['Current'])
        with self.assertRaisesRegex(ValueError, 'Read/use the skill'):
            self.agent.dispatch('finish', {})
        self.assertEqual(self.agent.dialogue(), [])
        self.assertEqual(self.agent.tests, {})
        with self.assertRaisesRegex(ValueError, 'resource configuration changed'):
            self.agent.dispatch('test_program', test_args)
        restored = host.Agent(self.agent.config)
        restored.programs = dict(self.agent.programs)
        self.assertFalse(restored.restore_checkpoint(saved))
        self.assertEqual(restored.dialogue(), [])

    def test_new_extraction_version_clears_only_the_replaced_pending_member(self):
        archive, configuration, member, extraction_args, test_args, extract = self.prepare_archived_script()
        self.agent.resource_selections['other.txt'] = dict(self.agent.resource_selections['scene.txt'])
        def extract_both(command, **kwargs):
            extract.members['other.txt'] = b'Next'
            result = extract(command, **kwargs)
            directory = pathlib.Path(command[command.index('-o') + 1])
            (directory / 'other.txt').write_bytes(b'Next')
            return result
        extraction_args['filter'] = r'.*\.txt$'
        with mock.patch.object(host.subprocess, 'run', side_effect=extract_both):
            old = self.agent.dispatch('run_skill', extraction_args)
        other_args = dict(kind='script', root='work', path=str(pathlib.Path(old['output']) / 'other.txt'))
        with mock.patch.object(host, 'run_parser', side_effect=self.rows):
            self.agent.dispatch('test_program', test_args)
            self.agent.dispatch('test_program', other_args)
            archive.write_bytes(b'modified archive')
            self.assertEqual(len(self.agent.preparation_status()['pending_script_inputs']), 2)
            for selection in self.agent.resource_selections.values():
                selection['input_stamps'][str(archive)] = self.agent.file_fingerprint(archive)
            with mock.patch.object(host.subprocess, 'run', side_effect=extract_both):
                refreshed = self.agent.dispatch('run_skill', extraction_args)
            test_args['path'] = str(pathlib.Path(refreshed['output']) / 'scene.txt')
            self.agent.dispatch('test_program', test_args)
            pending = self.agent.preparation_status()['pending_script_inputs']
            self.assertEqual(pending, [str(self.agent.work / other_args['path'])])
            other_args['path'] = str(pathlib.Path(refreshed['output']) / 'other.txt')
            self.agent.dispatch('test_program', other_args)
            self.assertEqual(self.agent.preparation_status()['pending_script_inputs'], [])

    def test_companion_origin_archive_change_invalidates_script_proof_and_readiness(self):
        archive, configuration, member, extraction_args, test_args, extract = self.prepare_archived_script()
        names_archive = self.root / 'names.arc'
        names_archive.write_bytes(b'original names')
        directory = self.agent.work / 'names-extracted'
        directory.mkdir()
        names = directory / 'names.txt'
        names.write_bytes(b'Alice')
        self.agent.extracted_from[str(directory)] = str(names_archive)
        self.agent.extraction_stamps[str(directory)] = self.agent.file_fingerprint(names_archive)
        self.agent.speaker_resource_inputs['names'] = self.agent.input_description(
            'load_script_metadata', dict(root='work', path=str(names.relative_to(self.agent.work))))
        with mock.patch.object(host, 'run_parser', side_effect=self.rows) as parser:
            self.agent.dispatch('test_program', test_args)
            original = names_archive.stat()
            names_archive.write_bytes(b'modified names')
            os.utime(names_archive, ns=(original.st_atime_ns, original.st_mtime_ns))
            with self.assertRaisesRegex(ValueError, 'Source archive changed'):
                self.agent.dispatch('test_program', test_args)
            self.assertEqual(parser.call_count, 1)
        self.assertEqual(self.agent.preparation_status()['dialogue_count'], 0)
        self.assertIn(str(member), self.agent.preparation_status()['pending_script_inputs'])

    def test_source_invalidation_preserves_only_the_unchanged_byte_contract_prerequisite(self):
        with mock.patch.object(host, 'run_parser', side_effect=self.rows):
            self.agent.dispatch('test_program', self.script_args)
        self.agent.checkpoint([])
        checkpoint = json.loads((self.agent.work / 'checkpoint.json').read_text('utf-8'))
        self.scene.write_bytes(b'Changed')
        restored = host.Agent(self.agent.config)
        restored.programs = dict(self.agent.programs)
        restored.pending_script_inputs.add('old member')
        restored.extraction_stamps['old target'] = dict(size=1)
        self.assertFalse(restored.restore_checkpoint(json.loads(json.dumps(checkpoint))))
        self.assertTrue(restored.read_contract)
        self.assertEqual(restored.pending_script_inputs, set())
        self.assertEqual(restored.script_dependency_stamps, {})
        self.assertEqual(restored.extraction_stamps, {})
        changed_contract = json.loads(json.dumps(checkpoint))
        changed_contract['contract_version'] = host.CONTRACT_VERSION - 1
        restored.restore_checkpoint(changed_contract)
        self.assertFalse(restored.read_contract)
        reference = self.root / 'reference'
        reference.mkdir()
        (reference / 'format-index.json').write_text('[]', encoding='utf-8')
        restored.restore_checkpoint(json.loads(json.dumps(checkpoint)))
        self.assertFalse(restored.read_contract)


if __name__ == '__main__':
    unittest.main()
