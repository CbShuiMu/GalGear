"""Save-first host scheduling boundaries; synthetic files and no model calls."""
import hashlib
import importlib.util
import pathlib
import struct
import tempfile
import unittest


spec = importlib.util.spec_from_file_location('preflight_host_review', pathlib.Path(__file__).with_name('agent_host.py'))
host = importlib.util.module_from_spec(spec)
spec.loader.exec_module(host)


class SavePreflightHostTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = pathlib.Path(temporary.name)
        for directory in ('game', 'save', 'skill'):
            (self.root / directory).mkdir()
        (self.root / 'game/fixture.exe').write_bytes(b'not executed')
        self.save = self.root / 'save/current.bin'
        self.save.write_bytes(b'HEAD' + struct.pack('<I', 7) + b'STORY')
        self.archive = self.root / 'game/resource.arc'
        self.archive.write_bytes(b'archive format dependency')
        self.config = dict(game=str(self.root / 'game'), save=str(self.root / 'save'),
            skill=str(self.root / 'skill'), work=str(self.root / 'game/work'),
            title='Synthetic preflight host', language='en-US', provider='deepseek', model='never-called',
            max_turns=0, required_saves=[str(self.save.resolve())])
        self.agent = host.Agent(self.config)

    def candidate(self):
        return dict(path='current.bin', classification='progress_candidate',
            explanation='A native state coordinate exists; the final node join is still required.',
            observations=[dict(role='current_coordinate', value=7,
                               proof=dict(offset=4, width=4, endian='little'))])

    def needs_context(self):
        return dict(path='current.bin', classification='needs_context',
            explanation='A resource sample is needed to interpret the saved runtime coordinate.',
            observations=[dict(role='container_header', value='HEAD',
                               proof=dict(offset=0, length=4, encoding='ascii'))],
            context_plan=dict(missing_fact='How does the resource define the saved coordinate?',
                targets=[dict(root='save', path='current.bin'), dict(root='game', path='resource.arc')]))

    def closed_workflow(self):
        self.agent.workflow.stage = 'resources'
        self.agent.workflow.inspection_turns = set(range(1, 10))
        self.agent.workflow.pending_validation = {'script'}

    def record(self, arguments):
        self.agent.check_model_tool('record_save_preflight', arguments)
        return self.agent.dispatch('record_save_preflight', arguments)

    def test_required_save_reads_and_record_remain_available_before_a_pending_script_test(self):
        self.closed_workflow()
        for name in ('list_files', 'read_text', 'read_bytes', 'inspect_data', 'read_json'):
            with self.subTest(tool=name):
                self.agent.check_model_tool(name, dict(root='save', path='current.bin'))
        self.record(self.candidate())
        self.assertTrue(self.agent.preflight_status()['ready_for_script_sample'])
        self.assertFalse(self.agent.preparation_status()['ready_for_finish'])

    def test_preflight_is_required_before_extraction_or_script_work_but_not_readonly_research(self):
        calls = [('run_skill', dict(command='x', archive='resource.arc', filter='*.scn')),
                 ('write_program', dict(kind='script', source='def extract(data, metadata): return []')),
                 ('test_program', dict(kind='script', root='game', path='resource.arc')),
                 ('load_script_metadata', dict(root='game', path='resource.arc', key='sample'))]
        for name, arguments in calls:
            with self.subTest(tool=name), self.assertRaises(host.PREFLIGHT['PreflightRequired']):
                self.agent.check_model_tool(name, arguments)
        self.agent.check_model_tool('inspect_data', dict(root='save', path='current.bin'))
        for command in ('i', 'l'):
            with self.subTest(readonly_command=command):
                self.agent.check_model_tool('run_skill', dict(command=command, archive='resource.arc'))
        self.assertEqual(self.agent.current_validated_saves(), {})
        self.assertFalse(self.agent.preparation_status()['ready_for_finish'])

    def test_needs_context_allows_readonly_research_but_preserves_context_and_mutation_gates(self):
        self.record(self.needs_context())
        self.closed_workflow()
        for name, arguments in (
            ('run_skill', dict(command='l', archive='resource.arc')),
            ('read_bytes', dict(root='game', path='resource.arc')),
            ('read_json', dict(root='game', path='resource.arc')),
            ('inspect_data', dict(root='game', path='resource.arc')),
            ('load_script_metadata', dict(root='game', path='resource.arc', key='sample')),
        ):
            with self.subTest(tool=name):
                self.agent.check_model_tool(name, arguments)
        unrelated = self.root / 'game/unrelated.arc'
        unrelated.write_bytes(b'Independent resource header')
        arguments = dict(root='game', path=unrelated.name, offset=0, length=12)
        self.agent.check_model_tool('read_bytes', arguments)
        result = self.agent.dispatch('read_bytes', arguments)
        self.assertEqual(result['offset'], 0)
        self.assertEqual(self.agent.workflow.pending_validation, {'script'})
        self.assertFalse(self.agent.preflight_status()['ready_for_script_batch'])
        self.assertEqual(self.agent.current_validated_saves(), {})
        self.assertFalse(self.agent.preparation_status()['ready_for_finish'])
        for name, arguments in (
            ('run_skill', dict(command='x', archive=unrelated.name, filter='*.scn')),
            ('write_program', dict(kind='script')),
        ):
            with self.subTest(tool=name), self.assertRaisesRegex(ValueError, 'Parser changed'):
                self.agent.check_model_tool(name, arguments)
        for name in ('test_program_batch', 'load_script_metadata_batch', 'verify_resource_selection_batch', 'finish'):
            with self.subTest(blocked=name), self.assertRaises(host.PREFLIGHT['PreflightRequired']):
                self.agent.check_model_tool(name, {})
        with self.assertRaises(ValueError):
            self.agent.dispatch('read_bytes', dict(root='game', path='../outside.arc', offset=0, length=12))

    def test_needs_context_allows_sample_but_blocks_full_batches_and_finish_until_rechecked(self):
        self.record(self.needs_context())
        for name, arguments in (
            ('write_program', dict(kind='script', source='def extract(data, metadata): return []')),
            ('test_program', dict(kind='script', root='game', path='resource.arc')),
        ):
            with self.subTest(tool=name):
                self.agent.check_model_tool(name, arguments)
        for name in ('test_program_batch', 'load_script_metadata_batch', 'verify_resource_selection_batch', 'finish'):
            with self.subTest(tool=name), self.assertRaises(host.PREFLIGHT['PreflightRequired']):
                self.agent.check_model_tool(name, {})
        self.record(self.candidate())
        self.assertTrue(self.agent.preflight_status()['ready_for_script_batch'])
        for name in ('test_program_batch', 'load_script_metadata_batch', 'verify_resource_selection_batch', 'finish'):
            with self.subTest(tool=name):
                self.agent.check_model_tool(name, {})
        self.assertEqual(self.agent.current_validated_saves(), {})
        self.assertFalse(self.agent.preparation_status()['ready_for_finish'])

    def test_native_unavailable_report_can_stop_before_script_validation(self):
        self.closed_workflow()
        arguments = dict(reason='missing_progress_save', explanation='Synthetic native initialization state.',
            observations=[dict(path='current.bin', proof=dict(offset=0, length=4, encoding='ascii'), value='HEAD')])
        self.agent.check_model_tool('report_unavailable', arguments)
        reply = self.agent.dispatch('report_unavailable', arguments)
        self.assertEqual(reply['status'], 'unavailable')
        self.assertEqual(reply['reason'], 'missing_progress_save')
        self.assertEqual(self.agent.current_validated_saves(), {})
        self.assertFalse((self.agent.work / 'result.json').exists())

    def test_existing_verified_current_nodes_bypass_missing_preflight_on_resume(self):
        name = str(self.save.resolve())
        self.agent.tests[name] = dict(coverage='node', matching_anchors=['STORY'],
            sha256=hashlib.sha256(self.save.read_bytes()).hexdigest(),
            save_verification_version=host.SAVE_VERIFICATION_VERSION)
        self.assertEqual(self.agent.save_preflight.records, {})
        self.assertTrue(self.agent.preflight_status()['satisfied_by_current_nodes'])
        self.closed_workflow()
        self.agent.check_model_tool('finish', {})

    def test_unknown_required_target_can_be_discovered_by_grounded_preflight(self):
        config = dict(self.config, required_saves=[], work=str(self.root / 'game/discovery-work'))
        agent = host.Agent(config)
        self.assertTrue(agent.preflight_status()['targets_unknown'])
        with self.assertRaises(host.PREFLIGHT['PreflightRequired']):
            agent.check_model_tool('test_program', dict(kind='script', root='game', path='resource.arc'))
        agent.check_model_tool('record_save_preflight', self.candidate())
        agent.dispatch('record_save_preflight', self.candidate())
        self.assertEqual(agent.preflight_status()['required'], ['current.bin'])
        self.assertTrue(agent.preflight_status()['ready_for_script_sample'])

    def test_changed_context_dependency_relocks_script_work_without_invalidating_a_node(self):
        self.record(self.needs_context())
        self.archive.write_bytes(b'changed resource bytes')
        with self.assertRaises(host.PREFLIGHT['PreflightRequired']):
            self.agent.check_model_tool('test_program', dict(kind='script', root='game', path='resource.arc'))
        self.assertEqual(self.agent.preflight_status()['pending'][0]['reason'], 'context_target_changed')
        self.assertEqual(self.agent.tests, {})

    def test_large_archive_context_target_is_streamed_without_save_input_limit(self):
        with self.archive.open('wb') as handle:
            handle.seek(33 * 1024 * 1024)
            handle.write(b'last byte')
        self.record(self.needs_context())
        self.assertTrue(self.agent.preflight_status()['ready_for_script_sample'])
        self.assertFalse(self.agent.preflight_status()['ready_for_script_batch'])

    def test_preflight_receipts_restore_on_new_host_but_changed_bytes_require_reinspection(self):
        self.record(self.needs_context())
        restored = host.Agent(self.config)
        self.assertTrue(restored.preflight_status()['ready_for_script_sample'])
        self.assertFalse(restored.preflight_status()['ready_for_script_batch'])
        self.save.write_bytes(self.save.read_bytes() + b'changed')
        restarted = host.Agent(self.config)
        self.assertFalse(restarted.preflight_status()['ready_for_script_sample'])
        self.assertEqual(restarted.preflight_status()['pending'][0]['reason'], 'not_inspected')


if __name__ == '__main__':
    unittest.main()
