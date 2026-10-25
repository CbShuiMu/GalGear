"""Offline host integration tests for archive members and source selection.

The fake GARbro process only lists or writes generic fixture files. It does not
decode any game format, invoke a model, or use the user's game/work directory.
"""
import importlib.util
import json
import pathlib
import re
import tempfile
import unittest
from unittest import mock


spec = importlib.util.spec_from_file_location('resource_discovery_host', pathlib.Path(__file__).with_name('agent_host.py'))
host = importlib.util.module_from_spec(spec)
spec.loader.exec_module(host)


class ResourceDiscoveryTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = pathlib.Path(temporary.name)
        (self.root / 'game.exe').write_bytes(b'generic fixture')
        self.archive = self.root / 'data.arc'
        self.archive.write_bytes(b'opaque generic archive fixture')
        self.config = dict(game=str(self.root), save=str(self.root), skill=str(self.root),
                           work=str(self.root / 'work'), language='en-US',
                           inventory=[dict(path=str(self.archive))])
        self.agent = host.Agent(self.config)
        self.agent.read_skill = self.agent.read_contract = True
        self.members = {'scripts/scene.bin': b'Current'}
        self.listing = ''

    @staticmethod
    def normalized(names):
        return [name.replace('\\', '/') for name in names]

    def garbro(self, command, **kwargs):
        """Replace only the absent process, retaining the real host boundary."""
        self.assertEqual(pathlib.Path(command[-1]), self.archive)
        if command[1] == 'l':
            rows = ['GARbro generic archive listing']
            for index, (name, data) in enumerate(self.members.items()):
                rows.append('[{:08X}] {:10d} {}'.format(index * 32, len(data), name.replace('/', '\\')))
            rows.append('End of listing: {} members'.format(len(self.members)))
            self.listing = '\n'.join(rows)
            return mock.Mock(returncode=0, stdout=self.listing.encode('utf-8'), stderr=b'')
        self.assertEqual(command[1], 'x')
        directory = pathlib.Path(command[command.index('-o') + 1])
        for name, data in self.members.items():
            target = directory / name
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(data)
        # This deliberately names no real member: extraction discovery must use
        # the actual output files rather than parse progress/status messages.
        stdout = 'Extracted successfully; progress-message-decoy.bin is not a member.\n' * 3
        return mock.Mock(returncode=0, stdout=stdout.encode('utf-8'), stderr=b'')

    def assert_inventory(self, result, *, extraction):
        self.assertEqual(result['member_count'], len(self.members))
        self.assertEqual(len(result['members']), min(30, len(self.members)))
        path = self.agent.path('work', result['member_inventory'])
        self.assertNotEqual(result['member_inventory'], result['full_output'])
        inventory = json.loads(path.read_text('utf-8'))
        self.assertEqual(self.normalized([inventory['archive']]), ['data.arc'])
        self.assertCountEqual(self.normalized(inventory['members']), list(self.members))
        self.assertEqual(self.normalized(result['members']), self.normalized(inventory['members'][:30]))
        if extraction:
            self.assertEqual(self.normalized([inventory['output']]), self.normalized([result['output']]))
            sample = self.agent.path('work', result['sample_output'])
            output = self.agent.path('work', result['output'])
            member = sample.relative_to(output).as_posix()
            self.assertIn(member, self.members)
            self.assertEqual(sample.read_bytes(), self.members[member])
        else:
            self.assertIsNone(inventory['output'])
            self.assertIsNone(result['sample_output'])
        return inventory

    def test_listing_members_use_complete_stdout_and_survive_pagination_and_cache(self):
        self.members = {f'scripts/chapter-{index:03d}-with-a-long-filename.bin': b'fixture'
                        for index in range(63)}
        with mock.patch.object(host.subprocess, 'run', side_effect=self.garbro) as process:
            first = self.agent.dispatch('run_skill', dict(command='l', archive='data.arc', length=60))
            self.assertEqual(first['stdout'], self.listing[:60])
            self.assertEqual(first['next_offset'], 60)
            inventory = self.assert_inventory(first, extraction=False)
            second = self.agent.dispatch('run_skill', dict(command='l', archive='data.arc', offset=60, length=80))
            self.assertEqual(second['stdout'], self.listing[60:140])
            self.assertTrue(second['reused'])
            self.assert_inventory(second, extraction=False)
            for key in ('members', 'member_count', 'member_inventory', 'sample_output'):
                self.assertEqual(first[key], second[key])
            process.assert_called_once()
        page = self.agent.dispatch('read_json', dict(root='work', path=first['member_inventory'],
                                  pointer='/members', offset=28, limit=6))
        self.assertEqual(page['items'], inventory['members'][28:34])
        self.assertEqual(page['total_entries'], 63)
        self.assertEqual(page['next_offset'], 34)

    def test_extraction_members_are_actual_files_and_cached_fields_remain_available(self):
        self.members = {f'scripts/chapter-{index:03d}.bin': f'fixture-{index}'.encode()
                        for index in range(34)}
        self.members['scripts/场景 35.bin'] = b'last fixture'
        arguments = dict(command='x', archive='data.arc', filter='\\.bin$', length=24)
        with mock.patch.object(host.subprocess, 'run', side_effect=self.garbro) as process:
            first = self.agent.dispatch('run_skill', arguments)
            self.assert_inventory(first, extraction=True)
            self.assertNotIn('progress-message-decoy.bin', first['members'])
            second = self.agent.dispatch('run_skill', dict(arguments, offset=24))
            self.assertTrue(second['reused'])
            self.assert_inventory(second, extraction=True)
            for key in ('members', 'member_count', 'member_inventory', 'sample_output'):
                self.assertEqual(first[key], second[key])
            process.assert_called_once()
            # A cache hit must not hide a corrupted member. The actual extraction
            # and its complete inventory become available again after rebuilding.
            self.agent.path('work', first['sample_output']).write_bytes(b'changed')
            rebuilt = self.agent.dispatch('run_skill', arguments)
            self.assertFalse(rebuilt.get('reused', False))
            self.assert_inventory(rebuilt, extraction=True)
            self.assertEqual(process.call_count, 2)

    def test_small_member_lists_fit_the_tool_budget_without_truncation(self):
        self.members = {'names table.tbl': b'names', 'scripts/场景 01.bin': b'one', 'scripts/scene02.bin': b'two'}
        with mock.patch.object(host.subprocess, 'run', side_effect=self.garbro):
            for command in ('l', 'x'):
                with self.subTest(command=command):
                    arguments = dict(command=command, archive='data.arc')
                    if command == 'x':
                        arguments['filter'] = '.'
                    result = self.agent.dispatch('run_skill', arguments)
                    self.assert_inventory(result, extraction=command == 'x')
                    bounded = host.CONTEXT['bounded_result'](self.agent.work,
                                                             dict(result, observation_ref='O1'))
                    self.assertNotIn('truncated', bounded)
                    self.assertCountEqual(self.normalized(bounded['members']), list(self.members))

    def test_verified_source_selection_precedes_first_script_program_even_when_gated(self):
        source = 'def extract(data, metadata): return []'
        with mock.patch.object(host.subprocess, 'run', side_effect=self.garbro):
            self.agent.dispatch('run_skill', dict(command='l', archive='data.arc'))
            with mock.patch.object(self.agent.workflow, 'describe', wraps=self.agent.workflow.describe) as describe:
                self.assertEqual(self.agent.work_guidance()['stage'], 'resources')
                prerequisites = describe.call_args.args[2]
                self.assertTrue(prerequisites['resource_selection_required'])
                self.assertFalse(prerequisites['resource_selected'])
            self.agent.workflow.inspection_turns = set(range(8))
            with self.assertRaisesRegex(ValueError, '[Vv]erify|[Rr]esource|[Ss]elect'):
                self.agent.dispatch('write_program', dict(kind='script', source=source))
            self.assertNotIn('script', self.agent.programs)
            self.assertFalse((self.agent.work / 'script_extractor.py').exists())
            selected = self.agent.dispatch('verify_resource_selection',
                dict(script_name='scene.bin', selected='data.arc'))
            self.assertEqual(selected['selected'], str(self.archive))
            with mock.patch.object(self.agent.workflow, 'describe', wraps=self.agent.workflow.describe) as describe:
                self.assertEqual(self.agent.work_guidance()['stage'], 'script_program')
                self.assertTrue(describe.call_args.args[2]['resource_selected'])
            result = self.agent.dispatch('write_program', dict(kind='script', source=source))
            self.assertEqual(result['saved'], 'script_extractor.py')
            self.assertIn('script', self.agent.programs)
            with mock.patch.object(self.agent.workflow, 'describe', wraps=self.agent.workflow.describe) as describe:
                self.agent.work_guidance()
                self.assertFalse(describe.call_args.args[2]['resource_selection_required'])

    def test_rejected_source_selection_does_not_unlock_first_script_write(self):
        with mock.patch.object(host.subprocess, 'run', side_effect=self.garbro):
            with self.assertRaisesRegex(ValueError, 'Selected resource'):
                self.agent.dispatch('verify_resource_selection', dict(script_name='absent.bin', selected='data.arc'))
        self.assertEqual(self.agent.resource_selections, {})
        self.assertEqual(self.agent.work_guidance()['stage'], 'resources')
        with self.assertRaisesRegex(ValueError, '[Vv]erify|[Rr]esource|[Ss]elect'):
            self.agent.dispatch('write_program', dict(kind='script', source='def extract(data, metadata): return []'))
        self.assertNotIn('script', self.agent.programs)

    def test_fixture_without_inventory_keeps_existing_script_write_path(self):
        config = dict(self.config, inventory=[], work=str(self.root / 'work-without-inventory'))
        agent = host.Agent(config)
        agent.read_skill = agent.read_contract = agent.used_skill = True
        with mock.patch.object(agent.workflow, 'describe', wraps=agent.workflow.describe) as describe:
            self.assertEqual(agent.work_guidance()['stage'], 'script_program')
            self.assertFalse(describe.call_args.args[2]['resource_selection_required'])
        result = agent.dispatch('write_program', dict(kind='script', source='def extract(data, metadata): return []'))
        self.assertEqual(result['saved'], 'script_extractor.py')

    def test_expanding_extraction_preserves_unchanged_sample_fingerprints(self):
        self.members = {'sample.bin': b'Original'}
        with mock.patch.object(host.subprocess, 'run', side_effect=self.garbro):
            first = self.agent.dispatch('run_skill', dict(command='x', archive='data.arc', filter='sample'))
            sample = self.agent.path('work', first['sample_output'])
            original = self.agent.file_fingerprint(sample)
            description = self.agent.input_description('load_script_metadata', dict(root='work', path=first['sample_output']))
            self.members = {'sample.bin': b'Original', 'later.bin': b'Another'}
            expanded = self.agent.dispatch('run_skill', dict(command='x', archive='data.arc', filter='.*'))
            self.assertEqual(expanded['member_count'], 2)
            self.assertEqual(self.agent.file_fingerprint(sample), original)
            self.assertEqual(self.agent.input_description('load_script_metadata', dict(root='work', path=first['sample_output'])), description)
            # A fresh process has no in-memory extraction cache. Re-extracting
            # identical bytes must still preserve all dependency fingerprints.
            restarted = host.Agent(self.config)
            restarted.read_skill = True
            restarted.dispatch('run_skill', dict(command='x', archive='data.arc', filter='.*'))
            self.assertEqual(restarted.file_fingerprint(sample), original)
            self.assertFalse(list((self.agent.work / 'unpacked').glob('.extract-*')))

    def test_members_and_sample_are_from_this_filter_not_previous_output(self):
        self.members = {'a-name.txt': b'Names'}
        with mock.patch.object(host.subprocess, 'run', side_effect=self.garbro):
            first = self.agent.dispatch('run_skill', dict(command='x', archive='data.arc', filter='name'))
            self.members = {'z-script.bin': b'Script'}
            second = self.agent.dispatch('run_skill', dict(command='x', archive='data.arc', filter='script'))
        self.assertEqual(second['members'], ['z-script.bin'])
        self.assertEqual(second['member_count'], 1)
        self.assertTrue(second['sample_output'].endswith('z-script.bin'))
        self.assertTrue(self.agent.path('work', first['sample_output']).is_file())

    def test_filename_glob_is_translated_for_garbro_and_cli_failure_is_explicit(self):
        self.members = {'sample.txt': b'Data'}
        with mock.patch.object(host.subprocess, 'run', side_effect=self.garbro) as process:
            result = self.agent.dispatch('run_skill', dict(command='x', archive='data.arc', filter='*.txt'))
        pattern = process.call_args.args[0][process.call_args.args[0].index('-f') + 1]
        self.assertIsNotNone(re.fullmatch(pattern, 'sample.txt'))
        self.assertIsNone(re.fullmatch(pattern, 'sample.bin'))
        self.assertEqual(result['filter_syntax'], 'glob')
        with mock.patch.object(host.subprocess, 'run', return_value=mock.Mock(returncode=1, stdout=b'', stderr=b'Error: Invalid filter')):
            failed = self.agent.dispatch('run_skill', dict(command='x', archive='data.arc', filter='*bad', filter_syntax='regex'))
        self.assertIn('error', failed)
        self.assertFalse(list((self.agent.work / 'unpacked').glob('.extract-*')))

    def test_no_match_batch_cannot_look_like_successful_verification(self):
        for name, extra in [('verify_resource_selection_batch', dict(selected='data.arc')),
                            ('test_program_batch', {})]:
            with self.subTest(tool=name), self.assertRaisesRegex(ValueError, 'GLOB matched no files'):
                self.agent.dispatch(name, dict(root='game', path='.', filter='nonexistent', **extra))


if __name__ == '__main__':
    unittest.main()
