"""Synthetic save failures remain actionable until the exact target succeeds."""
import copy
import hashlib
import importlib.util
import json
import pathlib
import unittest


spec = importlib.util.spec_from_file_location('save_failure_fixture_module', pathlib.Path(__file__).with_name('test_host_repairs.py'))
fixtures = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fixtures)
host = fixtures.host

BROKEN_SAVE = '''def extract(data, metadata):
    row = metadata['dialogue'][0]
    source_end = data.index(b'\\0')
    position_offset = source_end + 1
    text_offset = position_offset + 4
    data = data[text_offset:]
    text = data.decode('ascii').rstrip('|')
    return {'source':row['source'],'line':row['position'],'anchors':[text],
        'evidence':{'source':{'offset':0,'length':source_end,'encoding':'ascii'},
            'position':{'offset':position_offset,'width':4},
            'anchor':{'offset':text_offset,'length':len(data),'encoding':'ascii'}}}
'''
FIXED_SAVE = BROKEN_SAVE.replace("'length':len(data)", "'length':len(text.encode('ascii'))")


class SaveFailureRecoveryTests(unittest.TestCase):
    def setUp(self):
        # Reuse fixture setup only, not its TestCase inheritance/test methods.
        self.fixture = fixtures.HostRepairTests()
        self.fixture.setUp()
        self.addCleanup(self.fixture.doCleanups)
        self.agent, self.root = self.fixture.agent, self.fixture.root
        self.agent.used_skill = True
        self.agent.dispatch('load_script_metadata', dict(root='game', path='first.ws2', key='own-names',
            source=fixtures.HELPER, target_script=dict(root='game', path='first.lng')))
        self.fixture.run_script()
        native_node = b'first\0' + (0).to_bytes(4, 'little')
        (self.root / 'current.sav').write_bytes(native_node + b'Message first|')
        (self.root / 'other.sav').write_bytes(native_node + b'Message first')
        self.fixture.config['required_saves'] = [str(self.root / 'current.sav')]
        self.agent.required.add(str(self.root / 'current.sav'))
        self.agent.dispatch('write_program', dict(kind='save', source=BROKEN_SAVE))

    def run_save(self, path='current.sav', agent=None):
        return (agent or self.agent).dispatch('test_program', dict(kind='save', root='save', path=path))

    def fail_current(self, agent=None):
        agent = agent or self.agent
        with self.assertRaises((ValueError, RuntimeError)) as caught:
            self.run_save(agent=agent)
        failure = copy.deepcopy(agent.last_save_failure)
        self.assertEqual(failure, agent.preparation_status()['last_save_failure'])
        self.assertEqual({key: failure[key] for key in ('kind', 'root', 'path', 'category')},
                         dict(kind='save', root='save', path='current.sav', category='save'))
        self.assertEqual(failure['source_sha256'], hashlib.sha256(agent.programs['save'].encode()).hexdigest())
        self.assertEqual(failure['error'], str(caught.exception)[:5000])
        return failure

    def script_snapshot(self, agent=None):
        agent = agent or self.agent
        return copy.deepcopy(dict(parts=agent.parts, speakers=agent.speaker_resources,
                                  speaker_parts=agent.speaker_parts, script=agent.programs['script']))

    def assert_repair_goal(self, agent=None):
        guidance = (agent or self.agent).work_guidance()
        self.assertEqual(guidance['stage'], 'save_validation')
        self.assertIn('current.sav', guidance['next_action'])
        self.assertIn('program_sources["save"]', guidance['next_action'])
        self.assertIn('edits=', guidance['next_action'])
        self.assertIn('preserve verified script rows, speaker bindings', guidance['next_action'])
        return guidance

    def test_failure_preserves_script_speakers_and_other_accepted_save(self):
        self.run_save('other.sav')
        old_tests, script = copy.deepcopy(self.agent.tests), self.script_snapshot()
        failure = self.fail_current()
        self.assertIn('Current anchor needs exact byte evidence', failure['error'])
        self.assertEqual(self.agent.tests, old_tests)
        self.assertIn(str(self.root / 'other.sav'), self.agent.validated_saves())
        self.assertEqual(self.script_snapshot(), script)
        self.assert_repair_goal()

    def test_checkpoint_and_production_restart_restore_exact_failure_and_source(self):
        failure = self.fail_current()
        script = self.script_snapshot()
        self.assert_repair_goal()
        saved = self.fixture.checkpoint()
        self.assertEqual(saved['last_save_failure'], failure)
        restored = self.fixture.resume_without_model()
        self.assertEqual(restored.last_save_failure, failure)
        self.assertEqual(restored.preparation_status()['last_save_failure'], failure)
        self.assertEqual(restored.programs['save'], BROKEN_SAVE)
        self.assertEqual(self.script_snapshot(restored), script)
        self.assert_repair_goal(restored)

    def test_write_and_different_save_success_do_not_clear_failed_target(self):
        failure, script = self.fail_current(), self.script_snapshot()
        self.agent.dispatch('write_program', dict(kind='save', source=FIXED_SAVE))
        self.assertEqual(self.agent.last_save_failure, failure)
        self.run_save('other.sav')
        self.assertEqual(self.agent.last_save_failure, failure)
        self.assert_repair_goal()
        self.run_save()
        self.assertIsNone(self.agent.last_save_failure)
        self.assertIsNone(self.agent.preparation_status()['last_save_failure'])
        self.assertIn(str(self.root / 'other.sav'), self.agent.validated_saves())
        self.assertEqual(self.script_snapshot(), script)

    def test_failed_retest_updates_source_provenance_without_clearing_failure(self):
        first = self.fail_current()
        still_broken = BROKEN_SAVE.replace("'length':len(data)", "'length':len(data)+1")
        self.agent.dispatch('write_program', dict(kind='save', source=still_broken))
        self.assertEqual(self.agent.last_save_failure, first)
        second = self.fail_current()
        self.assertNotEqual(second['source_sha256'], first['source_sha256'])
        self.assert_repair_goal()

    def test_forced_context_reset_keeps_failure_source_and_exact_target(self):
        failure = self.fail_current()
        self.agent.resource_selections = {f'scene-{i}': dict(selected='fixture.arc', priority='first',
            candidates=[f'archive-{j}-' + 'x' * 100 for j in range(50)]) for i in range(45)}
        self.agent.context_system = 'Synthetic context recovery'
        self.agent.context_task = json.dumps(dict(task='Prepare synthetic bytes'))
        self.agent.audit_messages = []
        self.agent.reset_context = True
        guidance = self.assert_repair_goal()
        messages = []
        self.agent.prepare_request(messages, dict(phase='prepare', guidance=guidance, remaining_work_requests=5), 'Continue')
        state = json.loads(messages[2]['content'])['state']
        self.assertEqual(state['last_save_failure'], failure)
        self.assertEqual(state['program_sources']['save']['source'], BROKEN_SAVE)
        self.assertEqual(state['program_sources']['save']['sha256'], failure['source_sha256'])
        self.assertEqual(state['next_action'], guidance['next_action'])
        self.assertIn('resource_selections', state['section_archives'])
        self.assertLessEqual(len(host.CONTEXT['encode'](messages)), host.CONTEXT['MAX_CONTEXT_BYTES'])

    def test_save_failure_has_an_independent_budget_from_large_script_failure(self):
        source = "def extract(data, metadata): raise ValueError('Save anchor width invalid: ' + 'x' * 10000)"
        self.agent.dispatch('write_program', dict(kind='save', source=source))
        failure = self.fail_current()
        self.assertGreaterEqual(len(failure['error']), 3500)
        self.assertLessEqual(len(failure['error']), 5000)
        script_failure = dict(kind='script', root='game', path='first.lng', category='script',
                              error='Unrelated large script failure ' * 10000, source_sha256='a' * 64)
        state = host.CONTEXT['checkpoint_state'](self.agent.work, dict(
            last_script_failure=script_failure, last_save_failure=failure))
        self.assertEqual(state['last_save_failure'], failure)
        self.assertLessEqual(len(host.CONTEXT['encode'](state['last_save_failure'])), 8000)
        self.assertIn('last_script_failure', state['section_archives'])
        self.assertNotIn('last_save_failure', state['section_archives'])

    def test_after_save_edit_pending_goal_still_names_previously_failed_target(self):
        self.fail_current()
        self.assert_repair_goal()
        arguments = dict(kind='save', edits=[dict(old="'length':len(data)", new="'length':len(text.encode('ascii'))")])
        result = self.agent.dispatch('write_program', arguments)
        self.agent.workflow.observe('write_program', result, 1, self.agent.preparation_status(), arguments=arguments)
        guidance = self.agent.work_guidance()
        self.assertEqual(guidance['stage'], 'save_validation')
        self.assertEqual(guidance['pending_validation'], ['save'])
        self.assertIn('current.sav', guidance['next_action'])
        self.assertIn('kind="save"', guidance['next_action'])


if __name__ == '__main__':
    unittest.main()
