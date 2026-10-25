"""Advisory name research preserves counts and independent mutation gates."""
import importlib.util
from pathlib import Path
import struct
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('discovery_recovery_host', Path(__file__).with_name('agent_host.py'))
host = importlib.util.module_from_spec(spec)
spec.loader.exec_module(host)


class DiscoveryRecoveryTests(unittest.TestCase):
    def workflow(self):
        workflow = host.WORKFLOW['Workflow']()
        workflow.stage = 'speaker_review'
        workflow.speaker_metadata_step = 'discover_names'
        workflow.speaker_metadata_goal = 'Discover the actual name resource for the sample'
        workflow.inspection_turns = set(range(8))
        return workflow

    def observe_read(self, workflow, name, arguments, request):
        workflow.check(name, arguments)
        workflow.observe(name, dict(entries=[]), request,
            dict(dialogue_count=0, validated_saves=0, ready_for_finish=False), arguments=arguments)

    def test_directory_research_remains_available_after_discovery_warning_and_is_counted(self):
        workflow = self.workflow()
        for request, target in enumerate(('.', 'localization', '', 'localization', 'another-directory'), 8):
            self.observe_read(workflow, 'list_files', dict(root='game', path=target), request)
        self.assertEqual(len(workflow.inspection_turns), 13)
        self.assertEqual(workflow.discovery_reads, set())
        self.assertIsNone(workflow.verified_at)
        self.assertFalse(workflow.extension_allowed(13, {}, dict(dialogue_count=0, validated_saves=0)))

    def test_research_counts_and_pending_validation_survive_restart_without_becoming_proof(self):
        workflow = self.workflow()
        workflow.pending_validation.add('script')
        for request in (8, 9):
            self.observe_read(workflow, 'read_text', dict(root='work', path='notes.txt'), request)
        self.assertEqual(len(workflow.repeated_read_turns), 1)
        resumed = self.workflow()
        resumed.restore(workflow.snapshot())
        self.assertEqual(len(resumed.inspection_turns), 10)
        self.assertEqual(resumed.read_pages, workflow.read_pages)
        self.assertEqual(len(resumed.repeated_read_turns), 1)
        self.assertEqual(resumed.pending_validation, {'script'})
        resumed.check('list_files', dict(root='game', path='new-directory'))
        self.assertIsNone(resumed.verified_at)
        with self.assertRaisesRegex(ValueError, 'Parser changed'):
            resumed.check('write_program', dict(kind='script'))

    def test_directory_research_does_not_clear_required_parser_test_or_authorize_mutation(self):
        workflow = self.workflow()
        workflow.pending_validation.add('script')
        self.observe_read(workflow, 'list_files', dict(root='game', path='.'), 8)
        self.assertEqual(workflow.pending_validation, {'script'})
        self.assertFalse(workflow.test_attempted)
        self.assertEqual(len(workflow.inspection_turns), 9)
        self.assertEqual(workflow.discovery_reads, set())
        for name, arguments in (('write_program', dict(kind='script')),
                                ('run_skill', dict(command='x', archive='resource.arc'))):
            with self.subTest(tool=name), self.assertRaisesRegex(ValueError, 'Parser changed'):
                workflow.check(name, arguments)
        with self.assertRaisesRegex(ValueError, 'Parser changed'):
            workflow.plan_probe(dict(question='Read another file', expected_result='Bounded observation',
                                     targets=[dict(root='game', path='resource.arc')]))

    def test_repair_and_validation_allow_bounded_reads_without_authorizing_extraction(self):
        for stage in ('speaker_review', 'script_validation', 'save_validation'):
            with self.subTest(stage=stage):
                workflow = self.workflow()
                workflow.stage = stage
                workflow.speaker_metadata_step = 'repair_empty_scope'
                workflow.check('read_bytes', dict(root='game', path='unknown.arc'))
                workflow.check('list_files', dict(root='game', path='.'))
                self.assertFalse(workflow.test_attempted)
                self.assertEqual(len(workflow.inspection_turns), 8)
                with self.assertRaises(ValueError):
                    workflow.check('run_skill', dict(command='x', archive='unknown.arc'))

    def test_values_only_early_diagnostic_does_not_claim_names_already_exist(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            (root / 'game.exe').write_bytes(b'fixture')
            (root / 'sample.lng').write_bytes(struct.pack('<I', 1) + b'text')
            (root / 'sample.native').write_bytes(b'unknown native structure')
            agent = host.Agent(dict(game=str(root), save=str(root), skill=str(root),
                                    work=str(root / 'work'), language='en-US'))
            agent.dispatch('load_script_metadata', dict(root='game', path='sample.native', key='diagnostic',
                target_script=dict(root='game', path='sample.lng'),
                source="def extract(data, metadata): return {'values':{'size':len(data)},'speakers':{}}"))
            progress = agent.speaker_metadata_progress()
            self.assertEqual(progress['next_step'], 'discover_names')
            self.assertEqual(progress['empty_resources'][0]['records'], 0)
            self.assertTrue(agent.speaker_status()['speaker_review_required'])

    def test_empty_assertion_reports_exception_type_and_model_line(self):
        with self.assertRaisesRegex(RuntimeError, r'AssertionError at model parser line 2: No error message'):
            host.run_parser('def extract(data, metadata):\n    assert False\n', b'', {})

    def test_runtime_expression_error_reports_its_actual_model_line(self):
        with self.assertRaisesRegex(RuntimeError, r'ZeroDivisionError at model parser line 2:'):
            host.run_parser('def extract(data, metadata):\n    return 1 / 0\n', b'', {})


if __name__ == '__main__':
    unittest.main()
