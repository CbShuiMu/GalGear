"""Offline regressions for durable, partially accepted research notes."""
import importlib.util
import json
import pathlib
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('agent_research', pathlib.Path(__file__).with_name('agent_research.py'))
research = importlib.util.module_from_spec(spec)
spec.loader.exec_module(research)


class ResearchTests(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.work = pathlib.Path(temp.name)
        self.audit = []
        self.store = research.ResearchStore(self.work, lambda: self.audit)

    def observe(self, identity='call-long-one', result=None, name='inspect_data', inputs=None):
        return self.store.observe(identity, dict(name=name, arguments=json.dumps(dict(
            root='game', path='ev.noa', source='def extract(data, metadata): return {"offset":4}'))),
            result if result is not None else dict(offset=4, count=10, encoding='utf-16-le'), inputs=inputs)

    def finding(self, identity='layout', refs=None, status='observed'):
        return dict(id=identity, status=status, summary='The section payload begins after its header.',
                    evidence_refs=refs or ['O1'], coordinates={'section_header': {'base': 'file', 'offset': 16},
                                                             'section_payload': {'base': 'file', 'offset': 32}})

    def test_short_ids_are_stable_across_restart_and_legacy_lookup(self):
        handle = self.observe()
        self.assertEqual(handle['ref'], 'O1')
        self.assertEqual(self.store.resolve('call-long-one')['ref'], 'O1')
        recreated = research.ResearchStore(self.work)
        self.assertEqual(recreated.resolve('O1')['tool_call_id'], 'call-long-one')
        self.assertEqual(recreated.observe('call-two', {'name': 'read_bytes'}, {'offset': 2})['ref'], 'O2')
        self.assertEqual(recreated.observe('call-long-one', {'name': 'inspect_data'}, {})['ref'], 'O1')

    def test_partial_batch_acceptance_and_helpful_reference_error(self):
        self.observe()
        self.observe('call-long-two')
        invalid = self.finding('typo', ['call-long-on'])
        result = self.store.record_findings(dict(findings=[self.finding(), invalid,
            self.finding('second', ['latest']), None], next_steps=['Write a minimal parser.']))
        self.assertEqual([row['id'] for row in result['accepted']], ['layout', 'second'])
        self.assertEqual(len(result['errors']), 2)
        self.assertFalse(result['ok'])
        self.assertEqual(result['status'], 'partial')
        self.assertIn('Only 2', result['error'])
        self.assertEqual(result['findings_saved'], 2)
        self.assertTrue(result['next_steps_saved'])
        self.assertFalse(research.successful(result))
        self.assertIn('O1', result['errors'][0]['references'][0]['closest_refs'])
        self.assertEqual(self.store.notes['findings']['second']['evidence_refs'], ['O2'])
        self.assertEqual(self.store.notes['next_steps'], ['Write a minimal parser.'])
        all_invalid = self.store.record_findings({'findings': [self.finding('bad', ['invented'])], 'next_steps': False})
        self.assertEqual(all_invalid['accepted'], [])
        self.assertEqual(len(all_invalid['errors']), 2)
        self.assertEqual(len(self.store.notes['findings']), 2)
        self.assertEqual(all_invalid['status'], 'rejected')
        self.assertIsNone(all_invalid['saved'])
        self.assertFalse(all_invalid['next_steps_saved'])
        self.assertIn('No submitted findings were saved', all_invalid['error'])

    def test_failures_cannot_be_observed_or_replace_latest_success(self):
        self.observe()
        self.observe('call-error', {'error': 'Invalid table boundary'})
        self.assertEqual(self.store.resolve('latest')['ref'], 'O1')
        result = self.store.record_findings(dict(findings=[self.finding('bad', ['O2']),
            self.finding('rejected', ['O2'], status='rejected')]))
        self.assertEqual([row['id'] for row in result['accepted']], ['rejected'])
        self.assertEqual(len(result['errors']), 1)
        snapshot = self.store.snapshot()
        self.assertEqual([row['ref'] for row in snapshot['observations']], ['O1'])
        self.assertFalse(snapshot['host_verified'])

    def test_bootstraps_old_audit_without_copying_private_reasoning(self):
        function = dict(name='inspect_data', arguments='{"root":"game","path":"scene.csx"}')
        self.audit.extend([dict(role='assistant', reasoning_content='PRIVATE SECRET THOUGHTS',
                              tool_calls=[dict(id='legacy-call', function=function)]),
                           dict(role='tool', tool_call_id='legacy-call', content='{"offset":16}')])
        self.store.bootstrap()
        old_notes = dict(findings={'old': dict(status='observed', summary='Payload offset is 16.',
                                             evidence=[{'tool_call_id': 'legacy-call'}])}, next_steps=['Test.'])
        self.store.restore(notes=old_notes)
        self.assertEqual(self.store.notes['findings']['old']['evidence_refs'], ['O1'])
        for path in self.work.rglob('*.json'):
            self.assertNotIn('PRIVATE SECRET THOUGHTS', path.read_text('utf-8'))
        self.assertEqual(len(self.store.snapshot()['observations']), 1)

    def test_result_preview_is_bounded_and_does_not_duplicate_dialogue(self):
        dialogue = 'FULL_GAME_DIALOGUE_' * 5000
        self.observe(result={'text': dialogue, 'rows': [{'speaker': 'NAME', 'text': dialogue}],
                             'offset': 24, 'count': 999, 'encoding': 'utf-16-le'})
        ledger = json.loads((self.work / 'analysis-ledger.json').read_text('utf-8'))
        entry = ledger['observations'][0]
        self.assertLessEqual(len(research.encode(entry['preview'])), 1200)
        self.assertNotIn('FULL_GAME_DIALOGUE_', json.dumps(entry))
        self.assertEqual(entry['preview']['offset'], 24)
        self.assertEqual(entry['preview']['encoding'], 'utf-16-le')
        record = json.loads((self.work / entry['full_record']).read_text('utf-8'))
        self.assertEqual(record['result']['text'], dialogue)
        giant = research.structural_preview({str(index): {'offset': index} for index in range(1000)})
        self.assertLessEqual(len(research.encode(giant)), 1200)

    def test_changed_input_marks_affected_observations_and_notes_stale(self):
        stamp = dict(root='game', path='ev.noa', size=10, mtime_ns=5, sha256='a' * 64)
        self.observe(inputs=stamp)
        self.store.record_findings(dict(findings=[self.finding()]))
        changed = dict(stamp, sha256='b' * 64)
        self.assertTrue(self.store.set_inputs(changed))
        self.assertTrue(self.store.notes['findings']['layout']['stale'])
        self.assertTrue(self.store.needs_recheck)
        self.assertIsNone(self.store.resolve('latest'))
        new = self.observe('new-bytes', inputs=changed)
        self.assertFalse(new['stale'])
        self.assertEqual(self.store.resolve('latest')['ref'], 'O2')
        restored = research.ResearchStore(self.work)
        self.assertTrue(restored.resolve('O1')['stale'])
        self.assertFalse(restored.resolve('O2')['stale'])

    def test_notes_and_next_steps_are_independently_bounded(self):
        self.observe()
        items = [dict(self.finding(str(index)), summary='x' * 2300) for index in range(20)]
        result = self.store.record_findings(dict(findings=items, next_steps=['x' * 500] * 8))
        self.assertGreater(len(result['accepted']), 0)
        self.assertGreater(len(result['errors']), 0)
        self.assertLessEqual(len(research.encode(self.store.notes)), research.MAX_NOTES_BYTES)
        self.assertFalse(self.store.snapshot()['host_verified'])

    def test_legacy_evidence_field_and_no_guessed_coordinates(self):
        self.observe()
        item = self.finding()
        item['evidence_call_ids'] = ['call-long-one']
        del item['evidence_refs']
        self.store.record_findings(dict(findings=[item]))
        self.assertEqual(self.store.notes['findings']['layout']['coordinates']['section_payload']['offset'], 32)
        plain = self.finding('plain')
        del plain['coordinates']
        self.store.record_findings(dict(findings=[plain]))
        self.assertNotIn('coordinates', self.store.notes['findings']['plain'])
        invalid = self.finding('unknown')
        invalid['coordinates'] = {'invented_base': 100}
        self.assertEqual(len(self.store.record_findings(dict(findings=[invalid]))['errors']), 1)

    def test_explicit_layout_coordinates_from_failed_autonomous_run_survive_restart(self):
        self.observe()
        coordinates = [
            {'coordinate_base': 'input_file_byte_zero', 'archive': 'game/localized.arc',
             'member': 'chapter.lng', 'section_header': '0..341', 'section_payload': '342..10491'},
            {'file': 'work/unpacked/chapter.lng', 'table_offset': 4, 'count_offset': 0,
             'base': 342, 'pool_length': 10150},
            {'file': 'work/script_extractor.py', 'position_base': 'pool_start'},
            {'coordinate_base': 'save_file_byte_zero', 'save': {'offset': 16, 'unit': 'bytes'}}]
        items = [dict(self.finding('layout-' + str(index)), coordinates=value)
                 for index, value in enumerate(coordinates)]
        result = self.store.record_findings(dict(findings=items, next_steps=['Test the existing parser.']))
        self.assertTrue(result['ok'])
        self.assertEqual(result['status'], 'saved')
        self.assertNotIn('error', result)
        self.assertEqual(result['findings_saved'], 4)
        self.assertEqual(set(research.COORDINATES_SCHEMA['properties']), research.COORDINATES)
        restored = research.ResearchStore(self.work)
        for index, value in enumerate(coordinates):
            finding = restored.notes['findings']['layout-' + str(index)]
            self.assertEqual(finding['coordinates'], value)
            self.assertEqual(finding['evidence_refs'], ['O1'])
            self.assertFalse(restored.snapshot()['host_verified'])

    def test_missing_evidence_is_an_explicit_failure_and_can_be_repaired_without_research(self):
        self.observe()
        item = self.finding()
        del item['evidence_refs']
        result = self.store.record_findings(dict(findings=[item], next_steps=['Test the parser.']))
        self.assertFalse(result['ok'])
        self.assertEqual(result['status'], 'rejected')
        self.assertEqual(result['findings_saved'], 0)
        self.assertTrue(result['next_steps_saved'])
        self.assertEqual(result['errors'][0]['field'], 'evidence_refs')
        self.assertEqual(result['available_refs'], ['O1'])
        self.assertEqual(self.store.notes['findings'], {})
        self.assertFalse(research.successful(result))
        item['evidence_refs'] = result['available_refs']
        repaired = self.store.record_findings(dict(findings=[item]))
        self.assertTrue(repaired['ok'])
        self.assertEqual(repaired['findings_saved'], 1)
        self.assertEqual(self.store.notes['findings']['layout']['evidence_refs'], ['O1'])
        self.assertEqual(self.store.notes['next_steps'], ['Test the parser.'])

    def test_invalid_coordinate_feedback_preserves_previously_saved_finding(self):
        self.observe()
        self.store.record_findings(dict(findings=[self.finding()]))
        original = self.store.notes['findings']['layout']
        invalid = dict(self.finding(), coordinates={'invented_base': 100})
        result = self.store.record_findings(dict(findings=[invalid]))
        self.assertEqual(result['status'], 'rejected')
        self.assertEqual(result['errors'][0]['field'], 'coordinates')
        self.assertIn('Unsupported coordinate fields: invented_base', result['errors'][0]['error'])
        self.assertIn('coordinate_base', result['errors'][0]['error'])
        self.assertEqual(self.store.notes['findings']['layout'], original)

    def test_coordinate_values_are_bounded_json_not_implicit_or_nonfinite_claims(self):
        self.observe()
        for value in (True, [0, 4], float('nan'), float('inf'), {'offset': True},
                      {'base': {'base': {'base': {'offset': 0}}}}, 'x' * 2001):
            with self.subTest(value=str(value)[:40]):
                result = self.store.record_findings(dict(findings=[dict(self.finding(), coordinates={'member': value})]))
                self.assertEqual(result['status'], 'rejected')
                self.assertEqual(result['accepted'], [])
        self.assertEqual(self.store.notes['findings'], {})

    def test_non_object_notes_request_cannot_report_a_successful_save(self):
        for value in (None, [], 'notes'):
            with self.subTest(value=value):
                result = self.store.record_findings(value)
                self.assertFalse(result['ok'])
                self.assertEqual(result['status'], 'rejected')
                self.assertIsNone(result['saved'])
                self.assertEqual(result['findings_saved'], 0)
                self.assertTrue(result['error'])


if __name__ == '__main__':
    unittest.main()
