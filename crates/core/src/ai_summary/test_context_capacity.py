"""Public checkpoint capacity, exact archive recovery and source preservation."""
import copy
import hashlib
import json
import pathlib
import runpy
import tempfile
import unittest


MODULE_DIR = pathlib.Path(__file__).parent
context = runpy.run_path(str(MODULE_DIR / 'agent_context.py'))
host = runpy.run_path(str(MODULE_DIR / 'agent_host.py'))
research = runpy.run_path(str(MODULE_DIR / 'agent_research.py'))


class ContextCapacityTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.work = pathlib.Path(temporary.name)

    def fixture(self):
        def code(marker, size):
            return 'def extract(data, metadata): return {}\n#' + marker * size

        def inventory(marker, count):
            return dict(total_entries=count, entries={str(i): dict(binding=marker * 140,
                target=dict(root='work', path='paired-' + str(i) + '.bin')) for i in range(count)})

        failure = dict(kind='save', category='save', root='save', path='current.sav',
                       source_sha256='a' * 64, error='Current-node coordinate mismatch: ' + 'e' * 1000)
        action = 'Repair the current parser, preserve verified records, then test current.sav. ' + 'a' * 1850
        speakers = dict(sample=dict(root='work', path='sample.bin'), pending_targets=[],
                        next_step='ready_for_batch', resource_counts=dict(global_resources=1, scoped_resources=45),
                        scoped_resources=inventory('s', 22))
        progress = dict(dialogue_count=5701, speaker_named_rows=3213, validated_saves=0,
            required_saves=2, ready_for_finish=False, speaker_review_required=False,
            pending_script_inputs=[], pending_speaker_inputs=[], pending_targets=['current.sav'],
            pending_samples=['current.sav'], save_verification_version=1,
            transcript_coverage=dict(expected_members=45, covered_members=45, pending_members=[]),
            speaker_metadata_progress=dict(state_ref='state.speaker_metadata_progress'),
            last_script_failure=None, last_save_failure=failure)
        request = dict(copy.deepcopy(progress), phase='prepare', remaining_requests=70,
            remaining_work_requests=69, request_limit=100, hard_request_limit=100,
            guidance=dict(stage='save_validation', goal=action, next_action=action,
                gate_instruction='Repair this exact failure.', probe=dict(fact='native coordinate',
                    targets=[dict(root='save', path='current.sav')]), pending_validation=['current.sav']))
        programs = {kind: dict(path=kind + '_extractor.py', source=code(marker, size))
                    for kind, marker, size in [('script', 's', 3500), ('save', 'v', 5750)]}
        helpers = {key: dict(path=key + '.py', source=code(marker, size))
                   for key, marker, size in [('names', 'n', 1200), ('bindings', 'b', 3250)]}
        analyses = [dict(source=code(marker, size), result_preview=inventory(marker, 4),
                    full_record='tool-results/' + marker * 64 + '.json',
                    read_result=dict(root='work', path='tool-results/' + marker * 64 + '.json', pointer='/result'))
                    for marker, size in [('a', 3950), ('b', 4580)]]
        notes = dict(findings={str(i): dict(status='observed', summary='Observed fixture structure ' + 'n' * 300,
                    evidence_refs=['O' + str(i)], evidence_call_ids=['call-' + str(i)]) for i in range(4)},
                    next_steps=['Repair the exact current node.'], model_authored=True, host_verified=False)
        observations = [dict(ref='O' + str(i), tool='inspect_data', tool_call_id='call-' + str(i),
                    full_record='tool-results/' + str(i) * 64 + '.json', successful=True,
                    preview=inventory('p', 4), inputs=[dict(root='work', path='fixture.bin')]) for i in range(4)]
        state = dict(progress=progress, current_request=request, next_action=action,
            request_budget_instruction='Use the remaining work budget to repair the current failure.',
            last_script_failure=None, last_save_failure=failure, read_contract=True, read_skill=True,
            program_sources=programs, helper_sources=helpers, recent_analysis_sources=analyses,
            resource_selections=inventory('r', 13), speaker_resource_targets=inventory('t', 22),
            companion_metadata=inventory('c', 25), speaker_metadata_progress=speakers,
            metadata_helpers=inventory('h', 7), required_script_sets=inventory('q', 4),
            extracted_from=inventory('x', 6), speaker_contract=dict(proof='proof' * 550),
            workflow=dict(stage='save_validation', probe=dict(fact='native coordinate', target='current.sav'),
                          pending_validation=['current.sav'], read_pages=[dict(root='skill', path='engine.md',
                          offset=i * 100, length=100) for i in range(24)]),
            documents={'engine.md': dict(sha256='d' * 64, complete=True, total_characters=2400,
                                        next_unread_offset=2400),
                       'partial.md': dict(sha256='e' * 64, complete=False, total_characters=5000,
                                         next_unread_offset=2048)},
            section_archives={'resource_selections': 'tool-results/' + 'f' * 64 + '.json'})
        return dict(kind='verified_host_checkpoint', instruction='Host public state is authoritative.',
                    state=state, model_analysis=notes,
                    structural_ledger=dict(observations=observations, recent_observations=[]),
                    research=dict(observations=observations, known_errors=observations), recent_observations=[])

    def resolve(self, locator):
        self.assertEqual(locator['root'], 'work')
        path = self.work / locator['path']
        raw = path.read_bytes()
        self.assertEqual(path.stem, hashlib.sha256(raw).hexdigest())
        return host['json_pointer_value'](json.loads(raw), locator['pointer'])

    def test_complete_public_archive_and_every_moved_field_are_exact(self):
        original = self.fixture()
        before = copy.deepcopy(original)
        projected = context['compact_checkpoint'](self.work, original)
        self.assertEqual(original, before)
        self.assertEqual(self.resolve(projected['public_checkpoint']['read_json']), original)
        self.assertEqual(projected['public_checkpoint']['sha256'], hashlib.sha256(context['encode'](original)).hexdigest())
        for key in ('resource_selections', 'speaker_resource_targets', 'companion_metadata',
                    'speaker_metadata_progress', 'metadata_helpers', 'required_script_sets', 'extracted_from',
                    'speaker_contract'):
            item = projected['state'][key]
            self.assertEqual(self.resolve(item['read_json']), original['state'][key])
            self.assertEqual(hashlib.sha256(context['encode'](self.resolve(item['read_json']))).digest(),
                             hashlib.sha256(context['encode'](original['state'][key])).digest())
        self.assertEqual(projected['state']['section_archives'], original['state']['section_archives'])
        for section in ('structural_ledger', 'research'):
            for key, rows in projected[section].items():
                for index, row in enumerate(rows):
                    self.assertEqual(self.resolve(row['read_json']), original[section][key][index])

    def test_sources_current_failure_notes_probe_targets_and_document_offsets_remain_exact(self):
        original = self.fixture()
        projected = context['compact_checkpoint'](self.work, original)
        state, previous = projected['state'], original['state']
        for key in ('program_sources', 'helper_sources', 'last_save_failure', 'last_script_failure',
                    'next_action', 'documents', 'read_contract', 'read_skill'):
            self.assertEqual(state[key], previous[key])
        for old, new in zip(previous['recent_analysis_sources'], state['recent_analysis_sources']):
            self.assertEqual(old['source'], new['source'])
            self.assertEqual(old['read_result'], new['read_result'])
            self.assertEqual(self.resolve(new['result_preview']['read_json']), old['result_preview'])
        self.assertEqual(projected['model_analysis'], original['model_analysis'])
        self.assertEqual(state['workflow']['probe'], previous['workflow']['probe'])
        self.assertEqual(state['workflow']['pending_validation'], previous['workflow']['pending_validation'])
        self.assertEqual(state['progress']['pending_targets'], ['current.sav'])
        self.assertEqual(self.resolve(state['workflow']['read_pages']['read_json']), previous['workflow']['read_pages'])
        self.assertEqual(state['workflow']['read_pages']['document_state_ref'], 'state.documents')
        self.assertTrue(state['documents']['engine.md']['complete'])
        self.assertEqual(state['documents']['partial.md']['next_unread_offset'], 2048)

    def test_duplicate_progress_and_action_references_are_recoverable_and_not_circular(self):
        original = self.fixture()
        state = context['compact_checkpoint'](self.work, original)['state']
        request = state['current_request']
        duplicate = request['progress_fields']
        self.assertEqual(duplicate['state_ref'], 'state.progress')
        previous_request = self.resolve(duplicate['read_json'])
        self.assertEqual(set(duplicate['fields']), set(original['state']['progress']))
        for key in duplicate['fields']:
            self.assertNotIn(key, request)
            self.assertEqual(previous_request[key], original['state']['progress'][key])
        failure_ref = state['progress']['last_save_failure']
        self.assertEqual(failure_ref['state_ref'], 'state.last_save_failure')
        self.assertEqual(self.resolve(failure_ref['read_json']), state['last_save_failure'])
        for key in ('goal', 'next_action'):
            ref = request['guidance'][key]
            self.assertEqual(ref['state_ref'], 'state.next_action')
            self.assertEqual(self.resolve(ref['read_json']), state['next_action'])

    def test_incomplete_or_active_script_and_speaker_inventories_keep_their_visible_shapes(self):
        for change in ('stage', 'coverage', 'script_pending', 'speaker_pending', 'review', 'scope_pending'):
            with self.subTest(change=change):
                original = self.fixture()
                state = original['state']
                if change == 'stage': state['workflow']['stage'] = 'speaker_review'
                elif change == 'coverage': state['progress']['transcript_coverage']['covered_members'] = 1
                elif change == 'script_pending': state['progress']['pending_script_inputs'] = ['remaining.bin']
                elif change == 'speaker_pending': state['progress']['pending_speaker_inputs'] = ['remaining.bin']
                elif change == 'review': state['progress']['speaker_review_required'] = True
                else: state['speaker_metadata_progress']['pending_targets'] = ['remaining.bin']
                projected = context['compact_checkpoint'](self.work, original)['state']
                for key in ('resource_selections', 'speaker_resource_targets', 'companion_metadata',
                            'speaker_metadata_progress', 'metadata_helpers', 'required_script_sets',
                            'extracted_from', 'speaker_contract'):
                    self.assertEqual(projected[key], state[key])

    def test_capacity_includes_wire_escaping_opaque_reply_tools_and_one_next_progress(self):
        original = self.fixture()
        projected = context['compact_checkpoint'](self.work, original)
        prefix = [dict(role='system', content='s' * 7200), dict(role='user', content='t' * 6250),
                  dict(role='user', content=json.dumps(projected, ensure_ascii=False))]
        def sized_message(role, size):
            message = dict(role=role, content='')
            message['content'] = 'x' * (size - len(context['encode'](message)))
            self.assertEqual(len(context['encode'](message)), size)
            return message
        opaque = sized_message('assistant', 70823)
        tools = [sized_message('tool', 2538), sized_message('tool', 923)]
        update = context['progress_message'](self.work, original['state']['current_request'],
                                              original['state']['request_budget_instruction'])
        self.assertLessEqual(len(context['encode'](prefix)), 70000)
        self.assertLessEqual(len(context['encode'](prefix + [opaque] + tools + [update])), context['MAX_CONTEXT_BYTES'])
        self.assertEqual(sum(m['role'] == 'user' and m['content'].startswith('Host progress: ')
                             for m in prefix + tools + [update]), 1)
        self.assertEqual(context['MAX_CONTEXT_BYTES'], 480000)
        self.assertEqual((context['PROGRAM_SOURCES_BYTES'], context['HELPER_SOURCES_BYTES'],
                          context['ANALYSIS_SOURCES_BYTES']), (24000, 24000, 12000))

    def test_analysis_preview_and_explicit_pointer_select_the_same_actual_packet_layer(self):
        store = research['ResearchStore'](self.work)
        inner = dict(table_count=3, key_inventory=['native_id', 'coordinate'])
        outer = dict(result=inner, source_context=dict(path='fixture.bin'))
        function = dict(name='inspect_data', arguments=json.dumps(dict(root='work', path='fixture.bin',
                        source='def extract(data, metadata): return {}')))
        store.observe('actual-call', function, outer)
        calls = []
        def preview(value, budget):
            calls.append((value, budget))
            return copy.deepcopy(value)
        entries = context['recent_analysis_sources'](self.work, store.snapshot(), structural_preview=preview)
        entry = entries[0]
        self.assertEqual(calls, [(inner, 1200)])
        self.assertEqual(entry['result_preview_pointer'], '/result/result')
        self.assertEqual(self.resolve(entry['read_result']), outer)
        self.assertEqual(self.resolve(entry['read_analysis_result']), entry['result_preview'])
        self.assertEqual(entry['source'], json.loads(function['arguments'])['source'])

    def test_actual_structural_inventory_ignores_old_serialized_preview_and_stays_visible(self):
        store = research['ResearchStore'](self.work)
        inner = dict(native_id=list(range(1000)), coordinate=list(range(1000)), table_count=3,
                     **{'field_' + str(i): list(range(100)) for i in range(40)})
        packet = dict(source_context=dict(large='x' * 12000), result=inner)
        function = dict(name='inspect_data', arguments=json.dumps(dict(root='work', path='fixture.bin',
                        source='def extract(data, metadata): return {}')))
        handle = store.observe('old-packet', function, packet)
        store.records[handle['ref']]['preview'] = dict(structural_json='{"WRONG_OLD_PREFIX":')
        store._persist()
        entry = context['recent_analysis_sources'](self.work, store.snapshot(),
                    structural_preview=research['structural_preview'])[0]
        self.assertNotIn('structural_json', entry['result_preview'])
        keys = [row[0] for row in entry['result_preview']['key_inventory']]
        self.assertTrue({'native_id', 'coordinate', 'table_count'} <= set(keys))
        self.assertLessEqual(len(context['encode'](entry['result_preview'])), 1200)
        self.assertEqual(self.resolve(entry['read_analysis_result']), inner)
        original = self.fixture()
        original['state']['recent_analysis_sources'][0] = entry
        projected = context['compact_checkpoint'](self.work, original)
        self.assertEqual(projected['state']['recent_analysis_sources'][0], entry)


if __name__ == '__main__':
    unittest.main()
