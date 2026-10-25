"""Supplied resource inventory remains bounded, relative and readable after resets."""
import copy
import json
import pathlib
import tempfile
import unittest
from unittest import mock

import test_agent_host as fixtures


host = fixtures.host


class SourceInventoryTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = pathlib.Path(temporary.name)
        self.game, self.save, self.skill = (self.root / name for name in ('game', 'save', 'skill'))
        for folder in (self.game, self.save, self.skill):
            folder.mkdir()
        (self.game / 'fixture.exe').write_bytes(b'fixture')
        self.config = dict(game=str(self.game), save=str(self.save), skill=str(self.skill),
            work=str(self.game / '.galgear-ai-summary/fresh'), title='Inventory fixture', language='en-US',
            provider='offline-fixture', model='never-called', max_turns=2)

    def file(self, relative, data=b'actual file'):
        path = self.game / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
        return path

    def agent(self, inventory):
        return host.Agent(dict(self.config, inventory=inventory))

    def begin(self, agent):
        class InspectionStopped(Exception):
            pass
        def stop(messages, tools, turn, phase):
            self.last_request = copy.deepcopy(messages)
            raise InspectionStopped()
        with mock.patch.object(agent, 'run_turn', side_effect=stop), \
                mock.patch.object(agent, 'request_completion', side_effect=AssertionError('No model calls')) as request:
            with self.assertRaises(InspectionStopped):
                agent.run_session()
        request.assert_not_called()
        return json.loads(agent.context_task)

    def test_only_supplied_real_game_files_are_exposed_with_safe_relative_paths(self):
        first = self.file('nested/dialogue.arc')
        second = self.file('localized/story.pak')
        self.file('unlisted.arc')
        outside = self.root / 'private.arc'
        outside.write_bytes(b'private')
        cached = self.file('.galgear-ai-summary/old/answer.arc')
        current_work = self.file('.galgear-ai-summary/fresh/current.arc')
        inventory = [dict(path=str(first), size=first.stat().st_size, modified=123),
            dict(path='localized/story.pak'), dict(path=str(outside)), dict(path='missing.arc'),
            dict(path='nested'), dict(path='../private.arc'), dict(path=str(cached)),
            dict(path=str(current_work)), dict(path=str(first)), {'wrong': 'shape'}, None]
        before = copy.deepcopy(inventory)
        agent = self.agent(inventory)
        self.assertEqual(inventory, before)
        value = json.loads((agent.work / 'source-inventory.json').read_text('utf-8'))
        self.assertEqual(value['root'], 'game')
        self.assertEqual(value['supplied_entries'], len(inventory))
        self.assertEqual(value['excluded_entries'], len(inventory) - 2)
        self.assertEqual([row['path'] for row in value['entries']], ['nested/dialogue.arc', 'localized/story.pak'])
        self.assertEqual(value['entries'][0]['modified'], 123)
        self.assertEqual(value['entries'][1]['size'], second.stat().st_size)
        self.assertNotIn(str(self.root), json.dumps(value))

    def test_inventory_processing_never_scans_for_unlisted_files(self):
        listed = self.file('listed.arc')
        self.file('not-listed.arc')
        with mock.patch.object(pathlib.Path, 'iterdir', side_effect=AssertionError('No inventory rescan')):
            agent = self.agent([dict(path=str(listed))])
        with mock.patch.object(pathlib.Path, 'glob', side_effect=AssertionError('No inventory rescan')), \
                mock.patch.object(pathlib.Path, 'rglob', side_effect=AssertionError('No inventory rescan')), \
                mock.patch.object(pathlib.Path, 'iterdir', side_effect=AssertionError('No inventory rescan')):
            value = agent.prepare_source_inventory([dict(path=str(listed))])
        self.assertEqual([row['path'] for row in value['entries']], ['listed.arc'])

    def test_missing_inventory_does_not_claim_an_empty_resource_directory(self):
        self.file('real-resource.arc')
        agent = host.Agent(self.config)
        task = agent.source_inventory_task()
        self.assertFalse(task['provided'])
        self.assertEqual(task['preview'], [])
        self.assertIn('not that resource files are absent', task['instruction'])

    def test_full_inventory_and_pagination_survive_bounded_initial_preview(self):
        paths = [self.file('parts/chapter-' + str(index).zfill(3) + '.arc') for index in range(40)]
        agent = self.agent([dict(path=str(path)) for path in paths])
        task = self.begin(agent)['source_inventory']
        self.assertEqual(task['total_entries'], 40)
        self.assertEqual(len(task['preview']), 24)
        self.assertEqual(task['omitted_entries'], 16)
        self.assertLessEqual(len(host.CONTEXT['encode'](task)), 6000)
        self.assertEqual(task['read_json'], dict(root='work', path='source-inventory.json', pointer='/entries', offset=0, limit=20))
        first = agent.dispatch('read_json', task['read_json'])
        second = agent.dispatch('read_json', dict(task['read_json'], offset=20))
        self.assertEqual([row['path'] for row in first['items'] + second['items']],
                         [path.relative_to(self.game).as_posix() for path in paths])

    def test_utf8_preview_budget_is_enforced_without_clipping_file_paths(self):
        name = '\u8def' * 55
        paths = [self.file('/'.join((name + str(index), name, name, 'story.arc'))) for index in range(24)]
        agent = self.agent([dict(path=str(path)) for path in paths])
        task = agent.source_inventory_task()
        self.assertLess(len(task['preview']), 24)
        self.assertLessEqual(len(host.CONTEXT['encode'](task)), 6000)
        self.assertEqual(task['preview'], agent.source_inventory['entries'][:len(task['preview'])])
        self.assertEqual(agent.source_inventory['total_entries'], 24)

    def test_reset_preserves_the_complete_inventory_locator(self):
        path = self.file('scenario.arc')
        agent = self.agent([dict(path=str(path))])
        task = self.begin(agent)
        agent.reset_context = True
        messages = []
        agent.prepare_request(messages, dict(phase='prepare', remaining_work_requests=1), 'Continue')
        self.assertEqual(json.loads(messages[1]['content'])['source_inventory'], task['source_inventory'])
        summary = json.loads(messages[2]['content'])
        reference = summary['state']['source_inventory']['read_json']
        self.assertEqual(reference, task['source_inventory']['read_json'])
        self.assertEqual(agent.dispatch('read_json', reference)['items'][0]['path'], 'scenario.arc')

    def test_resume_of_an_older_task_refreshes_the_inventory_reference(self):
        path = self.file('scenario.arc')
        inventory = [dict(path=str(path))]
        agent = self.agent(inventory)
        self.begin(agent)
        legacy = json.loads(self.last_request[1]['content'])
        legacy.pop('source_inventory')
        self.last_request[1]['content'] = json.dumps(legacy)
        agent.audit_messages = copy.deepcopy(self.last_request)
        agent.checkpoint(self.last_request)
        restored = self.agent(inventory)
        task = self.begin(restored)
        sent_task = json.loads(self.last_request[1]['content'])
        self.assertEqual(sent_task['source_inventory'], task['source_inventory'])
        self.assertGreater(restored.context_resets, 0)
    def test_symlink_inventory_alias_is_excluded(self):
        target = self.file('actual.arc')
        link = self.game / 'alias.arc'
        try:
            link.symlink_to(target)
        except OSError as error:
            self.skipTest('Symlinks unavailable: ' + str(error))
        agent = self.agent([dict(path=str(link)), dict(path=str(target))])
        self.assertEqual([entry['path'] for entry in agent.source_inventory['entries']], ['actual.arc'])


    def test_native_count_tracks_canonical_part_replacement_and_removal(self):
        agent = self.agent([])
        first = [dict(source='one', position=0, text='First', speaker=None, _native=[dict(kind='call_return')]),
                 dict(source='one', position=1, text='Second', speaker=None, _native=[])]
        agent.parts['one'] = first
        self.assertEqual(agent.preparation_status()['native_attested_rows'], 1)
        cached = agent.native_row_counts['one']
        self.assertEqual(agent.preparation_status()['native_attested_rows'], 1)
        self.assertIs(agent.native_row_counts['one'], cached)
        agent.parts['one'] = [dict(row, _native=[]) for row in first]
        self.assertEqual(agent.preparation_status()['native_attested_rows'], 0)
        agent.parts['two'] = first
        self.assertEqual(agent.preparation_status()['native_attested_rows'], 1)
        agent.parts.clear()
        self.assertEqual(agent.preparation_status()['native_attested_rows'], 0)
        self.assertEqual(agent.native_row_counts, {})


if __name__ == '__main__':
    unittest.main()
