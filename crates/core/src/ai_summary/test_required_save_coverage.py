"""Required current saves cannot silently downgrade to partial coverage."""
import copy
import hashlib
import json
import os
import unittest
from unittest import mock


import test_save_failure_recovery as existing


EMPTY_SAVE = '''def extract(data, metadata):
    return {'source':'','line':0,'anchors':[],
            'warnings':['Native current node remains unresolved']}
'''
SCENE_ONLY_SAVE = '''def extract(data, metadata):
    return {'source':'','line':0,'anchors':[],'scene':'first',
            'warnings':['Scene only; native current node remains unresolved'],
            'evidence':{'scene':{'offset':0,'length':5,'encoding':'ascii'}}}
'''


class RequiredSaveCoverageTests(unittest.TestCase):
    def setUp(self):
        self.fixture = existing.SaveFailureRecoveryTests()
        self.fixture.setUp()
        self.addCleanup(self.fixture.doCleanups)
        self.agent, self.root = self.fixture.agent, self.fixture.root
        self.required = self.root / 'current.sav'

    def write_save(self, source):
        self.agent.dispatch('write_program', dict(kind='save', source=source))

    def run_save(self, name='current.sav', agent=None):
        return (agent or self.agent).dispatch('test_program', dict(kind='save', root='save', path=name))

    def seed_prior_native_failure(self):
        # Seed a previous failed native join, not a model-produced attestation.
        failure = dict(kind='save', root='save', path='current.sav', category='save',
            error='Saved native scope and return address must identify exactly one independently attested script node; matches=0',
            source_sha256=hashlib.sha256(self.agent.programs['save'].encode()).hexdigest(),
            input_sha256=hashlib.sha256(self.required.read_bytes()).hexdigest())
        self.agent.last_save_failure = copy.deepcopy(failure)
        return failure

    def assert_target_rejected(self, name='current.sav', agent=None):
        with self.assertRaisesRegex((ValueError, RuntimeError), '[Rr]equired.*node|current progress.*node'):
            self.run_save(name, agent=agent)

    def test_required_empty_result_is_error_but_stays_in_attempt_audit(self):
        self.write_save(EMPTY_SAVE)
        self.assert_target_rejected()
        record = self.agent.tests[str(self.required)]
        self.assertEqual(record['coverage'], 'unresolved')
        self.assertEqual(record['facts']['anchors'], [])
        self.assertNotIn(str(self.required), self.agent.validated_saves())
        label = hashlib.sha256(str(self.required).encode()).hexdigest()[:12]
        self.assertTrue((self.agent.work / 'tests' / ('save-' + label + '.json')).is_file())
        status = self.agent.preparation_status()
        self.assertTrue(status['save_failure_required'])
        self.assertFalse(status['ready_for_finish'])

    def test_required_scene_only_result_is_also_an_explicit_error(self):
        self.write_save(SCENE_ONLY_SAVE)
        self.assert_target_rejected()
        self.assertEqual(self.agent.tests[str(self.required)]['coverage'], 'scene_only')
        self.assertEqual(self.agent.last_save_failure['category'], 'save')

    def test_non_target_scene_only_remains_a_limitation_and_keeps_target_failure(self):
        prior = self.seed_prior_native_failure()
        self.write_save(SCENE_ONLY_SAVE)
        result = self.run_save('other.sav')
        self.assertEqual(result['coverage'], 'scene_only')
        self.assertIn(str(self.root / 'other.sav'), self.agent.tests)
        self.assertNotIn(str(self.root / 'other.sav'), self.agent.validated_saves())
        self.assertEqual(self.agent.last_save_failure, prior)

    def test_empty_downgrade_keeps_native_failure_in_guidance_and_checkpoint(self):
        prior = self.seed_prior_native_failure()
        self.write_save(EMPTY_SAVE)
        self.assert_target_rejected()
        failure = self.agent.last_save_failure
        self.assertEqual(failure['category'], 'save')
        details = failure['error_details']
        self.assertEqual(details['category'], 'required_current_node')
        self.assertEqual(details['coverage'], 'unresolved')
        self.assertIn('matches=0', details['previous_failure']['error'])
        self.assertEqual(details['previous_failure']['source_sha256'], prior['source_sha256'])
        self.assertIn('matches=0', json.dumps(self.agent.work_guidance(), ensure_ascii=False))
        saved = self.fixture.fixture.checkpoint()
        self.assertEqual(saved['last_save_failure'], failure)
        self.assertEqual(saved['tests'][str(self.required)]['coverage'], 'unresolved')

    def test_repeated_downgrade_does_not_grow_a_recursive_failure_chain(self):
        prior = self.seed_prior_native_failure()
        self.write_save(EMPTY_SAVE)
        for _ in range(3):
            self.assert_target_rejected()
            failure = self.agent.last_save_failure
            previous = failure['error_details']['previous_failure']
            self.assertIn('matches=0', previous['error'])
            self.assertEqual(previous['source_sha256'], prior['source_sha256'])
            self.assertNotIn('previous_failure', previous)
            self.assertLessEqual(len(json.dumps(failure, ensure_ascii=False).encode()), 8000)

    def test_cached_non_target_limitation_is_rechecked_when_it_becomes_required(self):
        self.agent.required = {str(self.root / 'other.sav')}
        self.write_save(SCENE_ONLY_SAVE)
        first = self.run_save()
        self.assertEqual(first['coverage'], 'scene_only')
        self.assertIn(('save', str(self.required)), self.agent.program_test_cache)
        self.agent.required = {str(self.required)}
        self.seed_prior_native_failure()
        self.assert_target_rejected()
        self.assertIn('matches=0', json.dumps(self.agent.last_save_failure, ensure_ascii=False))

    def test_successful_independent_node_still_clears_failure_without_script_regression(self):
        prior_script = copy.deepcopy(dict(parts=self.agent.parts, speakers=self.agent.speaker_resources,
                                         speaker_parts=self.agent.speaker_parts, source=self.agent.programs['script']))
        self.seed_prior_native_failure()
        self.write_save(existing.FIXED_SAVE)
        result = self.run_save()
        self.assertEqual(result['coverage'], 'node')
        self.assertIsNone(self.agent.last_save_failure)
        self.assertIn(str(self.required), self.agent.validated_saves())
        self.assertEqual(dict(parts=self.agent.parts, speakers=self.agent.speaker_resources,
                              speaker_parts=self.agent.speaker_parts, source=self.agent.programs['script']), prior_script)

    def test_default_latest_target_is_strict_without_an_explicit_required_list(self):
        self.agent.required.clear()
        self.agent.config['save_samples'] = 1
        older = self.root / 'other.sav'
        self.agent.samples.update({str(self.required): 'progress', str(older): 'progress'})
        os.utime(older, ns=(1_000_000_000, 1_000_000_000))
        os.utime(self.required, ns=(2_000_000_000, 2_000_000_000))
        self.assertEqual(self.agent.strict_save_validation_targets(), {str(self.required)})
        self.write_save(SCENE_ONLY_SAVE)
        self.assertEqual(self.run_save('other.sav')['coverage'], 'scene_only')
        self.assert_target_rejected()

    def test_legacy_multiple_sample_mode_retains_extra_scene_only_audit(self):
        self.agent.required.clear()
        self.agent.config['save_samples'] = 2
        self.agent.samples.update({str(self.required): 'progress', str(self.root / 'other.sav'): 'progress'})
        self.assertEqual(self.agent.strict_save_validation_targets(), set())
        self.write_save(SCENE_ONLY_SAVE)
        result = self.run_save('other.sav')
        self.assertEqual(result['coverage'], 'scene_only')
        self.assertEqual(self.agent.tests[str(self.root / 'other.sav')]['coverage'], 'scene_only')
        self.assertFalse(self.agent.preparation_status()['ready_for_finish'])

    def test_prior_native_error_marks_changed_save_input_as_history(self):
        prior = self.seed_prior_native_failure()
        self.required.write_bytes(self.required.read_bytes() + b'new bytes')
        self.write_save(EMPTY_SAVE)
        self.assert_target_rejected()
        previous = self.agent.last_save_failure['error_details']['previous_failure']
        self.assertTrue(previous['input_changed'])
        self.assertEqual(previous['input_sha256'], prior['input_sha256'])
        self.assertNotEqual(self.agent.last_save_failure['input_sha256'], prior['input_sha256'])

    def test_cached_verified_node_still_reuses_and_clears_exact_target_failure(self):
        self.write_save(existing.FIXED_SAVE)
        self.assertEqual(self.run_save()['coverage'], 'node')
        self.seed_prior_native_failure()
        with mock.patch.object(existing.host, 'run_parser', side_effect=AssertionError('Cached node should not run parser')):
            result = self.run_save()
        self.assertTrue(result['reused'])
        self.assertEqual(result['coverage'], 'node')
        self.assertIsNone(self.agent.last_save_failure)


if __name__ == '__main__':
    unittest.main()
