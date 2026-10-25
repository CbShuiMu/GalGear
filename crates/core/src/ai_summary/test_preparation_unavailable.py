"""Evidence-backed unavailability and cold preparation isolation; no model API."""
import copy
import json
import pathlib
import tempfile
import unittest
from unittest import mock

import test_agent_host as fixtures


host = fixtures.host


class PreparationUnavailableTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = pathlib.Path(temporary.name)
        (self.root / 'game.exe').write_bytes(b'fixture')
        self.saves = self.root / 'saves'
        self.saves.mkdir()
        self.skill = self.root / 'skill'
        self.skill.mkdir()
        self.config = dict(game=str(self.root), save=str(self.saves), skill=str(self.skill),
                           work=str(self.root / '.galgear-ai-summary' / 'fresh'),
                           title='Unavailable fixture', language='en-US',
                           provider='deepseek', model='test', max_turns=4)
        self.agent = host.Agent(self.config)

    def save(self, name='current.sav', text='Native menu state'):
        path = self.saves / name
        path.write_text(text, encoding='utf-8')
        return path

    def observation(self, path, value='Native menu state'):
        return dict(path=path.name, value=value,
                    proof=dict(offset=0, length=len(value.encode('utf-8')), encoding='utf-8'))

    def report(self, observations, reason='unsupported_format', agent=None):
        return (agent or self.agent).dispatch('report_unavailable', dict(
            reason=reason, explanation='Native state cannot establish a story node.',
            observations=observations))

    def test_required_targets_must_all_have_native_observations(self):
        first, second = self.save(), self.save('also-required.sav')
        self.agent.required = {str(first), str(second)}
        with self.assertRaisesRegex(ValueError, 'required latest save'):
            self.report([self.observation(first)])
        self.assertIsNone(self.agent.unavailable)
        self.assertFalse((self.agent.work / 'unavailable.json').exists())
        result = self.report([self.observation(first), self.observation(second)])
        self.assertEqual(result['status'], 'unavailable')
        self.assertEqual(len(result['observations']), 2)

    def test_wrong_native_value_and_out_of_bounds_proof_are_rejected(self):
        path = self.save()
        self.agent.required = {str(path)}
        wrong_value = self.observation(path)
        wrong_value['value'] = 'Invented menu state'
        wrong_span = self.observation(path)
        wrong_span['proof']['offset'] = path.stat().st_size
        for observation in (wrong_value, wrong_span):
            with self.subTest(proof=observation), self.assertRaises(ValueError):
                self.report([observation])
            self.assertIsNone(self.agent.unavailable)
            self.assertFalse((self.agent.work / 'unavailable.json').exists())

    def test_observation_must_cite_an_actual_save_inside_save_root(self):
        self.save()
        for path in ('missing.sav', '../game.exe'):
            observation = self.observation(pathlib.Path(path))
            observation['path'] = path
            with self.subTest(path=path), self.assertRaises(ValueError):
                self.report([observation])

    def test_empty_save_directory_can_report_missing_progress(self):
        result = self.report([], reason='missing_progress_save')
        self.assertEqual(result['status'], 'unavailable')
        self.assertEqual(result['reason'], 'missing_progress_save')
        persisted = json.loads((self.agent.work / 'unavailable.json').read_text('utf-8'))
        self.assertEqual(persisted, self.agent.unavailable)
        self.assertIsNone(self.agent.done)

    def test_nonempty_save_directory_cannot_report_without_observations(self):
        self.save()
        with self.assertRaisesRegex(ValueError, 'nonempty save directory'):
            self.report([], reason='missing_progress_save')
        self.assertIsNone(self.agent.unavailable)

    def test_verified_current_node_cannot_be_relabelled_unavailable(self):
        fixture = fixtures.PreparationLifecycleTests()
        fixture.setUp()
        self.addCleanup(fixture.doCleanups)
        fixture.validate_save('two.sav')
        self.assertTrue(fixture.agent.current_validated_saves())
        with self.assertRaisesRegex(ValueError, 'verified current progress node'):
            self.report([], agent=fixture.agent)
        self.assertIsNone(fixture.agent.unavailable)

    def test_reason_and_observation_payload_are_bounded(self):
        path = self.save()
        valid = dict(reason='unsupported_format', explanation='No verified node.',
                     observations=[self.observation(path)])
        mutations = [dict(reason='arbitrary_failure'), dict(explanation=''),
                     dict(explanation='x' * 1201), dict(observations=None),
                     dict(observations=valid['observations'] * 33),
                     dict(observations=[dict(path=path.name, proof={}, value='x', guessed=True)])]
        for mutation in mutations:
            with self.subTest(fields=list(mutation)), self.assertRaises(ValueError):
                self.agent.dispatch('report_unavailable', dict(copy.deepcopy(valid), **mutation))
        self.assertIsNone(self.agent.unavailable)

    def test_run_session_closes_unavailable_without_completing_or_calling_model(self):
        def report_during_turn(messages, tools, turn, phase):
            self.agent.requests_sent += 1
            self.report([], reason='missing_progress_save')

        with mock.patch.object(self.agent, 'run_turn', side_effect=report_during_turn) as turn:
            with mock.patch.object(self.agent, 'request_completion',
                                   side_effect=AssertionError('Unexpected model call')) as request:
                with self.assertRaises(host.PreparationUnavailable):
                    self.agent.run()
        request.assert_not_called()
        turn.assert_called_once()
        summary = json.loads((self.agent.work / 'session.json').read_text('utf-8'))
        self.assertEqual(summary['status'], 'unavailable')
        self.assertTrue(summary['closed'])
        self.assertEqual(summary['failure']['kind'], 'missing_progress_save')
        self.assertIsNone(self.agent.done)
        self.assertFalse((self.agent.work / 'result.json').exists())


class ColdPreparationIsolationTests(PreparationUnavailableTests):
    def setUp(self):
        super().setUp()
        self.config['cold_start'] = True
        self.config['work'] = str(self.root / '.galgear-ai-summary' / 'cold-fresh')
        self.agent = host.Agent(self.config)
        self.old = self.root / '.galgear-ai-summary' / 'zh-CN'
        self.old.mkdir()
        (self.old / 'old_parser.py').write_text('old private answer', encoding='utf-8')
        (self.agent.work / 'new_parser.py').write_text('this run only', encoding='utf-8')
        (self.root / 'resource.py').write_text('actual game resource', encoding='utf-8')
        self.save_cache = self.saves / '.galgear-ai-summary'
        self.save_cache.mkdir()
        (self.save_cache / 'old_parser.py').write_text('old private answer', encoding='utf-8')

    def test_cold_paths_block_old_cache_through_game_and_save_roots(self):
        cases = [('game', '.galgear-ai-summary/zh-CN/old_parser.py'),
                 ('game', '.galgear-ai-summary/cold-fresh/new_parser.py'),
                 ('save', '.galgear-ai-summary/old_parser.py')]
        for root, path in cases:
            with self.subTest(root=root, path=path), self.assertRaisesRegex(ValueError, 'Cold preparation'):
                self.agent.dispatch('read_text', dict(root=root, path=path))

    def test_work_root_can_read_only_the_new_runs_own_artifacts(self):
        result = self.agent.dispatch('read_text', dict(root='work', path='new_parser.py'))
        self.assertEqual(result['text'], 'this run only')
        with self.assertRaisesRegex(ValueError, 'outside allowed root'):
            self.agent.path('work', '../zh-CN/old_parser.py')

    def test_cold_constructor_rejects_prior_artifacts_before_writing_any_state(self):
        previous = {path.name: path.read_bytes() for path in self.agent.work.iterdir() if path.is_file()}
        with self.assertRaisesRegex(ValueError, 'fresh empty work directory'):
            host.Agent(self.config)
        self.assertEqual(previous, {path.name: path.read_bytes() for path in self.agent.work.iterdir() if path.is_file()})

    def test_cold_constructor_accepts_existing_empty_work_and_own_checkpointing(self):
        empty = self.root / '.galgear-ai-summary' / 'another-empty-cold'
        empty.mkdir()
        fresh = host.Agent(dict(self.config, work=str(empty)))
        fresh.checkpoint([])
        self.assertTrue((empty / 'checkpoint.json').is_file())
        self.assertEqual(fresh.requests_sent, 0)

    def test_listing_and_recursive_matching_do_not_disclose_old_cache(self):
        for root in ('game', 'save'):
            result = self.agent.dispatch('list_files', dict(root=root, path='.'))
            self.assertNotIn('.galgear-ai-summary', [item['name'] for item in result['entries']])
        self.assertEqual(self.agent.matching_files('game', '.', '*.py'), [self.root / 'resource.py'])
        with self.assertRaisesRegex(ValueError, 'matched no files'):
            self.agent.matching_files('save', '.', '*.py')
        self.assertEqual(self.agent.matching_files('work', '.', '*.py'),
                         [self.agent.work / 'new_parser.py'])


if __name__ == '__main__':
    unittest.main()
