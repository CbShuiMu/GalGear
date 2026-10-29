"""Preparation regressions for reusable model helpers and isolated dependencies."""
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
    'preparation_speaker_fixtures', pathlib.Path(__file__).with_name('test_speaker_scopes.py'))
fixtures = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fixtures)
host, HELPER, PARSER = fixtures.host, fixtures.HELPER, fixtures.PARSER

GLOBAL_HELPER = '''def extract(data, metadata):
    return {'values':{'encoding':data.decode('ascii')},'speakers':{}}
'''

EMPTY_HELPER = '''def extract(data, metadata):
    return {'values':{'inspected_bytes':len(data)},'speakers':{}}
'''


class PreparationReliabilityTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = pathlib.Path(temporary.name)
        (self.root / 'game.exe').write_bytes(b'fixture')
        self.skill = self.root / 'skill'
        self.skill.mkdir()
        self.config = dict(game=str(self.root), save=str(self.root), skill=str(self.skill),
                           work=str(self.root / 'work'), language='en-US')
        self.agent = host.Agent(self.config)
        self.agent.read_contract = self.agent.read_skill = True
        self.agent.dispatch('write_program', dict(kind='script', source=PARSER))
        for name, speaker in [('first', 'Alice'), ('second', 'Yuka'), ('narration', '')]:
            (self.root / (name + '.ws2')).write_bytes(fixtures.companion_bytes(0, speaker))
            text = ('Message ' + name).encode()
            (self.root / (name + '.lng')).write_bytes(struct.pack('<II', 1, len(text)) + text)

    def load(self, name='first', key='sample-companion', agent=None, source=HELPER):
        return (agent or self.agent).dispatch('load_script_metadata', dict(
            root='game', path=name + '.ws2', key=key, source=source,
            target_script=dict(root='game', path=name + '.lng')))

    def batch(self, helper_key='sample-companion', agent=None, **extra):
        return (agent or self.agent).dispatch('load_script_metadata_batch', dict(
            root='game', path='.', filter='*.ws2', helper_key=helper_key,
            target_root='game', target_path='.', target_extension='.lng', **extra))

    def run_script(self, name='first', agent=None):
        return (agent or self.agent).dispatch('test_program', dict(
            kind='script', root='game', path=name + '.lng'))

    def checkpoint(self):
        self.agent.checkpoint([])
        return json.loads((self.agent.work / 'checkpoint.json').read_text('utf-8'))

    def restored(self, saved):
        agent = host.Agent(self.config)
        # Agent.run reloads saved parser files before restore_checkpoint.
        agent.programs['script'] = (agent.work / 'script_extractor.py').read_text('utf-8')
        compatible = agent.restore_checkpoint(saved)
        return agent, compatible

    def test_verified_helper_is_saved_byte_for_byte_and_returned_for_reuse(self):
        result = self.load()
        self.assertEqual(result['helper_key'], 'sample-companion')
        helper_file = self.agent.work / result['helper_file']
        self.assertTrue(helper_file.resolve().is_relative_to(self.agent.work))
        self.assertEqual(helper_file.read_bytes(), HELPER.encode('utf-8'))
        self.assertEqual(self.agent.metadata_helpers[result['helper_key']]['sha256'],
                         hashlib.sha256(helper_file.read_bytes()).hexdigest())
        self.assertFalse(result['reused'])

    def test_batch_reuses_exact_model_source_and_preserves_verified_sample_key(self):
        sample = self.load()
        self.run_script()
        original_rows = copy.deepcopy(self.agent.parts[str(self.root / 'first.lng')])
        with mock.patch.object(host, 'run_parser', wraps=host.run_parser) as run_parser:
            result = self.batch(sample['helper_key'])
        self.assertEqual(result['loaded'], 3)
        self.assertEqual(len(run_parser.call_args_list), 3)
        self.assertTrue(all(call.args[0] == HELPER for call in run_parser.call_args_list))
        first = next(row for row in result['files'] if row['path'] == 'first.ws2')
        self.assertEqual(first['key'], 'sample-companion')
        self.assertTrue(first['reused'])
        self.assertEqual(self.agent.parts[str(self.root / 'first.lng')], original_rows)
        self.assertEqual(len(self.agent.speaker_resources), 3)
        targets = list(self.agent.speaker_resource_targets.values())
        self.assertEqual(len(targets), len(set(targets)))
        complete = self.agent.dispatch('test_program_batch', dict(root='game', path='.', filter='*.lng'))
        self.assertTrue(all('error' not in row for row in complete['files']))
        self.assertEqual({pathlib.Path(path).stem: rows[0]['speaker']
                          for path, rows in self.agent.parts.items()},
                         dict(first='Alice', second='Yuka', narration=None))

    def test_modified_saved_helper_is_rejected_before_loading_any_new_metadata(self):
        sample = self.load()
        before = copy.deepcopy(self.agent.speaker_resources)
        helper_file = self.agent.work / sample['helper_file']
        helper_file.write_bytes(helper_file.read_bytes() + b'\n# modified after validation\n')
        with self.assertRaisesRegex(ValueError, 'Saved helper changed'):
            self.batch(sample['helper_key'])
        self.assertEqual(self.agent.speaker_resources, before)

    def test_batch_requires_one_source_or_successfully_verified_helper_key(self):
        self.load()
        with self.assertRaisesRegex(ValueError, 'exactly one'):
            self.batch(source=HELPER)
        with self.assertRaisesRegex(ValueError, 'Unknown helper_key'):
            self.batch(helper_key='invented-helper')

    def test_adding_another_script_scope_keeps_the_first_validation_and_cache(self):
        self.load()
        self.run_script()
        source = str(self.root / 'first.lng')
        rows = copy.deepcopy(self.agent.parts[source])
        dependencies = copy.deepcopy(self.agent.script_dependency_stamps[source])
        self.load('second', key='second-companion')
        self.agent.preparation_status()
        self.assertEqual(self.agent.parts[source], rows)
        self.assertEqual(self.agent.script_dependency_stamps[source], dependencies)
        self.assertTrue(self.run_script()['reused'])
        self.assertEqual(self.run_script('second')['preview'][0]['speaker'], 'Yuka')

    def test_script_dependencies_include_own_scope_and_global_inputs_only(self):
        (self.root / 'global.tbl').write_bytes(b'ascii')
        self.agent.dispatch('load_script_metadata', dict(root='game', path='global.tbl',
            key='global-encoding', source=GLOBAL_HELPER))
        self.load()
        self.load('second', key='second-companion')
        first = self.root / 'first.lng'
        dependencies = self.agent.script_dependencies(first)
        self.assertEqual(set(dependencies), {str(first), str(self.root / 'first.ws2'),
                                             str(self.root / 'global.tbl')})
        (self.root / 'second.ws2').write_bytes(fixtures.companion_bytes(0, 'Other'))
        self.assertEqual(self.agent.script_dependencies(first), dependencies)
        (self.root / 'global.tbl').write_bytes(b'utf-8')
        self.assertNotEqual(self.agent.script_dependencies(first), dependencies)

    def test_checkpoint_restores_verified_helper_and_batch_continues_without_source(self):
        sample = self.load()
        self.run_script()
        saved = self.checkpoint()
        restored, compatible = self.restored(saved)
        self.assertTrue(compatible)
        self.assertEqual(restored.metadata_helpers, self.agent.metadata_helpers)
        result = self.batch(sample['helper_key'], agent=restored)
        self.assertEqual(result['loaded'], 3)
        self.assertTrue(next(row for row in result['files'] if row['path'] == 'first.ws2')['reused'])
        self.assertIn(str(self.root / 'first.lng'), restored.parts)
        self.assertEqual(self.run_script('second', agent=restored)['preview'][0]['speaker'], 'Yuka')

    def test_checkpoint_does_not_restore_a_tampered_helper_as_reusable(self):
        sample = self.load()
        saved = self.checkpoint()
        helper_file = self.agent.work / sample['helper_file']
        helper_file.write_bytes(b'def extract(data, metadata): return {}\n')
        restored, _ = self.restored(saved)
        self.assertNotIn(sample['helper_key'], restored.metadata_helpers)
        with self.assertRaisesRegex(ValueError, 'Unknown helper_key'):
            self.batch(sample['helper_key'], agent=restored)

    def test_named_chapter_does_not_hide_other_empty_companion_targets(self):
        self.load()
        self.run_script()
        accepted = copy.deepcopy(self.agent.parts[str(self.root / 'first.lng')])
        self.load('second', key='empty-second', source=EMPTY_HELPER)
        self.load('narration', key='empty-narration', source=EMPTY_HELPER)

        status = self.agent.preparation_status()
        self.assertEqual(status['speaker_named_rows'], 1)
        self.assertTrue(status['speaker_review_required'])
        self.assertEqual(status['pending_speaker_inputs'],
                         [str(self.root / name) for name in ('narration.lng', 'second.lng')])
        self.assertEqual(status['speaker_verification']['scope_version'], 0)
        progress = self.agent.speaker_metadata_progress()
        self.assertEqual(progress['next_step'], 'repair_empty_scope')
        self.assertEqual(progress['sample'], dict(root='game', path='narration.lng', rows=0, named_rows=0))
        self.assertEqual(progress['pending_targets'],
                         [dict(root='game', path=name) for name in ('narration.lng', 'second.lng')])
        self.assertEqual(self.agent.parts[str(self.root / 'first.lng')], accepted)
        self.assertTrue(self.run_script()['reused'])
        self.load('narration', key='proved-narration')
        self.run_script('narration')
        progress = self.agent.speaker_metadata_progress()
        self.assertEqual(progress['next_step'], 'repair_empty_scope')
        self.assertEqual(progress['sample']['path'], 'second.lng')
        self.assertEqual(progress['pending_targets'], [dict(root='game', path='second.lng')])

    def test_empty_companion_focus_and_exact_helper_survive_checkpoint_and_context_reset(self):
        self.load()
        self.run_script()
        other_source = EMPTY_HELPER + '# another empty target\n'
        focused_source = EMPTY_HELPER.replace('\n', '\r\n') + '# focused helper 名字\r\n'
        self.load('second', key='empty-second', source=other_source)
        self.load('narration', key='empty-narration', source=focused_source)
        restored, compatible = self.restored(self.checkpoint())
        self.assertTrue(compatible)
        self.assertEqual(restored.representative_script, dict(root='game', path='first.lng'))

        restored.context_system = 'Continue preparation from the verified checkpoint.'
        restored.context_task = '{}'
        restored.reset_context = True
        messages = []
        restored.prepare_request(messages, dict(phase='prepare', guidance=restored.work_guidance()), 'Continue')
        state = json.loads(messages[2]['content'])['state']
        focused_hash = hashlib.sha256(focused_source.encode()).hexdigest()
        self.assertEqual(state['helper_sources'][focused_hash]['source'], focused_source)
        self.assertEqual(state['helper_sources'][focused_hash]['helper_keys'], ['empty-narration'])
        self.assertNotIn(hashlib.sha256(other_source.encode()).hexdigest(), state['helper_sources'])
        progress = state['speaker_metadata_progress']
        self.assertEqual(progress['next_step'], 'repair_empty_scope')
        self.assertEqual(progress['sample']['path'], 'narration.lng')
        focused = next(item for item in progress['empty_resources'] if item['key'] == 'empty-narration')
        self.assertEqual(focused['companion_input']['path'], 'narration.ws2')
        self.assertEqual(state['program_sources']['script']['source'], PARSER)

    def test_formal_script_and_batch_reject_empty_scope_as_speaker_metadata_failure(self):
        self.load()
        self.run_script()
        self.load('narration', key='real-narration')
        self.load('second', key='empty-second', source=EMPTY_HELPER)
        for regions in ([], [dict(offset=0, length=(self.root / 'second.lng').stat().st_size)]):
            with self.subTest(required_table_regions=regions), mock.patch.dict(host.WORKFLOW,
                    required_table_regions=lambda skill, data: regions):
                with self.assertRaisesRegex(ValueError, 'bindings are empty or missing'):
                    self.run_script('second')
                failure = self.agent.last_script_failure
                self.assertEqual((failure['category'], failure['path']), ('speaker_metadata', 'second.lng'))
                self.assertEqual(failure['error_details']['observed']['scoped_records'], 0)
                self.assertNotIn(str(self.root / 'second.lng'), self.agent.parts)

        batch = self.agent.dispatch('test_program_batch', dict(root='game', path='.', filter='*.lng'))
        self.assertEqual({item['path'] for item in batch['files'] if 'error' in item}, {'second.lng'})
        self.assertEqual(batch['total_dialogue'], 2)
        self.assertEqual(self.agent.speaker_status()['pending_speaker_inputs'], [str(self.root / 'second.lng')])
        self.assertEqual(self.agent.last_script_failure['category'], 'speaker_metadata')

    def test_context_budget_keeps_focused_empty_target_input_when_it_is_last_in_large_inventory(self):
        resources = []
        for index in range(80):
            key = 'empty:chapter-' + str(index).zfill(3)
            resources.append(dict(key=key, records=0, empty=True,
                target=dict(root='work', path='unpacked/localized/chapter-' + str(index).zfill(3) + '.bin'),
                helper_file='metadata-helpers/' + 'a' * 64 + '.py',
                input_access='metadata["speaker_resources"][' + json.dumps(key) + ']',
                companion_input=dict(root='work',
                    path='unpacked/native/chapter-' + str(index).zfill(3) + '.bin',
                    size=47, sha256=hashlib.sha256(('native-' + str(index)).encode()).hexdigest())))
        focus = resources[-1]
        progress = dict(global_resources=[], scoped_resources=[], empty_resources=resources,
                        sample=dict(focus['target'], rows=0, named_rows=0),
                        pending_targets=[focus['target']], next_step='repair_empty_scope')
        messages = host.CONTEXT['reset_messages'](self.agent.work, 'Continue', '{}',
            dict(speaker_metadata_progress=progress,
                 progress=dict(dialogue_count=1, speaker_metadata_progress=progress)), [])
        state = json.loads(messages[2]['content'])['state']
        actual = state['speaker_metadata_progress']
        shown = actual['empty_resources']
        self.assertTrue(shown['truncated'])
        self.assertEqual(shown['total_entries'], 80)
        self.assertEqual(shown['entries'][0], focus)
        self.assertEqual(actual['sample']['path'], focus['target']['path'])
        self.assertEqual(actual['next_step'], 'repair_empty_scope')
        self.assertEqual(actual['resource_counts']['empty_resources'], 80)
        archive = state['section_archives']['speaker_metadata_progress.empty_resources']
        self.assertEqual(json.loads((self.agent.work / archive).read_text('utf-8')), resources)
        self.assertEqual(state['progress']['speaker_metadata_progress']['state_ref'],
                         'state.speaker_metadata_progress')

    def test_valid_replacement_binding_clears_empty_alias_and_resumes_full_batch(self):
        sample = self.load()
        self.run_script()
        accepted = copy.deepcopy(self.agent.parts[str(self.root / 'first.lng')])
        self.load('second', key='empty-second', source=EMPTY_HELPER)
        with self.assertRaises(ValueError):
            self.run_script('second')
        self.load('second', key='repaired-second')
        self.assertEqual(self.agent.speaker_resources['empty-second'], {})
        self.assertEqual(self.agent.empty_speaker_scope_targets(), [])
        self.assertEqual(self.agent.speaker_status()['pending_speaker_inputs'], [])
        self.assertEqual(self.agent.speaker_metadata_progress()['next_step'], 'ready_for_batch')
        self.assertEqual(self.run_script('second')['preview'][0]['speaker'], 'Yuka')
        self.assertIsNone(self.agent.last_script_failure)
        self.assertEqual(self.agent.parts[str(self.root / 'first.lng')], accepted)

        self.batch(sample['helper_key'])
        batch = self.agent.dispatch('test_program_batch', dict(root='game', path='.', filter='*.lng'))
        self.assertTrue(all('error' not in item for item in batch['files']))
        self.assertEqual(batch['total_dialogue'], 3)
        self.assertEqual(self.agent.speaker_status()['speaker_verification']['scope_version'], 2)

    def test_old_accepted_null_rows_and_matching_cache_cannot_bypass_empty_scope_verification(self):
        # Recreate an older verifier's accepted unresolved rows, then give its
        # cache every usual hit condition. The empty target must still force
        # the current independent verifier rather than reusing that result.
        result = self.run_script('second')
        source = str(self.root / 'second.lng')
        legacy_rows = copy.deepcopy(self.agent.parts[source])
        legacy_proof = copy.deepcopy(self.agent.speaker_parts[source])
        self.assertIsNone(legacy_rows[0]['speaker'])
        self.load('second', key='empty-second', source=EMPTY_HELPER)
        self.agent.parts[source] = legacy_rows
        self.agent.speaker_parts[source] = legacy_proof
        self.agent.script_dependency_stamps[source] = self.agent.script_dependencies(self.root / 'second.lng')
        args = dict(kind='script', root='game', path='second.lng')
        key, digest = self.agent.test_signature(args)
        artifact = self.agent.path('work', result['full_output'])
        self.agent.program_test_cache[key] = dict(signature=digest,
            state=self.agent.accepted_test_state(key), result=result,
            artifact=hashlib.sha256(artifact.read_bytes()).hexdigest())
        with mock.patch.object(host, 'run_parser', wraps=host.run_parser) as run_parser:
            with self.assertRaisesRegex(ValueError, 'bindings are empty or missing'):
                self.run_script('second')
        self.assertEqual(run_parser.call_count, 1)
        self.assertNotIn(source, self.agent.parts)
        self.assertEqual(self.agent.last_script_failure['category'], 'speaker_metadata')

    def test_project_memory_is_local_even_when_shared_history_has_the_same_project(self):
        scripts = self.skill / 'scripts'
        scripts.mkdir()
        memory_source = pathlib.Path(__file__).resolve().parents[4] / 'Galgear-GalgameReverse-Skill/GalgameResourceExtract/scripts/memory.py'
        (scripts / 'memory.py').write_bytes(memory_source.read_bytes())
        shared = self.root / 'shared-memory'
        shared.mkdir()
        project = 'galgear-' + hashlib.sha256(str(self.root).encode()).hexdigest()[:16]
        marker = 'PRIOR_SHARED_FORMAT_MUST_NOT_BE_IMPORTED'
        shared_file = shared / 'memory.json'
        shared_file.write_text(json.dumps(dict(schema_version=1, last_project=project,
            projects={project: dict(game_root=str(self.root), output_dir=str(self.agent.work), notes=marker)})),
            encoding='utf-8')
        shared_before = shared_file.read_bytes()
        with mock.patch.dict('os.environ', {'GAL_EXTRACT_HOME': str(shared)}):
            configured = self.agent.dispatch('project_memory', dict(action='configure'))
            self.assertEqual(configured['exit_code'], 0)
            self.agent.dispatch('project_memory', dict(action='update', patch={'counts': {'chapters': 3}}))
            result = self.agent.dispatch('project_memory', dict(action='read'))
        local_file = self.agent.work / 'project-memory.json'
        self.assertTrue(local_file.is_file())
        self.assertEqual(result['result']['counts'], {'chapters': 3})
        self.assertNotIn(marker, json.dumps(result))
        self.assertNotIn(marker, local_file.read_text('utf-8'))
        self.assertEqual(shared_file.read_bytes(), shared_before)

    def test_read_json_returns_small_objects_directly_and_bounds_large_objects(self):
        small = dict(helper_key='sample-companion', counts=dict(messages=169, named=94),
                     layout=dict(base=342, table_offset=4, encoding='utf-16-le'))
        path = self.agent.work / 'small-result.json'
        path.write_text(json.dumps(small), encoding='utf-8')
        result = self.agent.dispatch('read_json', dict(root='work', path=path.name))
        self.assertEqual(result['value'], small)
        self.assertNotIn('keys', result)
        pointed = self.agent.dispatch('read_json', dict(root='work', path=path.name, pointer='/layout'))
        self.assertEqual(pointed['value'], small['layout'])

        large = dict(large_string='字' * 3000, layout=small['layout'])
        path.write_text(json.dumps(large, ensure_ascii=False), encoding='utf-8')
        bounded = self.agent.dispatch('read_json', dict(root='work', path=path.name))
        self.assertNotIn('value', bounded)
        self.assertEqual(set(bounded['keys']), set(large))
        self.assertEqual(bounded['total_keys'], len(large))
        self.assertEqual(self.agent.dispatch('read_json', dict(root='work', path=path.name,
                         pointer='/layout'))['value'], small['layout'])

    def test_context_reset_keeps_workflow_action_separate_from_request_budget(self):
        self.agent.context_system = 'Continue preparation from the verified host checkpoint.'
        self.agent.context_task = json.dumps(dict(task='Prepare fixture transcript and saves.'))
        self.agent.reset_context = True
        goal = 'Reuse the verified helper for all remaining scripts, then test the full transcript batch.'
        budget = 'There are 12 requests left; preserve one request for finish.'
        messages = []
        self.agent.prepare_request(messages, dict(phase='prepare', remaining_work_requests=12,
                                   guidance=dict(next_action=goal)), budget)
        state = json.loads(messages[2]['content'])['state']
        self.assertEqual(state['next_action'], goal)
        self.assertEqual(state['request_budget_instruction'], budget)
        self.assertEqual(state['current_request']['remaining_work_requests'], 12)

    def test_script_test_registers_full_archive_coverage_without_companion_metadata(self):
        archive = self.root / 'dialogue.pack'
        archive.write_bytes(b'archive fixture with three independent chapter members')
        self.agent.extracted_from[str(self.root)] = str(archive)
        self.agent.extraction_stamps[str(self.root)] = self.agent.file_fingerprint(archive)
        listing = '\n'.join('[{:08X}] {} {}.lng'.format(index * 32,
                            (self.root / (name + '.lng')).stat().st_size, name)
                            for index, name in enumerate(('first', 'second', 'narration')))
        original_dispatch = self.agent._dispatch

        def dispatch(name, args):
            if name == 'run_skill':
                self.assertEqual(args, dict(command='l', archive=archive.name))
                return dict(exit_code=0, stdout=listing, stderr='')
            return original_dispatch(name, args)

        self.assertEqual(self.agent.speaker_resources, {})
        self.assertEqual(self.agent.required_script_sets, {})
        with mock.patch.object(self.agent, '_dispatch', side_effect=dispatch):
            result = self.run_script()
            self.assertEqual(result['count'], 1)
            coverage = self.agent.preparation_status()['transcript_coverage']
            self.assertEqual(coverage['expected_members'], 3)
            self.assertEqual(coverage['covered_members'], 1)
            self.assertEqual({item['member'] for item in coverage['pending_members']},
                             {'second.lng', 'narration.lng'})
            complete = self.agent.dispatch('test_program_batch', dict(root='game', path='.', filter='*.lng'))
        self.assertTrue(all('error' not in row for row in complete['files']))
        self.assertEqual(self.agent.transcript_coverage()['pending_members'], [])
        self.assertEqual(self.agent.speaker_resources, {})

    def test_failed_parser_and_evidence_probes_do_not_register_an_archive_family(self):
        archive = self.root / 'dialogue.pack'
        archive.write_bytes(b'archive with original and translated fixture members')
        self.agent.extracted_from[str(self.root)] = str(archive)
        self.agent.extraction_stamps[str(self.root)] = self.agent.file_fingerprint(archive)
        listing = '\n'.join('[{:08X}] {} {}'.format(index * 32, path.stat().st_size, path.name)
                            for index, path in enumerate(sorted(self.root.glob('*.*')))
                            if path.suffix in ('.lng', '.ws2'))
        original_dispatch = self.agent._dispatch
        listings_requested = []

        def dispatch(name, args):
            if name == 'run_skill':
                listings_requested.append(args)
                return dict(exit_code=0, stdout=listing, stderr='')
            return original_dispatch(name, args)

        failures = [('native-version parse probe', 'first.ws2',
                     'def extract(data, metadata):\n    raise ValueError("Fixture parse failure")\n'),
                    ('translated evidence failure', 'first.lng',
                     PARSER.replace("'text':text,", "'text':text + ' forged',"))]
        with mock.patch.object(self.agent, '_dispatch', side_effect=dispatch):
            for label, path, source in failures:
                with self.subTest(failure=label):
                    self.agent.dispatch('write_program', dict(kind='script', source=source))
                    with self.assertRaises((RuntimeError, ValueError)):
                        self.agent.dispatch('test_program', dict(kind='script', root='game', path=path))
                    self.assertEqual(self.agent.required_script_sets, {})
                    self.assertEqual(self.agent.parts, {})
                    self.assertEqual(self.agent.transcript_coverage(),
                                     dict(expected_members=0, covered_members=0, pending_members=[]))
        self.assertEqual(listings_requested, [])

    def test_successful_family_without_metadata_survives_later_failed_parser_rewrite(self):
        archive = self.root / 'dialogue.pack'
        archive.write_bytes(b'archive fixture with three translated chapter members')
        self.agent.extracted_from[str(self.root)] = str(archive)
        self.agent.extraction_stamps[str(self.root)] = self.agent.file_fingerprint(archive)
        members = ['first.lng', 'second.lng', 'narration.lng']
        listing = '\n'.join('[{:08X}] {} {}'.format(index * 32,
                            (self.root / name).stat().st_size, name)
                            for index, name in enumerate(members))
        original_dispatch = self.agent._dispatch

        def dispatch(name, args):
            if name == 'run_skill':
                self.assertEqual(args, dict(command='l', archive=archive.name))
                return dict(exit_code=0, stdout=listing, stderr='')
            return original_dispatch(name, args)

        self.assertEqual(self.agent.script_resources, {})
        self.assertEqual(self.agent.speaker_resources, {})
        with mock.patch.object(self.agent, '_dispatch', side_effect=dispatch):
            self.assertEqual(self.run_script()['count'], 1)
            accepted_family = copy.deepcopy(self.agent.required_script_sets)
            self.assertEqual(accepted_family, {str(archive) + '|.lng':
                             dict(archive=str(archive), members=sorted(members))})
            self.agent.dispatch('write_program', dict(kind='script', source=
                'def extract(data, metadata):\n    raise ValueError("Rewritten parser failed")\n'))
            self.assertEqual(self.agent.parts, {})
            self.assertEqual(self.agent.required_script_sets, accepted_family)
            with self.assertRaises(RuntimeError):
                self.run_script()
        self.assertEqual(self.agent.required_script_sets, accepted_family)
        coverage = self.agent.preparation_status()['transcript_coverage']
        self.assertEqual(coverage['expected_members'], 3)
        self.assertEqual(coverage['covered_members'], 0)
        self.assertEqual({item['member'] for item in coverage['pending_members']}, set(members))
        self.assertEqual(self.agent.script_resources, {})
        self.assertEqual(self.agent.speaker_resources, {})


if __name__ == '__main__':
    unittest.main()
