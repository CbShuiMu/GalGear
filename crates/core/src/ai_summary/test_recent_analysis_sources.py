"""Synthetic recovery tests; no game inputs, parser execution or provider calls."""
import hashlib
import json
import pathlib
import runpy
import tempfile
import unittest


MODULE_DIR = pathlib.Path(__file__).parent
context = runpy.run_path(str(MODULE_DIR / 'agent_context.py'))
research = runpy.run_path(str(MODULE_DIR / 'agent_research.py'))


class RecentAnalysisSourcesTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.work = pathlib.Path(temporary.name)
        self.store = research['ResearchStore'](self.work)
        self.serial = 0

    def observe(self, source=None, result=None, tool='inspect_data', target=None):
        self.serial += 1
        arguments = dict(root='work', path='fixture-' + str(self.serial) + '.bin')
        if source is not None:
            arguments['source'] = source
        if target is not None:
            arguments['target_script'] = target
        function = dict(name=tool, arguments=json.dumps(arguments, ensure_ascii=False))
        handle = self.store.observe('call-' + str(self.serial), function,
                                    result if result is not None else dict(count=self.serial, offset=4))
        return handle

    def reset(self, facts=None, audit=None):
        facts = dict(facts or {}, structural_ledger=self.store.snapshot())
        messages = context['reset_messages'](self.work, 'system', '{}', facts, audit or [])
        return messages, json.loads(messages[2]['content'])

    def test_two_sources_survive_many_unrelated_observations_and_repeated_resets(self):
        class PrivateAssistant(dict):
            def __getitem__(self, key):
                if key in ('content', 'reasoning_content'):
                    raise AssertionError('Recovery must not inspect private assistant fields')
                return super().__getitem__(key)

            def get(self, key, default=None):
                if key in ('content', 'reasoning_content'):
                    raise AssertionError('Recovery must not inspect private assistant fields')
                return super().get(key, default)

        discarded = 'def extract(data, metadata): return {"count": 1}'
        first = '# 中文 "quote"\r\ndef extract(data, metadata):\r\n    return {"count": len(data)}\r\n'
        second = 'def extract(data, metadata):\n    raise RuntimeError("never execute during recovery")\n'
        self.observe(discarded)
        first_handle = self.observe(first)
        second_handle = self.observe(second)
        for _ in range(25):
            self.observe(tool='read_bytes')
        snapshot = self.store.snapshot()
        self.assertTrue(all(row['tool'] == 'read_bytes' for row in snapshot['observations']))
        messages, checkpoint = self.reset(audit=[PrivateAssistant(role='assistant', content='PRIVATE_ASSISTANT_TEXT',
                                                               reasoning_content='PRIVATE_REASONING_TEXT')])
        entries = checkpoint['state']['recent_analysis_sources']
        self.assertEqual([row['source'] for row in entries], [first, second])
        self.assertEqual([row['observation_ref'] for row in entries], [first_handle['ref'], second_handle['ref']])
        self.assertEqual(entries[0]['source_sha256'], hashlib.sha256(first.encode('utf-8')).hexdigest())
        self.assertEqual(entries[0]['root'], 'work')
        self.assertEqual(entries[0]['path'], 'fixture-2.bin')
        self.assertNotIn(discarded, json.dumps(checkpoint, ensure_ascii=False))
        self.assertNotIn('PRIVATE_', json.dumps(messages, ensure_ascii=False))
        restored = research['ResearchStore'](self.work)
        self.store = restored
        _, again = self.reset()
        self.assertEqual(again['state']['recent_analysis_sources'], entries)
        self.assertIn('not saved parsers or verified helpers', checkpoint['instruction'])

    def test_oversized_sources_are_whole_archives_without_partial_code(self):
        sources = ['def extract(data, metadata): return {}\n#' + marker * 6100 for marker in ('a', 'b')]
        for source in sources:
            self.observe(source)
        _, checkpoint = self.reset()
        entries = checkpoint['state']['recent_analysis_sources']
        self.assertEqual(len(entries), 2)
        for entry, source in zip(entries, sources):
            self.assertNotIn('source', entry)
            self.assertTrue(entry['source_archived'])
            self.assertEqual(entry['source_pointer'], '/source')
            archived = json.loads((self.work / entry['full_source']).read_bytes())
            self.assertEqual(archived['source'], source)
            self.assertEqual(entry['source_utf8_bytes'], len(source.encode('utf-8')))

    def test_result_locator_reads_the_actual_exchange_payload_without_unwrapping(self):
        source = 'def extract(data, metadata): return {"count": len(data)}'
        payload = dict(count=19, coordinates=[dict(offset=4, length=7)],
                       full_output='untrusted-result-value')
        handle = self.observe(source, result=payload)
        _, checkpoint = self.reset()
        entry = checkpoint['state']['recent_analysis_sources'][0]
        self.assertEqual(entry['full_record'], handle['full_record'])
        self.assertEqual(entry['read_result'], dict(root='work', path=handle['full_record'], pointer='/result'))
        # Resolve the advertised call through the real read_json tool, not an
        # alternate test-only pointer implementation. No parser or model runs.
        host = runpy.run_path(str(MODULE_DIR / 'agent_host.py'))
        (self.work / 'fixture.exe').write_bytes(b'fixture')
        agent = host['Agent'](dict(game=str(self.work), save=str(self.work), skill=str(self.work),
            work=str(self.work), title='Synthetic result locator', language='en-US',
            provider='deepseek', model='offline-fixture', max_turns=30))
        self.assertEqual(agent.dispatch('read_json', entry['read_result']), dict(value=payload))
        wrapper = agent.dispatch('read_json', dict(entry['read_result'], pointer=''))['value']
        self.assertEqual(set(wrapper), {'tool_call_id', 'function', 'result'})
        self.assertEqual(wrapper['result'], payload)
        self.assertEqual(wrapper['tool_call_id'], entry['tool_call_id'])
        self.assertEqual(json.loads(wrapper['function']['arguments'])['source'], source)
        self.assertNotIn('full_output', entry)
        self.assertIn('full_record is the exchange wrapper', checkpoint['instruction'])
        self.assertIn('pointer=""', checkpoint['instruction'])
        self.assertIn('does not unwrap automatically', checkpoint['instruction'])

    def test_analysis_budget_does_not_displace_current_program_or_helper_sources(self):
        sources = ['def extract(data, metadata): return {}\n#' + marker * 5500 for marker in ('a', 'b')]
        for source in sources:
            self.observe(source)
        program = 'def extract(data, metadata): return []\n#' + 'p' * 10000
        helper = 'def extract(data, metadata): return {}\n#' + 'h' * 10000
        messages, checkpoint = self.reset(dict(
            program_sources=dict(script=dict(path='script_extractor.py', source=program),
                                 save=dict(path='save_extractor.py', source=program)),
            helper_sources=dict(one=dict(path='one.py', source=helper),
                                two=dict(path='two.py', source=helper + '2')),
            documents={str(index): dict(complete=True, total_characters=5000, text='x' * 1000)
                       for index in range(100)},
            next_action='Repair the existing parser.'))
        state = checkpoint['state']
        self.assertEqual(state['program_sources']['script']['source'], program)
        self.assertEqual(state['helper_sources']['one']['source'], helper)
        entries = state['recent_analysis_sources']
        self.assertEqual([row['source'] for row in entries], sources)
        self.assertLessEqual(sum(len(context['encode'](row['source'])) for row in entries),
                             context['ANALYSIS_SOURCES_BYTES'])
        self.assertEqual(state['next_action'], 'Repair the existing parser.')
        self.assertLess(len(context['encode'](messages)), context['MAX_CONTEXT_BYTES'])

    def test_failed_inspections_and_other_tools_do_not_replace_successful_sources(self):
        source = 'def extract(data, metadata): return {"count":len(data)}'
        handle = self.observe(source)
        for failure in (dict(error='failed'), dict(exit_code=1), dict(ok=False), dict(success=False)):
            self.observe('def extract(data, metadata): return {"bad":1}', result=failure)
        self.observe('def extract(data, metadata): return {"other":1}', tool='load_script_metadata')
        _, checkpoint = self.reset()
        entries = checkpoint['state']['recent_analysis_sources']
        self.assertEqual(len(entries), 1)
        self.assertEqual(entries[0]['observation_ref'], handle['ref'])
        self.assertEqual(entries[0]['source'], source)

    def test_target_scope_staleness_and_structural_preview_remain_observations(self):
        source = 'def extract(data, metadata): return {"count":len(data)}'
        handle = self.observe(source, result=dict(count=19, encoding='utf-8', arbitrary='STORY_TEXT'),
                              target=dict(root='work', path='paired.fixture'))
        self.store.records[handle['ref']]['stale'] = True
        self.store._persist()
        _, checkpoint = self.reset()
        entry = checkpoint['state']['recent_analysis_sources'][0]
        self.assertEqual(entry['target_script'], dict(root='work', path='paired.fixture'))
        self.assertEqual(entry['result_preview']['count'], 19)
        self.assertEqual(entry['result_preview']['encoding'], 'utf-8')
        self.assertEqual(entry['result_preview']['arbitrary'], dict(string_bytes=10))
        self.assertTrue(entry['stale'])
        self.assertTrue(entry['model_authored'])
        self.assertFalse(entry['host_verified'])
        self.assertNotIn('STORY_TEXT', json.dumps(entry))

    def test_corrupt_content_address_and_mismatched_source_hash_are_skipped(self):
        source = 'def extract(data, metadata): return {}'
        good = self.observe(source)
        corrupt = self.observe(source + '\n# corrupt')
        bad_hash = self.observe(source + '\n# mismatched')
        self.store.records[bad_hash['ref']]['analysis_source_sha256'] = '0' * 64
        self.store._persist()
        (self.work / corrupt['full_record']).write_text('{}', encoding='utf-8')
        _, checkpoint = self.reset()
        entries = checkpoint['state']['recent_analysis_sources']
        self.assertEqual([entry['observation_ref'] for entry in entries], [good['ref']])

    def test_full_record_target_and_success_must_match_ledger(self):
        source = 'def extract(data, metadata): return {}'
        good = self.observe(source)
        wrong_id = self.observe(source + '\n# wrong id')
        failed = self.observe(source + '\n# actual error', result=dict(error='failure'))
        self.store.records[wrong_id['ref']]['tool_call_id'] = 'other-call'
        self.store.records[failed['ref']]['successful'] = True
        self.store._persist()
        _, checkpoint = self.reset()
        self.assertEqual([entry['observation_ref'] for entry in checkpoint['state']['recent_analysis_sources']],
                         [good['ref']])

    def test_invalid_locator_and_outside_ledger_paths_are_never_followed(self):
        source = 'def extract(data, metadata): return {}'
        handle = self.observe(source)
        row = dict(self.store.records[handle['ref']])
        for locator in ('../outside.json', 'tool-results/../outside.json',
                        'tool-results/' + 'g' * 64 + '.json', str(self.work / handle['full_record'])):
            with self.subTest(locator=locator):
                row['full_record'] = locator
                self.assertEqual(context['recent_analysis_sources'](
                    self.work, dict(observations=[row], ledger_file='../analysis-ledger.json')), [])

    def test_snapshot_records_still_work_when_durable_ledger_is_missing_or_invalid(self):
        source = 'def extract(data, metadata): return {}'
        self.observe(source)
        snapshot = self.store.snapshot()
        ledger = self.work / 'analysis-ledger.json'
        ledger.write_text('{', encoding='utf-8')
        entries = context['recent_analysis_sources'](self.work, snapshot)
        self.assertEqual(entries[0]['source'], source)
        ledger.unlink()
        self.assertEqual(context['recent_analysis_sources'](self.work, snapshot), entries)


if __name__ == '__main__':
    unittest.main()
