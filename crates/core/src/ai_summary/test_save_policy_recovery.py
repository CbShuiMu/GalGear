"""Save proof upgrades must not discard already verified script preparation."""
import copy
import importlib.util
import json
import pathlib
import tempfile
import unittest
from unittest import mock

SPEC = importlib.util.spec_from_file_location('save_policy_host', pathlib.Path(__file__).with_name('agent_host.py'))
host = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(host)


class SavePolicyRecoveryTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = pathlib.Path(temporary.name)
        (self.root / 'game.exe').write_bytes(b'fixture')
        self.script = self.root / 'chapter.txt'
        self.script.write_bytes(b'Current')
        self.save = self.root / 'slot.sav'
        self.save.write_bytes(b'Current')
        self.config = dict(game=str(self.root), save=str(self.root), skill=str(self.root),
                           work=str(self.root / 'work'), language='en-US',
                           title='Synthetic', provider='fixture', model='fixture', max_turns=2,
                           required_saves=[str(self.save)], save_samples=1)
        self.agent = host.Agent(self.config)
        self.agent.read_skill = self.agent.read_contract = self.agent.used_skill = True
        for kind in ('script', 'save'):
            source = 'def extract(data, metadata):\r\n return []\r\n'
            self.agent.programs[kind] = source
            (self.agent.work / (kind + '_extractor.py')).write_bytes(source.encode())
        self.agent.parts[str(self.script)] = [dict(source='chapter.txt', position=0, text='Current', speaker='Name')]
        self.agent.speaker_parts[str(self.script)] = dict(inline=1, external=0, narration=0, scope_version=2)
        self.agent.sources.add(str(self.script))
        self.agent.write_dialogue()
        self.agent.samples[str(self.save)] = 'progress'
        self.agent.tests[str(self.save)] = dict(facts=dict(source='chapter.txt', line=0, anchors=['Current']),
            matching_anchors=['Current'], coverage='node', sha256=host.hashlib.sha256(self.save.read_bytes()).hexdigest(),
            save_verification_version=host.SAVE_VERIFICATION_VERSION)

    def checkpoint(self):
        self.agent.checkpoint([dict(role='system', content='Synthetic'), dict(role='user', content='{}')])
        return json.loads((self.agent.work / 'checkpoint.json').read_text('utf-8'))

    def restore(self, saved):
        restored = host.Agent(self.config)
        restored.programs = dict(self.agent.programs)
        self.assertTrue(restored.restore_checkpoint(copy.deepcopy(saved)))
        return restored

    def test_old_save_policy_retests_slots_without_discarding_verified_transcript(self):
        saved = self.checkpoint()
        saved.pop('save_verification_version')
        restored = self.restore(saved)
        self.assertEqual(restored.parts, saved['parts'])
        self.assertEqual(restored.speaker_parts, saved['speaker_parts'])
        self.assertEqual(restored.programs, self.agent.programs)
        self.assertEqual(restored.tests, {})
        self.assertEqual(restored.preparation_status()['pending_samples'], [str(self.save)])
        self.assertFalse(restored.preparation_status()['ready_for_finish'])
        self.assertTrue(restored.save_validation_invalidated)
        self.assertFalse(restored.read_contract)

    def test_current_policy_preserves_unchanged_save_checks(self):
        saved = self.checkpoint()
        restored = self.restore(saved)
        self.assertEqual(restored.tests, saved['tests'])
        self.assertEqual(set(restored.validated_saves()), {str(self.save)})
        self.assertTrue(restored.preparation_status()['ready_for_finish'])
        self.assertFalse(restored.save_validation_invalidated)

    def test_individual_old_save_check_cannot_inherit_a_new_checkpoint_version(self):
        saved = self.checkpoint()
        saved['tests'][str(self.save)].pop('save_verification_version')
        restored = self.restore(saved)
        self.assertEqual(restored.tests, {})
        self.assertEqual(restored.parts, saved['parts'])
        self.assertFalse(restored.preparation_status()['ready_for_finish'])

    def test_unversioned_in_memory_check_cannot_be_promoted_by_finish(self):
        self.agent.tests[str(self.save)].pop('save_verification_version')
        self.assertEqual(self.agent.validated_saves(), {})
        with self.assertRaisesRegex(ValueError, 'current progress saves'):
            self.agent.dispatch('finish', dict(engine='fixture', script_language='en-US', speaker_resolution='inline'))

    def test_save_test_signature_includes_policy_but_script_signature_is_unchanged(self):
        save_args = dict(kind='save', root='save', path=self.save.name)
        script_args = dict(kind='script', root='game', path=self.script.name)
        save_before = self.agent.test_signature(save_args)
        script_before = self.agent.test_signature(script_args)
        with mock.patch.object(host, 'SAVE_VERIFICATION_VERSION', host.SAVE_VERIFICATION_VERSION + 1):
            self.assertNotEqual(self.agent.test_signature(save_args), save_before)
            self.assertEqual(self.agent.test_signature(script_args), script_before)

    def test_old_completed_result_cannot_return_before_new_save_checks(self):
        self.checkpoint()
        old = dict(contract_version=host.CONTRACT_VERSION, language='en-US',
                   script_verification_version=host.SCRIPT_VERIFICATION_VERSION,
                   artifacts=self.agent.artifact_hashes(), source_stamps=self.agent.source_stamps(),
                   speaker_verification=dict(version=1, external_rows=0), inventory=None)
        (self.agent.work / 'result.json').write_text(json.dumps(old), encoding='utf-8')
        prior_parts = copy.deepcopy(self.agent.parts)
        calls = []

        def finish_without_validation(messages, tools):
            calls.append(True)
            return dict(role='assistant', content=None, tool_calls=[dict(id='finish', type='function',
                function=dict(name='finish', arguments=json.dumps(dict(engine='fixture', script_language='en-US', speaker_resolution='inline'))))])

        restored = host.Agent(self.config)
        with mock.patch.object(restored, 'request_completion', side_effect=finish_without_validation):
            with self.assertRaisesRegex(RuntimeError, 'budget exhausted'):
                restored.run()
        self.assertTrue(calls)
        self.assertEqual(restored.parts, prior_parts)
        self.assertEqual(restored.tests, {})
        self.assertFalse((self.agent.work / 'result.json').exists())
        self.assertEqual(len(list(self.agent.work.glob('invalid-result-*.json'))), 1)


if __name__ == '__main__':
    unittest.main()
