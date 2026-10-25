"""Native-coordinate recovery and independent save/discovery phase budgets."""
import copy
import importlib.util
import pathlib
import unittest


spec = importlib.util.spec_from_file_location('workflow_recovery', pathlib.Path(__file__).with_name('agent_workflow.py'))
workflow = importlib.util.module_from_spec(spec)
spec.loader.exec_module(workflow)


class NativeJoinRecoveryTests(unittest.TestCase):
    def setUp(self):
        self.flow = workflow.Workflow()
        self.state = dict(dialogue_count=3, validated_saves=0, ready_for_finish=False)
        self.programs = dict(script='accepted script parser', save='save parser snapshot')
        self.prerequisites = dict(read_skill=True, read_contract=True, used_skill=True)

    def describe_failure(self, error, **fields):
        self.state['last_save_failure'] = dict(kind='save', category='save', root='save',
            path='current.sav', error=error, **fields)
        return self.flow.describe(self.state, self.programs, self.prerequisites)

    def test_legacy_and_bundled_snapshot_no_match_use_native_version_recovery(self):
        for error in (
            'Saved native scope and return address must identify exactly one independently attested script node; matches=0',
            'Current caption needs the saved native script source',
            'ValueError at model parser line 248: active native scope+return matches=0; inspect the effective resource archive/script version',
        ):
            with self.subTest(error=error):
                state = self.describe_failure(error)
                self.assertEqual(state['stage'], 'save_validation')
                self.assertIn('Repair the native coordinate relation', state['goal'])
                self.assertIn('different effective script version', state['goal'])
                self.assertIn('same resource basename', state['goal'])
                self.assertIn('not proof of runtime loading precedence', state['goal'])
                self.assertIn('Do not alter saved native values', state['goal'])
                self.assertNotIn('A save schema error', state['goal'])

    def test_no_attestations_routes_to_validation_before_version_investigation(self):
        self.state['native_attested_rows'] = 0
        state = self.describe_failure('active native scope+return matches=0; different effective resource')
        self.assertIn('host reports no independently attested native rows', state['goal'])
        self.assertIn('validate the affected scripts', state['goal'])
        self.assertIn('does not establish a wrong version', state['goal'])
        self.assertNotIn('checking other archives', state['goal'])

    def test_existing_attestations_preserve_accepted_parser_and_profile(self):
        self.state['native_attested_rows'] = 120
        state = self.describe_failure('active native scope+return matches=0; current module mismatch')
        self.assertIn('host already has independently attested native rows', state['goal'])
        self.assertIn('Preserve the accepted script parser and audited calling-ABI profile', state['goal'])
        self.assertIn('inspect/extract/test affected candidates separately', state['goal'])
        self.assertIn('incorrect coordinate/profile interpretation', state['goal'])
        self.assertIn('Preserve verified speaker bindings', state['goal'])

    def test_multiple_matches_require_uniqueness_without_caption_selection(self):
        self.state['native_attested_rows'] = 9
        for error in ('active native scope+return matches=2; unsupported ambiguity',
                      'Saved native scope and return address must identify exactly one node; matches=3'):
            with self.subTest(error=error):
                state = self.describe_failure(error)
                self.assertIn('This is ambiguous', state['goal'])
                self.assertIn('never select the first/last row or use the caption to disambiguate', state['goal'])
                self.assertNotIn('checking other archives', state['goal'])

    def test_schema_codec_and_unknown_diagnostics_are_not_native_join_failures(self):
        for error in (
            'unsupported nested VM-context layout',
            'Current anchor needs exact byte evidence at evidence.anchor',
            'native scope field has an invalid schema',
            'unknown field active native scope+return matches=0x at profile',
            'active native scope+return matches=-1',
            'active native scope+return matches=1; unrelated schema failure',
            'erisa_nemesis decoder output disagrees with native size',
        ):
            with self.subTest(error=error):
                state = self.describe_failure(error)
                self.assertIn('Repair the actual current save parser failure', state['goal'])
                self.assertNotIn('Repair the native coordinate relation', state['goal'])

    def test_structured_category_and_unchanged_prior_native_failure_are_supported(self):
        state = self.describe_failure('Bounded diagnostic',
            error_details=dict(category='native_coordinate_join', matches=0))
        self.assertIn('Repair the native coordinate relation', state['goal'])
        prior = dict(error='active native scope+return matches=0; version/profile mismatch', input_changed=False)
        state = self.describe_failure('Required current progress save has no independently verified node',
            error_details=dict(category='required_current_node', previous_failure=prior))
        self.assertIn('Repair the native coordinate relation', state['goal'])
        prior['input_changed'] = True
        state = self.describe_failure('Required current progress save has no independently verified node',
            error_details=dict(category='required_current_node', previous_failure=prior))
        self.assertNotIn('Repair the native coordinate relation', state['goal'])

    def test_pending_parser_test_allows_native_research_but_blocks_mutation_and_probe(self):
        self.flow.pending_validation = {'save'}
        state = self.describe_failure('active native scope+return matches=0; no match')
        self.assertIn('Parser changed: first call test_program', state['goal'])
        for command in ('i', 'l'):
            args = dict(command=command, archive='candidate.arc')
            self.flow.check('run_skill', args)
            self.flow.observe('run_skill', {}, 10 if command == 'i' else 11, self.state, args)
        self.assertEqual(self.flow.inspection_turns, {10, 11})
        self.assertEqual(self.flow.pending_validation, {'save'})
        self.assertFalse(self.flow.test_attempted)
        self.assertIsNone(self.flow.verified_at)
        for name, args in (
            ('run_skill', dict(command='x', archive='candidate.arc')),
            ('run_skill', dict(command='unknown', archive='candidate.arc')),
            ('write_program', dict(kind='save')),
            ('reuse_engine_adapter', dict(kind='save')),
        ):
            with self.subTest(name=name, args=args), self.assertRaisesRegex(ValueError, 'Parser changed'):
                self.flow.check(name, args)
        with self.assertRaisesRegex(ValueError, 'Parser changed'):
            self.flow.plan_probe(dict(question='Which candidate matches?', expected_result='Choose a test.',
                targets=[dict(root='game', path='candidate.arc')]))


class PreflightPhaseBudgetTests(unittest.TestCase):
    def setUp(self):
        self.flow = workflow.Workflow()
        self.flow.stage = 'resources'
        self.flow.inspection_turns = set(range(1, 10))
        self.flow.probe = dict(remaining=1, targets=[dict(root='game', path='candidate.arc')])
        self.flow.recovery_reads = {'page-a', 'page-b', 'page-c'}
        self.flow.repeated_read_turns = {2, 3}
        self.state = dict(ready_for_script_sample=True, pending=[], required=['current.sav'], records=[
            dict(path='current.sav', classification='needs_context',
                 input=dict(path='C:/synthetic/save/current.sav', sha256='a' * 64, size=10))])

    def complete(self, state=None, receipt=None):
        state = self.state if state is None else state
        return self.flow.complete_save_preflight(state, state['records'][0] if receipt is None else receipt)

    def test_initial_current_receipt_replenishes_only_exploration_phase(self):
        self.flow.pending_validation = {'script'}
        self.flow.test_attempted = True
        self.flow.verified_at = 17
        self.flow.last_signature = (30, 1)
        self.flow.read_pages = {'prior parsed page'}
        self.flow.discovery_reads = {'prior name-resource directory'}
        self.flow.script_batch_attempted = True
        self.assertTrue(self.complete())
        self.assertEqual(self.flow.inspection_turns, set())
        self.assertIsNone(self.flow.probe)
        self.assertEqual(self.flow.recovery_reads, {'page-a', 'page-b', 'page-c'})
        self.assertEqual(self.flow.repeated_read_turns, set())
        self.assertEqual(self.flow.pending_validation, {'script'})
        self.assertTrue(self.flow.test_attempted)
        self.assertEqual(self.flow.verified_at, 17)
        self.assertEqual(self.flow.last_signature, (30, 1))
        self.assertEqual(self.flow.read_pages, {'prior parsed page'})
        self.assertEqual(self.flow.discovery_reads, {'prior name-resource directory'})
        self.assertTrue(self.flow.script_batch_attempted)

    def test_phase_refill_does_not_replenish_recovery_reads_before_a_pending_test(self):
        self.flow.pending_validation = {'script'}
        self.assertTrue(self.complete())
        args = dict(root='work', path='source-inventory.json', pointer='/entries')
        self.flow.check('read_json', args)
        self.flow.observe('read_json', {}, 10, self.state, args)
        self.assertEqual(self.flow.recovery_reads, {'page-a', 'page-b', 'page-c'})
        self.assertEqual(self.flow.inspection_turns, {10})
        self.assertEqual(self.flow.pending_validation, {'script'})
        self.assertFalse(self.flow.test_attempted)
        with self.assertRaisesRegex(ValueError, 'Parser changed'):
            self.flow.check('run_skill', dict(command='x', archive='candidate.arc'))

    def test_first_save_phase_keeps_listing_advisory_and_never_bypasses_pending_test(self):
        args = dict(root='game', path='Script')
        self.flow.check('list_files', args)
        with self.assertRaisesRegex(ValueError, 'Resource exploration gate'):
            self.flow.check('run_skill', dict(command='x', archive='unplanned.arc'))
        self.assertTrue(self.complete())
        self.assertEqual(self.flow.recovery_reads, set())
        self.flow.check('list_files', args)
        self.flow.pending_validation = {'script'}
        self.flow.check('list_files', args)
        self.flow.observe('list_files', {}, 10, self.state, args)
        self.assertEqual(self.flow.inspection_turns, {10})
        self.assertEqual(self.flow.pending_validation, {'script'})
        self.assertEqual(self.flow.recovery_reads, set())
        with self.assertRaisesRegex(ValueError, 'Parser changed'):
            self.flow.check('run_skill', dict(command='x', archive='candidate.arc'))
        with self.assertRaisesRegex(ValueError, 'Parser changed'):
            self.flow.plan_probe(dict(question='Inspect the listed file?', expected_result='Choose the next test.',
                targets=[dict(root='game', path='Script/actual.ws2')]))

    def test_repeat_reclassification_mtime_and_restart_do_not_refill_again(self):
        self.assertTrue(self.complete())
        self.flow.inspection_turns = set(range(10, 18))
        self.state['records'][0]['classification'] = 'progress_candidate'
        self.state['records'][0]['input']['mtime_ns'] = 12345
        self.assertFalse(self.complete())
        self.assertEqual(len(self.flow.inspection_turns), 8)
        restored = workflow.Workflow()
        restored.restore(self.flow.snapshot())
        self.assertFalse(restored.complete_save_preflight(self.state, self.state['records'][0]))
        self.assertEqual(len(restored.inspection_turns), 8)

    def test_new_required_bytes_get_one_phase_and_returning_old_bytes_does_not(self):
        self.assertTrue(self.complete())
        self.state['records'][0]['input']['sha256'] = 'b' * 64
        self.assertTrue(self.complete())
        self.flow.inspection_turns = set(range(8))
        self.state['records'][0]['input']['sha256'] = 'a' * 64
        self.assertFalse(self.complete())
        self.assertEqual(len(self.flow.inspection_turns), 8)

    def test_pending_stale_unknown_and_nonrequired_receipts_do_not_refill(self):
        cases = []
        unknown = copy.deepcopy(self.state)
        unknown.update(required=[], records=[])
        cases.append((unknown, {}))
        pending = copy.deepcopy(self.state)
        pending['pending'] = [dict(path='current.sav', reason='context_target_changed')]
        cases.append((pending, pending['records'][0]))
        not_ready = copy.deepcopy(self.state)
        not_ready['ready_for_script_sample'] = False
        cases.append((not_ready, not_ready['records'][0]))
        cases.append((self.state, dict(path='older.sav', input=dict(path='C:/synthetic/save/older.sav', sha256='a' * 64))))
        cases.append((self.state, dict(path='current.sav', input=dict(path='C:/synthetic/save/current.sav', sha256='b' * 64))))
        for status, receipt in cases:
            with self.subTest(status=status, receipt=receipt):
                self.assertFalse(self.flow.complete_save_preflight(status, receipt))
                self.assertEqual(len(self.flow.inspection_turns), 9)
                self.assertEqual(self.flow.preflight_budget_grants, set())

    def test_all_required_current_records_are_needed_and_order_does_not_change_identity(self):
        self.state['required'].append('other.sav')
        self.assertFalse(self.complete())
        self.state['records'].append(dict(path='other.sav', classification='progress_candidate',
            input=dict(path='C:/synthetic/save/other.sav', sha256='b' * 64)))
        self.assertTrue(self.complete())
        self.state['required'].reverse()
        self.state['records'].reverse()
        self.flow.inspection_turns = set(range(8))
        self.assertFalse(self.complete())
        self.assertEqual(len(self.flow.inspection_turns), 8)

    def test_budget_receipt_cannot_grant_verified_progress_or_extension(self):
        self.assertTrue(self.complete())
        self.assertIsNone(self.flow.verified_at)
        self.assertFalse(self.flow.extension_allowed(1, dict(script='x', save='y'), dict(dialogue_count=10)))
        legacy = self.flow.snapshot()
        legacy.pop('preflight_budget_grants')
        self.flow.restore(legacy)
        self.assertEqual(self.flow.preflight_budget_grants, set())
        for malformed in (None, {}, ['not-a-digest', 5]):
            self.flow.restore(dict(legacy, preflight_budget_grants=malformed))
            self.assertEqual(self.flow.preflight_budget_grants, set())


class SourceInventoryGateTests(unittest.TestCase):
    def test_inventory_and_other_read_only_research_remain_available_after_exploration_budget(self):
        flow = workflow.Workflow()
        flow.stage = 'resources'
        flow.inspection_turns = set(range(8))
        flow.recovery_reads = {'a', 'b', 'c'}
        for pointer, offset in (('/entries', 0), ('/entries', 20), ('/entries/30/path', 0)):
            flow.check('read_json', dict(root='work', path='source-inventory.json', pointer=pointer, offset=offset))
        for name, args in (
            ('read_json', dict(root='work', path='other-inventory.json')),
            ('read_json', dict(root='game', path='source-inventory.json')),
            ('read_text', dict(root='work', path='source-inventory.json')),
            ('list_files', dict(root='game', path='Script')),
        ):
            with self.subTest(name=name, args=args):
                flow.check(name, args)
        self.assertEqual(flow.inspection_turns, set(range(8)))
        self.assertEqual(flow.recovery_reads, {'a', 'b', 'c'})
        self.assertIsNone(flow.probe)
        for command in ('x', 'unknown'):
            with self.subTest(command=command), self.assertRaisesRegex(ValueError, 'Resource exploration gate'):
                flow.check('run_skill', dict(command=command, archive='candidate.arc'))
        flow.stage = 'script_program'
        flow.check('read_json', dict(root='work', path='source-inventory.json'))
        with self.assertRaisesRegex(ValueError, 'Exploration gate'):
            flow.check('run_skill', dict(command='x', archive='candidate.arc'))

    def test_inventory_research_keeps_pending_test_counts_and_restart_state(self):
        flow = workflow.Workflow()
        flow.stage = 'resources'
        flow.pending_validation = {'script'}
        args = dict(root='work', path='source-inventory.json', pointer='/entries')
        flow.check('read_json', args)
        flow.observe('read_json', {}, 1, {}, args)
        flow.check('read_json', args)
        flow.observe('read_json', {}, 2, {}, args)
        flow.recovery_reads.update({'first', 'second', 'third'})
        next_page = dict(args, offset=20)
        flow.check('read_json', next_page)
        flow.observe('read_json', {}, 3, {}, next_page)
        self.assertEqual(flow.inspection_turns, {1, 2, 3})
        self.assertEqual(flow.repeated_read_turns, {2})
        self.assertEqual(len(flow.read_pages), 2)
        self.assertEqual(flow.recovery_reads, {'first', 'second', 'third'})
        self.assertEqual(flow.pending_validation, {'script'})
        self.assertFalse(flow.test_attempted)
        self.assertIsNone(flow.verified_at)
        restored = workflow.Workflow()
        restored.restore(flow.snapshot())
        self.assertEqual(restored.pending_validation, {'script'})
        self.assertEqual(restored.recovery_reads, flow.recovery_reads)
        self.assertEqual(restored.read_pages, flow.read_pages)
        self.assertEqual(len(restored.inspection_turns), 3)
        self.assertEqual(len(restored.repeated_read_turns), 1)
        restored.check('read_json', args)
        restored.observe('read_json', {}, 1, {}, args)
        self.assertEqual(len(restored.inspection_turns), 4)
        self.assertEqual(len(restored.repeated_read_turns), 2)
        state = restored.describe({}, {}, dict(read_skill=True, read_contract=True, used_skill=False))
        self.assertEqual(state['pending_validation'], ['script'])
        self.assertEqual(state['exploratory_requests'], 4)
        self.assertEqual(state['repeated_read_requests'], 2)
        self.assertIn('Repeated read requests: 2', state['read_only_research_advisory'])
        for name, call_args in (
            ('run_skill', dict(command='x', archive='candidate.arc')),
            ('write_program', dict(kind='script')),
            ('reuse_engine_adapter', dict(kind='script')),
        ):
            with self.subTest(name=name), self.assertRaisesRegex(ValueError, 'Parser changed'):
                restored.check(name, call_args)


if __name__ == '__main__':
    unittest.main()
