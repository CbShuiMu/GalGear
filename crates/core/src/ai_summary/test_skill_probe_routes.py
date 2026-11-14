"""Offline regression coverage for eager reuse and conservative native routes."""
import hashlib
import json
import pathlib
import runpy
import struct
import sys
import tempfile
import unittest
from unittest import mock

from test_entis_adapter import fixture, header, section, text

HERE = pathlib.Path(__file__).resolve().parent
HOST = runpy.run_path(str(HERE / 'agent_host.py'))
PROBE = HOST['SKILL_PROBE']
ROUTES = HOST['NATIVE_ROUTES']


def slot_fixture(version=20250210):
    return b'BM' + struct.pack('<I', 64) + bytes(58) + fixture(version=version)[0]


class SkillProbeRoutesTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = pathlib.Path(temporary.name)
        for name in ('game', 'save', 'skill/scripts', 'skill/tools/GARbro-cli'):
            (self.root / name).mkdir(parents=True)
        (self.root / 'game/game.exe').write_bytes(b'not executed')
        self.save = self.root / 'save/current.bmp'
        self.save.write_bytes(slot_fixture())
        self.skill = self.root / 'skill'
        # An installed fixed-name synthetic helper, never an executable game file.
        (self.skill / 'scripts/inspect_cotomi_save.py').write_text(
            'def inspect(path, backend):\n'
            ' data=path.read_bytes()\n'
            ' at=data.index(b"context ",int.from_bytes(data[2:6],"little")+64)+16\n'
            ' return {"fixture":True}, data[at+4:]\n', encoding='utf-8')
        (self.skill / 'scripts/parse_cotopha_csx.py').write_text('# synthetic dependency\n', encoding='utf-8')
        (self.skill / 'tools/GARbro-cli/GalArchiveExport.exe').write_bytes(b'not executed')
        self.agent = HOST['Agent'](dict(game=str(self.root / 'game'), save=str(self.root / 'save'),
            skill=str(self.skill), work=str(self.root / 'game/work'), title='Fixture', language='en-US',
            provider='deepseek', model='never-called', max_turns=0, required_saves=[str(self.save)]))

    def test_automatic_probe_survives_unknown_version_without_granting_progress(self):
        probes = self.agent.automatic_probes()
        self.assertEqual(len(probes), 1)
        result = json.loads((self.agent.work / probes[0]['skill_result']).read_text(encoding='utf-8'))['result']
        self.assertEqual(result['raw_title']['fields'][0]['value'], 20250210)
        self.assertFalse(result['verified_current_node'])
        self.assertTrue(pathlib.Path(result['decoded_path']).is_file())
        self.assertEqual(self.agent.programs, {})
        self.assertEqual(self.agent.tests, {})
        self.assertFalse(self.agent.preflight_status()['ready_for_script_sample'])
        self.assertTrue(self.agent.research.adapter_fallbacks())
        # The negative adapter result remains readable at its advertised pointer.
        attempt = self.agent.research.adapter_attempts()[-1]
        self.assertTrue(attempt)
        archived = json.loads((self.agent.work / probes[0]['adapter_result']).read_text(encoding='utf-8'))
        self.assertEqual(archived['result']['inspection']['status'], 'unsupported_variant')
        self.agent.workflow.stage = 'resources'
        self.agent.workflow.pending_validation.add('script')
        self.agent.check_model_tool('read_json', dict(root='work', path=probes[0]['skill_result']))

    def test_cache_reuses_same_bytes_and_invalidates_changed_input_or_tool(self):
        first = self.agent.skill_probes.run(self.save)
        command = [sys.executable, '-I', '-S', str(HERE / 'agent_host.py')]
        probe = PROBE['SkillProbes'](self.skill, self.agent.work, command)
        with mock.patch.object(PROBE['subprocess'], 'run', side_effect=AssertionError('cached decoder rerun')):
            second = probe.run(self.save)
        self.assertTrue(second['reused'])
        self.assertEqual(first['key'], second['key'])
        self.save.write_bytes(slot_fixture(20250211))
        changed = probe.run(self.save)
        self.assertNotEqual(first['key'], changed['key'])
        helper = self.skill / 'scripts/parse_cotopha_csx.py'
        helper.write_text('# updated helper\n', encoding='utf-8')
        changed_tool = probe.run(self.save)
        self.assertNotEqual(changed['key'], changed_tool['key'])
        pathlib.Path(changed_tool['decoded_path']).write_bytes(b'corrupt')
        repaired = probe.run(self.save)
        self.assertFalse(repaired['reused'])
        self.assertEqual(hashlib.sha256(pathlib.Path(repaired['decoded_path']).read_bytes()).hexdigest(), repaired['decoded_sha256'])

    def test_probe_fields_can_be_independently_replayed_for_preflight(self):
        probe = self.agent.skill_save_probe('save', 'current.bmp')
        self.assertEqual(probe['save_template']['profile'], {'mode': 'raw_observations'})
        self.assertIn('state = inspect_save_state(data)', probe['save_template']['entry_prefix'])
        receipt = self.agent.dispatch('record_save_preflight', dict(path='current.bmp',
            classification='progress_candidate', explanation='Synthetic execution state fields',
            observations=probe['preflight_observations']))
        self.assertEqual(receipt['receipt']['classification'], 'progress_candidate')
        self.assertEqual(self.agent.tests, {})
        candidates = probe['primitive_candidates']
        self.assertFalse(candidates['complete_slot_walk'])
        self.assertFalse(candidates['verified_current_node'])
        self.assertTrue(candidates['values'])
        self.assertTrue(all(row['certainty'] == 'candidate' for row in candidates['values']))

    def test_unknown_discovered_save_is_probed_without_becoming_required(self):
        self.agent.required.clear()
        self.agent.samples.clear()
        self.agent.config['save_candidates'] = [dict(path=str(self.save), kind='unknown', source='roaming')]
        probes = self.agent.automatic_probes()
        self.assertEqual(probes[0]['path'], 'current.bmp')
        self.assertIn('skill_result', probes[0])
        self.assertEqual(self.agent.save_validation_targets(), set())
        self.assertEqual(self.agent.programs, {})
        self.assertEqual(self.agent.preflight_targets(), ['current.bmp'])
        self.agent.read_skill = self.agent.read_contract = True
        self.assertEqual(self.agent.work_guidance()['stage'], 'save_preflight')
        with self.assertRaises(ValueError):
            self.agent.check_model_tool('read_bytes', dict(root='game', path='game.exe'))
        # The model can call the same operation without repeating decompression.
        args = dict(root='save', path='current.bmp')
        self.agent.check_model_tool('inspect_skill_save', args)
        result = self.agent.dispatch('inspect_skill_save', args)
        self.assertTrue(result['reused'])
        self.assertFalse(result['verified_current_node'])
        self.assertEqual(result['raw_title']['fields'][0]['value'], 20250210)

    def test_preflight_reuses_exact_current_probe_fields_but_still_replays_bytes(self):
        probe = self.agent.skill_save_probe('save', 'current.bmp')
        args = dict(path='current.bmp', classification='progress_candidate',
                    explanation='Observed execution fields', observations_from_probe=True)
        receipt = self.agent.dispatch('record_save_preflight', args)['receipt']
        self.assertEqual(receipt['observations'], probe['preflight_observations'])
        self.assertFalse(receipt['verified_current_node'])
        self.assertEqual(self.agent.tests, {})
        with self.assertRaisesRegex(ValueError, 'not both'):
            self.agent.dispatch('record_save_preflight', dict(args, observations=probe['preflight_observations']))
        # A stored observation cannot qualify changed bytes or bypass field replay.
        self.save.write_bytes(slot_fixture(20250211))
        with self.assertRaisesRegex(ValueError, 'unchanged Skill probe'):
            self.agent.dispatch('record_save_preflight', args)
        self.agent.skill_save_probe('save', 'current.bmp')
        self.agent.save_probe_fields[str(self.save)]['observations'][0]['value'] = 1
        with self.assertRaisesRegex(ValueError, r'observations\[0\]'):
            self.agent.dispatch('record_save_preflight', args)

    def test_ready_template_bounds_only_post_preflight_save_program_research(self):
        self.agent.automatic_probes()
        self.agent.workflow.stage = 'save_program'
        self.agent.workflow.inspection_turns = {1, 2, 3}
        args = dict(root='save', path='current.bmp', offset=0, length=16)
        self.agent.check_model_tool('read_bytes', args)
        self.agent.dispatch('record_save_preflight', dict(path='current.bmp',
            classification='progress_candidate', explanation='Observed fields', observations_from_probe=True))
        self.agent.workflow.inspection_turns = {4, 5, 6}
        with self.assertRaisesRegex(ValueError, 'Raw save template'):
            self.agent.check_model_tool('read_bytes', args)
        self.agent.workflow.stage = 'save_validation'
        self.agent.check_model_tool('read_bytes', args)

    def test_verified_dialogue_lookup_preserves_ambiguity_and_does_not_validate_save(self):
        rows = [dict(source='chapter', position=i, text='Same caption', speaker='Name',
                     _native=[dict(kind='call_return', scope='scene', value=i + 1)]) for i in range(3)]
        self.agent.parts['fixture'] = rows
        result = self.agent.dispatch('lookup_verified_dialogue', dict(text='Same caption', limit=2))
        self.assertEqual(result['total_matches'], 3)
        self.assertTrue(result['truncated'])
        self.assertFalse(result['verified_current_node'])
        self.assertEqual(result['rows'][0]['_native'], rows[0]['_native'])
        result['rows'][0]['_native'][0]['value'] = 999
        self.assertEqual(rows[0]['_native'][0]['value'], 1)
        self.assertEqual(self.agent.tests, {})
        self.assertEqual(self.agent.dispatch('lookup_verified_dialogue', dict(text='Same caption', source='other'))['rows'], [])
        self.assertEqual(self.agent.dispatch('lookup_verified_dialogue', dict(text='Same'))['rows'], [])
        with self.assertRaises(ValueError):
            self.agent.dispatch('lookup_verified_dialogue', dict(text='', limit=21))

    def test_skill_probe_rejects_other_roots_and_candidates_outside_save(self):
        self.agent.required.clear()
        self.agent.samples.clear()
        other = self.root / 'game/old.bmp'
        other.write_bytes(self.save.read_bytes())
        self.agent.config['save_candidates'] = [dict(path=str(other), kind='unknown'),
                                                dict(path=str(self.save), kind='control')]
        self.assertEqual(self.agent.probe_save_candidates(), [])
        with self.assertRaisesRegex(ValueError, 'root=save'):
            self.agent.dispatch('inspect_skill_save', dict(root='game', path='old.bmp'))
        with self.assertRaises(ValueError):
            self.agent.dispatch('inspect_skill_save', dict(root='save', path='../game/old.bmp'))

    def test_unknown_header_and_missing_skill_do_not_block_model_fallback(self):
        self.save.write_bytes(b'not a Cotomi save')
        self.assertEqual(self.agent.automatic_probes(), [])
        self.save.write_bytes(slot_fixture())
        (self.skill / 'scripts/inspect_cotomi_save.py').unlink()
        result = self.agent.automatic_probes()[0]
        self.assertIn('lacks', result['skill_failure'])
        self.assertTrue(self.agent.research.adapter_fallbacks())

    def test_native_calls_remain_candidates_with_exact_offsets(self):
        image = (b'\x11' + struct.pack('<I', 0) + b'\x00\x04' + struct.pack('<I', 64)).ljust(128, b'\0')
        funcs = struct.pack('<I', 2)
        for name, at in [('chapter', 0), ('ending_candidate', 64)]:
            funcs += struct.pack('<IIII', 0, at, 64, 0) + text(name)
        data = header('Cotopha Image file') + section('image', image) + section('conststr', struct.pack('<I', 0)) + section('funcinfo', funcs)
        facts = ROUTES['cotopha'](data, 'script.csx', [dict(position=80), dict(position=144)])
        call = next(item for item in facts if item.get('kind') == 'call_candidate')
        self.assertEqual(call['position'], 80)
        self.assertEqual(call['target_position'], 144)
        self.assertEqual(call['certainty'], 'candidate')
        self.assertFalse(call['instruction_boundary_verified'])
        self.assertEqual(facts[0]['sha256'], hashlib.sha256(data).hexdigest())
        self.assertEqual(facts[0]['coverage'], 'partial_static_candidates')
        # A native callee without dialogue remains a function, never a proved ending.
        limited = ROUTES['cotopha'](data, 'script.csx', [dict(position=80)])
        target = next(item for item in limited if item.get('name') == 'ending_candidate')
        self.assertFalse(target['has_dialogue'])
        self.assertEqual(target['kind'], 'function')

    def test_reuse_proposal_uses_observed_abi_and_actual_frame_scope_relation(self):
        module = dict(code=bytes(12) + struct.pack('<I', 1),
                      strings=[dict(text='Line'), dict(text='Speaker')],
                      functions=[dict(name='module@scene')])
        sample = dict(scalar_arguments=2, call_arity=6, speaker_argument=True,
                      instruction_position=16, text_index=0)
        inspection = dict(call_abi_candidates=[dict(callee='Dialog::Show', samples=[sample])])
        result = PROBE['script_reuse_candidates'](module, inspection, {'scene'})[0]
        profile = result['arguments']['profile']
        self.assertEqual(profile['scalar_arguments'], 2)
        self.assertEqual(profile['call_arity'], 6)
        self.assertEqual(profile['scope_transform'], dict(kind='last_component', separator='@'))
        self.assertEqual(result['sample_rows'][0]['speaker'], 'Speaker')
        self.assertTrue(result['analysis_only'])
        self.assertNotIn('scope_transform', PROBE['script_reuse_candidates'](module, inspection, {'other'})[0]['arguments']['profile'])
        inspection['call_abi_candidates'][0]['samples'].append(dict(sample, call_arity=7))
        self.assertEqual(PROBE['script_reuse_candidates'](module, inspection, {'scene'}), [])

    def test_export_records_unsupported_inputs_without_dropping_coverage(self):
        path = self.root / 'game/unknown.bin'
        path.write_bytes(b'unknown script')
        sha = ROUTES['export'](self.agent.work, {str(path): []}, pathlib.Path)
        raw = (self.agent.work / 'route-evidence.jsonl').read_bytes()
        self.assertEqual(sha, hashlib.sha256(raw).hexdigest())
        self.assertEqual(json.loads(raw)['coverage'], 'unavailable')


if __name__ == '__main__':
    unittest.main()
