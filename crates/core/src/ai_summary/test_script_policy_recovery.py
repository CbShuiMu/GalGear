"""Native scoped-message policy upgrades preserve research and preparation inputs."""
import copy
import importlib.util
import json
import pathlib
import struct
import tempfile
import unittest
from unittest import mock

SPEC = importlib.util.spec_from_file_location('script_policy_host', pathlib.Path(__file__).with_name('agent_host.py'))
host = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(host)

HELPER = '''def extract(data, metadata):
    end = data.index(bytes([0]), 1)
    message = end + 2
    native = int.from_bytes(data[message+1:message+5], 'little')
    name = data[1:end].decode('cp932')
    return {'values':{'native':native}, 'speakers':{'message':{
        'id':native, 'text':name if name else None,
        'id_evidence':{'offset':message+1,'width':4,'endian':'little'},
        'evidence':{'offset':1,'length':end-1 if name else 1,'encoding':'cp932'},
        'native_link':{'kind':'willplus_ws2_name_message_config1_cp932',
                       'name_command_offset':0,'message_command_offset':message}}}}
'''


class ScriptPolicyRecoveryTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = pathlib.Path(temporary.name)
        (self.root / 'game.exe').write_bytes(b'fixture')
        self.source = self.root / 'chapter.ws2'
        self.source.write_bytes(b'\x15Alice\0\0\x14' + struct.pack('<I', 0) + b'\0Original\0\0')
        self.target = self.root / 'chapter.lng'
        self.target.write_bytes(b'translated fixture')
        self.save = self.root / 'slot.sav'
        self.save.write_bytes(b'Current')
        self.config = dict(game=str(self.root), save=str(self.root), skill=str(self.root),
            work=str(self.root / 'work'), language='en-US', title='Synthetic',
            provider='fixture', model='fixture', max_turns=0,
            required_saves=[str(self.save)], save_samples=1)
        self.agent = host.Agent(self.config)
        self.agent.read_skill = self.agent.read_contract = self.agent.used_skill = True
        self.load()
        for kind in ('script', 'save'):
            program = 'def extract(data, metadata): return []'
            self.agent.programs[kind] = program
            (self.agent.work / (kind + '_extractor.py')).write_text(program, encoding='utf-8')
        self.agent.parts[str(self.target)] = [dict(source='chapter.lng', position=0, text='Current', speaker='Alice')]
        self.agent.speaker_parts[str(self.target)] = dict(inline=0, external=1, narration=0,
            scope_version=2, script_verification_version=host.SCRIPT_VERIFICATION_VERSION)
        self.agent.sources.update((str(self.source), str(self.target)))
        self.agent.script_dependency_stamps[str(self.target)] = self.agent.script_dependencies(self.target)
        self.agent.write_dialogue()
        self.agent.tests[str(self.save)] = dict(facts=dict(source='chapter.lng', line=0, anchors=['Current']),
            matching_anchors=['Current'], coverage='node',
            sha256=host.hashlib.sha256(self.save.read_bytes()).hexdigest(),
            save_verification_version=host.SAVE_VERIFICATION_VERSION)

    def load(self, source=HELPER, key='messages', scoped=True):
        args = dict(root='game', path=self.source.name, key=key, source=source)
        if scoped:
            args['target_script'] = dict(root='game', path=self.target.name)
        return self.agent.dispatch('load_script_metadata', args)

    def checkpoint(self):
        self.agent.checkpoint([dict(role='system', content='Synthetic'), dict(role='user', content='{}')])
        return json.loads((self.agent.work / 'checkpoint.json').read_text('utf-8'))

    def restore(self, saved):
        restored = host.Agent(self.config)
        restored.programs = dict(self.agent.programs)
        self.assertTrue(restored.restore_checkpoint(copy.deepcopy(saved)))
        return restored

    def test_scoped_call_requires_native_link_while_global_names_remain_compatible(self):
        unlinked = HELPER.replace("'native_link':{'kind':'willplus_ws2_name_message_config1_cp932',\n                       'name_command_offset':0,'message_command_offset':message}", "'ignored':True")
        with self.assertRaises(host.EVIDENCE['SpeakerMetadataError']):
            self.load(unlinked, key='missing-link')
        self.assertNotIn('missing-link', self.agent.speaker_resource_attestations)
        result = self.load(unlinked, key='global-names', scoped=False)
        self.assertFalse(result['native_link_verified'])
        self.assertEqual(self.agent.speaker_resources['global-names']['message']['text'], 'Alice')
        self.assertNotIn('global-names', self.agent.speaker_resource_attestations)

    def test_successful_scoped_call_mints_exact_host_bound_receipt(self):
        stamp = self.agent.speaker_resource_attestations['messages']
        self.assertEqual(stamp['version'], host.SCRIPT_VERIFICATION_VERSION)
        self.assertEqual(stamp['input_sha256'], host.hashlib.sha256(self.source.read_bytes()).hexdigest())
        self.assertEqual(stamp['helper_sha256'], host.hashlib.sha256(HELPER.encode()).hexdigest())
        self.assertEqual(stamp['target_script'], str(self.target))
        self.assertTrue(self.agent.has_current_native_link_attestation('messages'))
        self.assertEqual(self.agent.pending_native_link_resources(), [])
        self.assertTrue(self.load()['reused'])

    def test_old_checkpoint_only_invalidates_scoped_maps_dependent_parts_and_save_checks(self):
        arguments = dict(root='game', path=self.source.name, offset=0, length=8)
        function = dict(name='read_bytes', arguments=json.dumps(arguments))
        result = self.agent.dispatch('read_bytes', arguments)
        record_path = host.CONTEXT['record_call'](self.agent.work, 'native-layout', function, result)
        observation = self.agent.research.observe('native-layout', function, result, full_record=record_path,
                                                 inputs=self.agent.input_description('read_bytes', arguments))
        self.agent.dispatch('record_findings', dict(findings=[dict(id='message-format', status='observed',
            summary='Confirmed fixture layout', evidence_refs=[observation['ref']])],
            next_steps=['Preserve existing source']))
        notes = copy.deepcopy(self.agent.analysis_notes)
        self.agent.speaker_resources['global-names'] = {'name': dict(id='name', text='Alice')}
        self.agent.script_resources['global-names'] = {'name': 'Alice'}
        self.agent.required_script_sets['fixture-family'] = dict(archive='fixture', members=['chapter.lng'])
        saved = self.checkpoint()
        saved.pop('script_verification_version')
        saved.pop('speaker_resource_attestations')
        restored = self.restore(saved)
        self.assertEqual(restored.parts, {})
        self.assertEqual(restored.tests, {})
        self.assertEqual(restored.speaker_resources['messages'], {})
        self.assertNotIn('messages', restored.script_resources)
        for name in ('metadata_helpers', 'speaker_resource_inputs', 'speaker_resource_targets',
                     'speaker_resource_dependencies', 'required_script_sets', 'sources', 'documents'):
            actual = getattr(restored, name)
            expected = set(saved[name]) if name == 'sources' else saved[name]
            self.assertEqual(actual, expected, name)
        self.assertEqual(restored.programs, self.agent.programs)
        self.assertEqual(restored.speaker_resources['global-names'], saved['speaker_resources']['global-names'])
        for key in ('summary', 'evidence_refs'):
            self.assertEqual(restored.analysis_notes['findings']['message-format'][key],
                             notes['findings']['message-format'][key])
        self.assertIsNotNone(restored.research.resolve(observation['ref']))
        self.assertEqual(restored.analysis_notes['next_steps'], notes['next_steps'])
        self.assertEqual(restored.pending_script_inputs, {str(self.target)})
        repair = restored.preparation_status()['native_link_repair']
        self.assertEqual(repair['next_resource']['key'], 'messages')
        self.assertEqual(repair['next_resource']['input'], saved['speaker_resource_inputs']['messages'])
        self.assertEqual(repair['next_resource']['helper_source']['path'], saved['metadata_helpers']['messages']['path'])
        self.assertFalse(restored.read_contract)
        self.assertFalse(restored.preparation_status()['ready_for_finish'])

    def test_previous_v2_unscoped_null_speaker_parts_require_the_new_independent_script_policy(self):
        self.agent.speaker_resource_targets.clear()
        self.agent.speaker_resources.clear()
        self.agent.speaker_resource_attestations.clear()
        self.agent.parts[str(self.target)][0]['speaker'] = None
        self.agent.write_dialogue()
        saved = self.checkpoint()
        saved['script_verification_version'] = SCRIPT_OLD_VERSION = 2
        self.assertNotEqual(SCRIPT_OLD_VERSION, host.SCRIPT_VERIFICATION_VERSION)
        restored = self.restore(saved)
        self.assertEqual(restored.parts, {})
        self.assertEqual(restored.tests, {})
        self.assertEqual(restored.speaker_parts, {})
        self.assertIn(str(self.target), restored.pending_script_inputs)
        self.assertTrue(restored.script_validation_invalidated)
        self.assertFalse(restored.preparation_status()['ready_for_finish'])
        self.assertEqual(restored.programs, self.agent.programs)
        self.assertEqual(restored.metadata_helpers, saved['metadata_helpers'])
        self.assertEqual(restored.resource_selections, saved['resource_selections'])
        self.assertEqual(restored.required_script_sets, saved['required_script_sets'])

    def test_previous_save2_cannot_keep_current_results_in_a_script3_checkpoint(self):
        saved = self.checkpoint()
        saved['save_verification_version'] = 2
        self.assertEqual(saved['script_verification_version'], 3)
        self.assertEqual(host.SAVE_VERIFICATION_VERSION, 3)
        restored = self.restore(saved)
        self.assertEqual(restored.parts, saved['parts'])
        self.assertEqual(restored.tests, {})
        self.assertFalse(restored.preparation_status()['ready_for_finish'])

    def test_old_result_cannot_keep_parts_accepted_by_a_current_checkpoint(self):
        self.agent.dispatch('finish', dict(engine='Synthetic', script_language='en-US',
            speaker_resolution='external', notes='A fixture for cache migration.'))
        self.checkpoint()
        result_path = self.agent.work / 'result.json'
        result = json.loads(result_path.read_text('utf-8'))
        result['script_verification_version'] = 1
        result_path.write_text(json.dumps(result), encoding='utf-8')
        restored = host.Agent(self.config)
        with mock.patch.object(restored, 'request_completion', side_effect=AssertionError('Unexpected network')):
            with self.assertRaisesRegex(RuntimeError, 'budget exhausted'):
                restored.run()
        self.assertEqual(restored.parts, {})
        self.assertEqual(restored.tests, {})
        self.assertIn(str(self.target), restored.pending_script_inputs)
        self.assertEqual(restored.programs, self.agent.programs)
        self.assertFalse(restored.preparation_status()['ready_for_finish'])

    def test_current_attested_checkpoint_preserves_parts_checks_and_receipt(self):
        saved = self.checkpoint()
        restored = self.restore(saved)
        for name in ('parts', 'tests', 'speaker_resources', 'speaker_resource_attestations', 'metadata_helpers'):
            self.assertEqual(getattr(restored, name), saved[name], name)
        self.assertFalse(restored.script_validation_invalidated)
        self.assertTrue(restored.preparation_status()['ready_for_finish'])

    def test_version_flag_cannot_promote_missing_receipt_or_changed_records(self):
        saved = self.checkpoint()
        for change in ('missing', 'records', 'helper', 'input', 'target'):
            with self.subTest(change=change):
                altered = copy.deepcopy(saved)
                stamp = altered['speaker_resource_attestations']['messages']
                if change == 'missing':
                    altered['speaker_resource_attestations'].clear()
                elif change == 'records':
                    altered['speaker_resources']['messages']['message']['text'] = 'Changed'
                else:
                    stamp[{'helper':'helper_sha256', 'input':'input_sha256', 'target':'target_script'}[change]] = 'changed'
                restored = self.restore(altered)
                self.assertEqual(restored.parts, {})
                self.assertEqual(restored.tests, {})
                self.assertTrue(restored.script_validation_invalidated)

    def test_in_memory_unattested_map_cannot_reach_finish(self):
        self.agent.speaker_resource_attestations.clear()
        with self.assertRaisesRegex(ValueError, 'current progress saves'):
            self.agent.dispatch('finish', dict(engine='fixture', script_language='en-US', speaker_resolution='external'))
        self.assertEqual(self.agent.parts, {})
        self.assertEqual(self.agent.tests, {})
        self.assertEqual(self.agent.pending_script_inputs, {str(self.target)})

    def test_script_and_save_signatures_change_with_script_policy(self):
        for kind, root, path in (('script', 'game', self.target.name), ('save', 'save', self.save.name)):
            args = dict(kind=kind, root=root, path=path)
            before = self.agent.test_signature(args)
            with mock.patch.object(host, 'SCRIPT_VERIFICATION_VERSION', host.SCRIPT_VERIFICATION_VERSION + 1):
                self.assertNotEqual(self.agent.test_signature(args), before)

    def test_old_completed_result_cannot_fast_return_even_with_current_checkpoint(self):
        self.checkpoint()
        result = dict(contract_version=host.CONTRACT_VERSION, save_verification_version=host.SAVE_VERIFICATION_VERSION,
            language='en-US', artifacts=self.agent.artifact_hashes(), source_stamps=self.agent.source_stamps(),
            speaker_verification=dict(version=1, external_rows=1, scope_version=2), inventory=None)
        (self.agent.work / 'result.json').write_text(json.dumps(result), encoding='utf-8')
        restored = host.Agent(self.config)
        with mock.patch.object(restored, 'request_completion', side_effect=AssertionError('No model request allowed')):
            with self.assertRaisesRegex(RuntimeError, 'budget exhausted'):
                restored.run()
        self.assertFalse((restored.work / 'result.json').exists())
        self.assertEqual(len(list(restored.work.glob('invalid-result-*.json'))), 1)
        self.assertEqual(restored.parts, {})
        self.assertEqual(restored.tests, {})
        self.assertEqual(restored.speaker_resource_attestations, {})
        self.assertEqual(restored.programs, self.agent.programs)
        self.assertEqual(restored.metadata_helpers, self.agent.metadata_helpers)
        self.assertIn(str(self.target), restored.pending_script_inputs)

    def test_focused_contract_exposes_required_link_profile(self):
        for topic in ('script', 'speaker'):
            result = self.agent.dispatch('verification_contract', dict(topic=topic))
            self.assertEqual(result['script_verification_version'], host.SCRIPT_VERIFICATION_VERSION)
            self.assertEqual(result['native_message_link'], host.EVIDENCE['NATIVE_MESSAGE_LINK_CONTRACT'])
            self.assertLess(len(json.dumps(result).encode()), host.CONTEXT['MAX_TOOL_BYTES'])


if __name__ == '__main__':
    unittest.main()
