"""Bundled-adapter tools preserve normal source, scheduling and proof boundaries."""
import copy
import hashlib
import importlib.util
import json
import pathlib
import tempfile
import unittest
from unittest import mock


spec = importlib.util.spec_from_file_location('engine_adapter_tool_host', pathlib.Path(__file__).with_name('agent_host.py'))
host = importlib.util.module_from_spec(spec)
spec.loader.exec_module(host)


class EngineAdapterToolTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = pathlib.Path(temporary.name)
        for name in ('game', 'save', 'skill'):
            (self.root / name).mkdir()
        (self.root / 'game/fixture.exe').write_bytes(b'not executed')
        (self.root / 'game/member.bin').write_bytes(b'FORMAT')
        (self.root / 'save/current.sav').write_bytes(b'Current')
        self.agent = host.Agent(dict(game=str(self.root / 'game'), save=str(self.root / 'save'),
            skill=str(self.root / 'skill'), work=str(self.root / 'game/work'), title='Fixture',
            language='en-US', provider='deepseek', model='never-called', max_turns=0,
            required_saves=[str((self.root / 'save/current.sav').resolve())]))
        self.calls = []

        def inspect(kind, data, metadata, profile):
            self.calls.append(dict(kind=kind, data=data, metadata=copy.deepcopy(metadata), profile=copy.deepcopy(profile)))
            if metadata.get('dialogue'):
                metadata['dialogue'][0]['text'] = 'Mutated only inside the trusted library copy'
            profile['changed'] = True
            return dict(format='synthetic', bytes=len(data), rows=len(metadata.get('dialogue', [])))

        def program(kind, profile):
            self.calls.append(dict(program_kind=kind, profile=copy.deepcopy(profile)))
            if kind == 'save' and profile == {'mode': 'raw_observations'}:
                return 'def inspect_save_state(data):\n return {}\n'
            return 'import json\ndef extract(data, metadata):\n return json.loads(' + repr(json.dumps(profile)) + ')\n'

        self.implementation = dict(describe=lambda: dict(profile=dict(variant='synthetic')),
            inspect_script=lambda *args: inspect('script', *args),
            inspect_save=lambda *args: inspect('save', *args), program=program)
        patch = mock.patch.dict(host.ADAPTER_LIBRARY['_LOADED'], entis_cotopha=self.implementation, clear=True)
        patch.start()
        self.addCleanup(patch.stop)

    def reuse(self, kind='script', profile=None):
        return self.agent.dispatch('reuse_engine_adapter', dict(adapter='entis_cotopha', kind=kind,
            profile=dict(variant='synthetic') if profile is None else profile))

    def preflight(self):
        self.agent.dispatch('record_save_preflight', dict(path='current.sav', classification='progress_candidate',
            explanation='Synthetic current caption; final node validation remains required.',
            observations=[dict(role='current_caption', value='Current',
                               proof=dict(offset=0, length=7, encoding='ascii'))]))

    def test_catalog_and_compact_contract_use_fixed_registry_ids(self):
        catalog = self.agent.dispatch('engine_adapter_contract', {})
        self.assertEqual(catalog['adapters'][0]['id'], 'entis_cotopha')
        description = self.agent.dispatch('engine_adapter_contract', dict(adapter='entis_cotopha'))
        self.assertEqual(description['adapter_version'], 1)
        self.assertEqual(description['contract']['profile']['variant'], 'synthetic')
        self.assertLessEqual(len(json.dumps(description, ensure_ascii=False).encode()), 8192)
        self.assertEqual(self.agent.programs, {})
        self.assertEqual(self.agent.tests, {})

    def test_raw_save_reuse_requires_a_sandboxed_semantic_entry(self):
        args = dict(adapter='entis_cotopha', kind='save', profile=dict(mode='raw_observations'))
        with self.assertRaisesRegex(ValueError, 'entry_source'):
            self.agent.dispatch('reuse_engine_adapter', args)
        with self.assertRaises(ValueError):
            self.agent.dispatch('reuse_engine_adapter', dict(args, entry_source='import os\ndef extract(data, metadata): return {}'))
        self.assertEqual(self.agent.programs, {})
        entry = 'def extract(data, metadata):\n return {"source":"","line":0,"anchors":[],"warnings":[]}'
        self.agent.dispatch('verification_contract', {})
        with self.assertRaisesRegex(ValueError, 'must call state = inspect_save_state'):
            self.agent.dispatch('reuse_engine_adapter', dict(args, entry_source=entry))
        self.assertEqual(self.agent.programs, {})
        entry = 'def extract(data, metadata):\n state = inspect_save_state(data)\n return {"source":"","line":0,"anchors":[],"warnings":[]}'
        self.agent.dispatch('reuse_engine_adapter', dict(args, entry_source=entry))
        self.assertTrue(self.agent.programs['save'].endswith(entry))
        self.assertEqual(self.agent.tests, {})

    def test_contract_rejects_oversized_description_instead_of_hiding_required_fields(self):
        self.implementation['describe'] = lambda: dict(schema='x' * 8192)
        with self.assertRaisesRegex(ValueError, '8192-byte'):
            self.agent.dispatch('engine_adapter_contract', dict(adapter='entis_cotopha'))

    def test_stationary_search_cursor_is_diagnosed_before_execution(self):
        source = 'def extract(data, metadata):\n o = data.find(b"x")\n while o >= 0:\n  data.find(b"y", o)\n return {}'
        with self.assertRaisesRegex(ValueError, 'line 3 never updates counter o'):
            host.validate_program(source)
        for repair in ('o = data.find(b"x", o + 1)', 'o -= 1', 'break', 'return {}', 'raise ValueError("missing field")'):
            host.validate_program(source.replace('data.find(b"y", o)', repair))
        host.validate_program('def extract(data, metadata):\n items = [1]\n while items:\n  items.pop()\n return {}')

    def test_incomplete_replacement_keeps_the_saved_decoder_and_validation(self):
        self.agent.dispatch('verification_contract', {})
        entry = 'def extract(data, metadata):\n return inspect_save_state(data)'
        with self.assertRaisesRegex(ValueError, 'Complete save source must define inspect_save_state'):
            self.agent.dispatch('write_program', dict(kind='save', source=entry))
        self.assertEqual(self.agent.programs, {})
        original = 'def inspect_save_state(data):\n return {}\ndef extract(data, metadata):\n return inspect_save_state(data)'
        self.agent.dispatch('write_program', dict(kind='save', source=original))
        self.agent.tests['existing'] = {'verified': True}
        with self.assertRaisesRegex(ValueError, 'Complete save source must define inspect_save_state'):
            self.agent.dispatch('write_program', dict(kind='save', source=entry))
        self.assertEqual(self.agent.programs['save'], original)
        self.assertEqual((self.agent.work / 'save_extractor.py').read_text(encoding='utf-8'), original)
        self.assertEqual(self.agent.tests, {'existing': {'verified': True}})

    def test_adapter_paths_and_unknown_ids_cannot_select_external_code(self):
        for adapter in ('../outside.py', 'C:\\outside.py', 'entis_adapter.py', 'unknown'):
            for name, arguments in (
                ('engine_adapter_contract', dict(adapter=adapter)),
                ('inspect_engine_adapter', dict(adapter=adapter, kind='script', root='game', path='member.bin')),
                ('reuse_engine_adapter', dict(adapter=adapter, kind='script', profile={})),
            ):
                with self.subTest(adapter=adapter, tool=name), self.assertRaisesRegex(ValueError, 'Unknown bundled'):
                    self.agent.dispatch(name, arguments)
        self.assertEqual(self.calls, [])

    def test_save_inspection_receives_normal_metadata_and_dialogue_list_without_mutating_it(self):
        row = dict(source='member.bin', position=0, text='Current', speaker=None)
        self.agent.parts['fixture-script'] = [row]
        self.agent.script_resources['global-values'] = dict(format='synthetic')
        self.agent.speaker_resources['global-values'] = {'entry': dict(id=1, text='Synthetic name')}
        before = copy.deepcopy(self.agent.parts)
        profile = dict(variant='synthetic')
        result = self.agent.dispatch('inspect_engine_adapter', dict(adapter='entis_cotopha', kind='save',
            root='save', path='current.sav', profile=profile))
        metadata = self.calls[0]['metadata']
        self.assertEqual(metadata['dialogue'], [row])
        self.assertIsInstance(metadata['dialogue'], list)
        self.assertEqual(metadata['resources'], self.agent.script_resources)
        self.assertEqual(metadata['speaker_resources'], self.agent.speaker_resources)
        self.assertEqual(self.agent.parts, before)
        self.assertEqual(profile, dict(variant='synthetic'))
        self.assertTrue(result['analysis_only'])
        self.assertFalse(result['verified_current_node'])
        self.assertEqual(self.agent.tests, {})
        self.assertEqual(self.agent.validated_saves(), {})

    def test_inspection_uses_same_root_path_stamp_and_durable_research_location(self):
        arguments = dict(adapter='entis_cotopha', kind='script', root='game', path='member.bin')
        self.assertEqual(self.agent.input_description('inspect_engine_adapter', arguments),
                         self.agent.input_description('inspect_data', arguments))
        result = self.agent.dispatch('inspect_engine_adapter', arguments)
        reference = self.agent.research.observe('synthetic-inspect',
            dict(name='inspect_engine_adapter', arguments=json.dumps(arguments)), result,
            inputs=self.agent.input_description('inspect_engine_adapter', arguments))
        record = self.agent.research.records[reference['ref']]
        self.assertEqual(record['inputs'][0]['path'], 'member.bin')
        self.assertEqual(record['preview_target_pointer'], '/result/inspection')
        self.assertIn('inspect_engine_adapter', host.RESEARCH['STRUCTURAL_TOOLS'])

    def test_unsupported_save_guidance_survives_reads_and_restart_until_input_changes(self):
        import test_entis_adapter as fixtures
        data, unused = fixtures.fixture(version=7)
        path = self.root / 'save/current.sav'
        path.write_bytes(data)
        self.implementation['inspect_save'] = fixtures.ADAPTER['inspect_save']
        arguments = dict(adapter='entis_cotopha', kind='save', root='save', path='current.sav')
        result = self.agent.dispatch('inspect_engine_adapter', arguments)
        self.assertFalse(result['ok'])
        self.assertFalse(host.RESEARCH['successful'](result))
        self.agent.research.observe('unsupported-save', dict(name='inspect_engine_adapter', arguments=arguments),
            result, inputs=self.agent.input_description('inspect_engine_adapter', arguments))
        for index in range(8):
            self.agent.research.observe('script-' + str(index),
                dict(name='inspect_engine_adapter', arguments=dict(adapter='entis_cotopha', kind='script')),
                dict(inspection=dict(function_count=2)))
        restored = host.Agent(self.agent.config)
        guidance = restored.work_guidance()
        self.assertEqual(len(guidance['adapter_fallbacks']), 1)
        self.assertEqual(guidance['adapter_fallbacks'][0]['observed_title']['version'], 7)
        self.assertIn('First inspect the current required save', guidance['next_action'])
        restored.workflow.stage = 'save_program'
        with mock.patch.object(restored.workflow, 'describe', return_value=dict(goal='Write the save parser.', exploratory_requests=0)):
            self.assertIn('write_program(kind="save")', restored.work_guidance()['next_action'])
        self.assertFalse(restored.preparation_status()['ready_for_finish'])
        self.assertEqual(restored.programs, {})
        path.write_bytes(fixtures.fixture()[0])
        restored.research.set_inputs(restored.input_description('inspect_engine_adapter', arguments))
        self.assertEqual(restored.work_guidance()['adapter_fallbacks'], [])

    def test_stamped_adapter_inspection_can_support_speaker_review_with_real_inventory(self):
        self.agent.parts['fixture-script'] = [dict(source='member.bin', position=0, text='Current', speaker=None)]
        refs = []
        for serial, (name, arguments) in enumerate((
            ('list_files', dict(root='game', path='')),
            ('inspect_engine_adapter', dict(adapter='entis_cotopha', kind='script', root='game', path='member.bin')),
        )):
            result = self.agent.dispatch(name, arguments)
            observed = self.agent.research.observe('speaker-review-' + str(serial),
                dict(name=name, arguments=json.dumps(arguments)), result,
                inputs=self.agent.input_description(name, arguments))
            refs.append(observed['ref'])
        result = self.agent.dispatch('review_speakers', dict(reason='Synthetic inspected format has no recoverable displayed-name field.',
                                                           evidence_refs=refs))
        self.assertFalse(result['speaker_review_required'])

    def test_non_json_profiles_and_oversized_inspections_are_rejected(self):
        for profile in (dict(callback=lambda: None), dict(number=float('nan'))):
            with self.subTest(profile=profile), self.assertRaisesRegex(ValueError, 'ordinary JSON'):
                self.agent.dispatch('inspect_engine_adapter', dict(adapter='entis_cotopha', kind='script',
                    root='game', path='member.bin', profile=profile))
            with self.subTest(reuse_profile=profile), self.assertRaisesRegex(ValueError, 'ordinary JSON'):
                self.reuse(profile=profile)
        self.implementation['inspect_script'] = lambda *args: dict(value='x' * 40000)
        with self.assertRaisesRegex(ValueError, '40000 UTF-8'):
            self.agent.dispatch('inspect_engine_adapter', dict(adapter='entis_cotopha', kind='script',
                root='game', path='member.bin'))
        self.assertEqual(self.agent.programs, {})

    def test_reuse_saves_complete_source_and_never_calls_test_or_finish(self):
        self.agent.read_contract = True
        with mock.patch.object(host, 'run_parser', side_effect=AssertionError('Reuse must not execute a parser')):
            result = self.reuse(profile=dict(variant='synthetic', label='"; import os; #'))
        source = (self.agent.work / 'script_extractor.py').read_text('utf-8')
        digest = hashlib.sha256(source.encode()).hexdigest()
        self.assertEqual(result['source_sha256'], digest)
        self.assertEqual(result['sha256'], digest)
        self.assertEqual((result['adapter_id'], result['adapter_version']), ('entis_cotopha', 1))
        self.assertEqual(result['requires_test'], 'script')
        self.assertEqual(self.agent.tests, {})
        self.assertEqual(self.agent.parts, {})
        self.assertIsNone(self.agent.done)
        self.assertNotIn('adapter_library', source)
        self.assertNotIn('galgear_adapters', source)
        self.implementation['program'] = lambda *args: 'def extract(data, metadata): return {"changed":True}'
        parsed = host.run_parser(source, b'raw', {})
        self.assertEqual(parsed, dict(variant='synthetic', label='"; import os; #'))

    def test_reuse_preserves_write_program_contract_ast_and_resource_selection_guards(self):
        with self.assertRaisesRegex(ValueError, 'Read verification_contract'):
            self.reuse()
        self.agent.read_contract = True
        self.agent.config['inventory'] = [dict(path=str(self.root / 'game/member.bin'))]
        with self.assertRaisesRegex(ValueError, 'Select the actual UI-language'):
            self.reuse()
        self.agent.config['inventory'] = []
        self.implementation['program'] = lambda *args: 'import adapter_library\ndef extract(data, metadata): return []'
        with self.assertRaisesRegex(ValueError, 'Only these pure modules'):
            self.reuse()
        self.assertFalse((self.agent.work / 'script_extractor.py').exists())

    def test_reuse_save_invalidates_old_tests_while_preserving_transcript(self):
        self.agent.read_contract = True
        self.agent.parts = {'fixture': [dict(source='member.bin', position=0, text='Current', speaker=None)]}
        self.agent.tests['old-save'] = dict(coverage='node')
        before = copy.deepcopy(self.agent.parts)
        result = self.reuse(kind='save')
        self.assertEqual(result['invalidated_save_tests'], 1)
        self.assertEqual(self.agent.tests, {})
        self.assertEqual(self.agent.parts, before)

    def test_reuse_is_a_program_write_for_preflight_pending_tests_and_exploration(self):
        arguments = dict(adapter='entis_cotopha', kind='script', profile={})
        with self.assertRaises(host.PREFLIGHT['PreflightRequired']):
            self.agent.check_model_tool('reuse_engine_adapter', arguments)
        self.preflight()
        self.agent.read_contract = True
        self.agent.check_model_tool('reuse_engine_adapter', arguments)
        result = self.agent.dispatch('reuse_engine_adapter', arguments)
        self.agent.workflow.observe('reuse_engine_adapter', result, 1, self.agent.preparation_status(), arguments)
        self.assertEqual(self.agent.workflow.pending_validation, {'script'})
        with self.assertRaisesRegex(ValueError, 'Parser changed'):
            self.agent.check_model_tool('reuse_engine_adapter', arguments)
        self.agent.workflow.observe('inspect_engine_adapter', dict(analysis_only=True), 2,
                                    self.agent.preparation_status(), dict(root='game', path='member.bin'))
        self.assertIn(2, self.agent.workflow.inspection_turns)

    def test_save_adapter_inspection_remains_available_through_an_unrelated_script_gate(self):
        self.agent.workflow.stage = 'resources'
        self.agent.workflow.inspection_turns = set(range(8))
        self.agent.workflow.pending_validation = {'script'}
        self.agent.check_model_tool('inspect_engine_adapter', dict(adapter='entis_cotopha', kind='save',
            root='save', path='current.sav'))

    def test_initial_task_advertises_adapters_without_model_requests(self):
        with self.assertRaisesRegex(RuntimeError, 'budget exhausted'):
            self.agent.run_session()
        task = json.loads(self.agent.context_task)
        self.assertEqual(task['engine_adapters'][0]['id'], 'entis_cotopha')
        self.assertIn('inspect_engine_adapter and reuse_engine_adapter', self.agent.context_system)
        self.assertEqual(self.agent.requests_sent, 0)


if __name__ == '__main__':
    unittest.main()
