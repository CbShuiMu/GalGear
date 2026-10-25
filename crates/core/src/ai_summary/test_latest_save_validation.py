"""Default preparation validates the latest target, retaining older audit data."""
import copy
import json
import os
import unittest
from unittest import mock

import test_agent_host as fixtures

host = fixtures.host


class LatestSaveValidationTests(unittest.TestCase):
    def setUp(self):
        self.fixture = fixtures.PreparationLifecycleTests()
        self.fixture.setUp()
        self.addCleanup(self.fixture.doCleanups)
        self.agent = self.fixture.agent
        self.agent.config.pop('save_samples')
        self.old = self.fixture.saves / 'one.sav'
        self.latest = self.fixture.saves / 'two.sav'
        os.utime(self.old, ns=(1_000_000_000, 1_000_000_000))
        os.utime(self.latest, ns=(2_000_000_000, 2_000_000_000))

    def require_latest(self):
        self.agent.config['required_saves'] = [str(self.latest)]
        self.agent.required = {str(self.latest)}
        self.agent.samples[str(self.latest)] = 'progress'

    def test_default_finishes_after_one_current_save(self):
        self.require_latest()
        self.fixture.validate_save(self.latest.name)
        self.assertEqual(self.agent.preparation_status()['required_saves'], 1)
        self.assertTrue(self.agent.preparation_status()['ready_for_finish'])
        result = self.agent.dispatch('finish', self.fixture.finish)
        self.assertEqual(list(result['validated_saves']), [str(self.latest)])

    def test_old_checkpoint_failures_remain_auditable_without_blocking_current_target(self):
        self.require_latest()
        self.old.write_bytes(b'Invalid old save')
        with self.assertRaises((ValueError, RuntimeError)):
            self.fixture.validate_save(self.old.name)
        failure = copy.deepcopy(self.agent.last_save_failure)
        self.fixture.validate_save(self.latest.name)
        self.fixture.save_checkpoint()
        saved = json.loads((self.agent.work / 'checkpoint.json').read_text('utf-8'))
        restored = host.Agent(self.agent.config)
        restored.programs = dict(self.agent.programs)
        self.assertTrue(restored.restore_checkpoint(saved))
        self.assertEqual(restored.last_save_failure, failure)
        self.assertEqual(restored.samples[str(self.old)], 'progress')
        status = restored.preparation_status()
        self.assertTrue(status['ready_for_finish'])
        self.assertFalse(status['save_failure_required'])
        self.assertEqual(status['pending_samples'], [])
        self.assertEqual(status['validation_targets'], [str(self.latest)])
        self.assertEqual(restored.work_guidance()['stage'], 'finish')

    def test_old_success_cannot_replace_a_failed_current_target(self):
        self.require_latest()
        self.fixture.validate_save(self.old.name)
        self.latest.write_bytes(b'Invalid latest save')
        with self.assertRaises((ValueError, RuntimeError)):
            self.fixture.validate_save(self.latest.name)
        status = self.agent.preparation_status()
        self.assertFalse(status['ready_for_finish'])
        self.assertEqual(status['validated_saves'], 0)
        self.assertTrue(status['save_failure_required'])
        self.assertEqual(status['pending_targets'], [str(self.latest)])
        self.assertEqual(self.agent.work_guidance()['stage'], 'save_validation')
        self.assertIn(self.latest.name, self.agent.work_guidance()['next_action'])

    def test_custom_suffix_uses_newest_identified_progress_save_even_when_older_test_passes_later(self):
        old = self.old.rename(self.old.with_suffix('.custom'))
        latest = self.latest.rename(self.latest.with_suffix('.custom'))
        latest.write_bytes(b'Invalid latest save')
        os.utime(latest, ns=(2_000_000_000, 2_000_000_000))
        with self.assertRaises((ValueError, RuntimeError)):
            self.fixture.validate_save(latest.name)
        self.fixture.validate_save(old.name)
        status = self.agent.preparation_status()
        self.assertEqual(status['validation_targets'], [str(latest)])
        self.assertFalse(status['ready_for_finish'])
        self.assertEqual(status['validated_saves'], 0)
        self.assertTrue(status['save_failure_required'])
        self.assertIn(latest.name, self.agent.work_guidance()['next_action'])

    def test_default_successful_sample_scope_excludes_old_validated_saves(self):
        self.fixture.validate_save(self.old.name)
        self.fixture.validate_save(self.latest.name)
        self.assertEqual(len(self.agent.validated_saves()), 2)
        self.assertEqual(self.agent.preparation_status()['validated_saves'], 1)
        result = self.agent.dispatch('finish', self.fixture.finish)
        self.assertEqual(list(result['validated_saves']), [str(self.latest)])
        self.assertEqual(result['rejected_samples'], [])
        self.assertIn(str(self.old), self.agent.tests)

    def test_legacy_two_sample_result_is_reused_with_only_current_target(self):
        self.agent.config['save_samples'] = 2
        self.fixture.validate_save(self.old.name)
        self.fixture.validate_save(self.latest.name)
        old_result = self.agent.dispatch('finish', self.fixture.finish)
        self.assertEqual(len(old_result['validated_saves']), 2)
        self.fixture.save_checkpoint()
        config = dict(self.agent.config, save_samples=1, required_saves=[str(self.latest)])
        restored = host.Agent(config)
        with mock.patch.object(restored, 'request_completion', side_effect=AssertionError('Unexpected model request')) as request:
            result = restored.run()
        request.assert_not_called()
        self.assertEqual(list(result['validated_saves']), [str(self.latest)])
        self.assertEqual(result['rejected_samples'], [])
        self.assertEqual(len(restored.tests), 2)
        self.assertEqual(json.loads((restored.work / 'result.json').read_text('utf-8'))['validated_saves'],
                         result['validated_saves'])


if __name__ == '__main__':
    unittest.main()
