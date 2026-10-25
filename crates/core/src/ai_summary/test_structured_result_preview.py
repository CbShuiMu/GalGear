"""Bounded structural previews of real synthetic JSON, without parser execution."""
import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest import mock


SPEC = importlib.util.spec_from_file_location('structured_preview_research', Path(__file__).with_name('agent_research.py'))
research = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(research)


class StructuredResultPreviewTests(unittest.TestCase):
    def assert_bounded_json(self, value, budget=1200):
        preview = research.structural_preview(value, budget=budget)
        raw = research.encode(preview)
        self.assertLessEqual(len(raw), budget)
        self.assertEqual(json.loads(raw, parse_constant=lambda value: self.fail('Nonfinite JSON token ' + value)), preview)
        return preview

    def large_object(self):
        return {**{f'field_{index:02d}': {'nested': ['PRIVATE_DIALOGUE_' * 1000] * 5}
                   for index in range(24)}, 'tail_counter': 17}

    def test_small_layout_scalars_keep_compatible_shape_without_dialogue(self):
        result = self.assert_bounded_json(dict(offset=24, encoding='utf-16-le', count=9,
            text='PRIVATE_DIALOGUE_' * 1000, rows=[dict(text='PRIVATE_DIALOGUE_', speaker='PRIVATE_NAME')]))
        self.assertEqual(result['offset'], 24)
        self.assertEqual(result['encoding'], 'utf-16-le')
        self.assertEqual(result['count'], 9)
        self.assertEqual(result['text'], dict(string_bytes=len(('PRIVATE_DIALOGUE_' * 1000).encode())))
        self.assertNotIn('PRIVATE_DIALOGUE_', json.dumps(result))
        self.assertNotIn('PRIVATE_NAME', json.dumps(result))

    def test_large_object_reserves_every_top_level_key_before_nested_values(self):
        value = self.large_object()
        result = self.assert_bounded_json(value)
        inventory = result['key_inventory']
        self.assertEqual([row[0] for row in inventory], list(value))
        self.assertEqual(result['omitted_keys'], 0)
        self.assertEqual(result['inventory_columns'], ['key', 'type', 'count_or_utf8_bytes'])
        self.assertEqual(inventory[-1], ['tail_counter', 'number'])
        self.assertEqual(inventory[0], ['field_00', 'object', 1])
        self.assertTrue(result['ellipsis'])
        self.assertNotIn('structural_json', result)
        self.assertNotIn('PRIVATE_DIALOGUE_', json.dumps(result))

    def test_many_keys_have_honest_inventory_counts_without_invented_keys(self):
        value = {f'actual_{index}': index for index in range(10000)}
        result = self.assert_bounded_json(value)
        inventory = result.get('key_inventory', [])
        self.assertTrue(inventory)
        self.assertEqual(result['count'], len(value))
        self.assertEqual(result['omitted_keys'], len(value) - len(inventory))
        self.assertTrue(all(row[0] in value for row in inventory))
        self.assertNotIn('structural_json', result)

    def test_unicode_long_keys_use_explicit_prefix_and_identity_not_false_keys(self):
        prefix = '\U0001f600' * 1000
        value = {prefix + str(index): ['PRIVATE_DIALOGUE_'] for index in range(30)}
        result = self.assert_bounded_json(value)
        inventory = result['key_inventory']
        self.assertTrue(inventory)
        for row in inventory:
            label = row[0]
            self.assertIsInstance(label, dict)
            self.assertIn('key_prefix', label)
            self.assertNotIn(label['key_prefix'], value)
            original = next(key for key in value if hashlib.sha256(key.encode()).hexdigest() == label['key_sha256'])
            self.assertEqual(label['key_utf8_bytes'], len(original.encode()))
            self.assertTrue(original.startswith(label['key_prefix']))
        self.assertEqual(result['omitted_keys'], len(value) - len(inventory))
        self.assertNotIn('PRIVATE_DIALOGUE_', json.dumps(result))

    def test_utf8_budget_holds_for_large_unicode_and_escaped_keys_at_each_budget(self):
        values = [self.large_object(), {'\U0001f600' * 500 + str(i): {'x': i} for i in range(20)},
                  {'\\"\n' * 100 + str(i): '\u4e2d\u6587' * 1000 for i in range(20)},
                  ['\U0001f600' * 10000] * 10000, '\u4e2d\u6587' * 100000]
        for budget in (64, 96, 128, 256, 512, 1200, 2400):
            for index, value in enumerate(values):
                with self.subTest(budget=budget, value=index):
                    self.assert_bounded_json(value, budget)

    def test_deep_structure_and_giant_arrays_do_not_copy_values(self):
        nested = 'PRIVATE_DIALOGUE_' * 1000
        for _ in range(1000):
            nested = {'child': [nested]}
        preview = self.assert_bounded_json(nested)
        self.assertNotIn('PRIVATE_DIALOGUE_', json.dumps(preview))
        value = [dict(body='PRIVATE_DIALOGUE_' * 10000, offset=index) for index in range(1000)]
        preview = self.assert_bounded_json(value)
        self.assertEqual(preview['count'], 1000)
        self.assertGreater(preview['omitted_items'], 0)
        self.assertNotIn('PRIVATE_DIALOGUE_', json.dumps(preview))

    def test_no_prefix_is_parsed_as_structure_or_guessed_future_fields(self):
        result = self.assert_bounded_json({'structural_json': '{"invented_answer":', 'actual_counter': 9})
        self.assertEqual(result['structural_json'], dict(string_bytes=len('{"invented_answer":'.encode())))
        self.assertEqual(result['actual_counter'], 9)
        self.assertNotIn('invented_answer', json.dumps(result))

    def test_large_numeric_scalars_and_nonfinite_floats_remain_valid_bounded_json(self):
        for value in (10 ** 10000, -(10 ** 10000), float('inf'), float('-inf'), float('nan')):
            with self.subTest(type=type(value).__name__):
                result = self.assert_bounded_json(value)
                self.assertEqual(result['type'], 'number')
        self.assertEqual(self.assert_bounded_json({'ok': True, 'none': None}), {'ok': True, 'none': None})

    def test_preview_is_pure_and_never_follows_full_output_or_source_context(self):
        packet = dict(full_output='../../PRIVATE_OUTSIDE.json', source_context={'body': 'PRIVATE_DIALOGUE_' * 10000},
                      result=self.large_object())
        with mock.patch.object(Path, 'read_bytes', side_effect=AssertionError('A preview cannot open archives')):
            result = self.assert_bounded_json(packet)
        self.assertNotIn('PRIVATE_DIALOGUE_', json.dumps(result))
        self.assertNotIn('PRIVATE_OUTSIDE', json.dumps(result))
        # A pure structural preview never pretends the packet's inner keys are
        # at its root. The caller must advertise an explicitly selected layer.
        self.assertNotIn('tail_counter', result)

    def test_invalid_budget_fails_clearly(self):
        for budget in (0, 2, 63, None, True, '1200', 1200.0):
            with self.subTest(budget=budget), self.assertRaisesRegex(ValueError, 'UTF-8 bytes'):
                research.structural_preview({}, budget)


class StructuredPreviewArchiveTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.work = Path(temporary.name)
        self.store = research.ResearchStore(self.work)
        self.function = dict(name='inspect_data', arguments=json.dumps(dict(root='work', path='fixture.bin',
            source='def extract(data, metadata): return {"actual": len(data)}')))

    def saved(self, result, function=None):
        handle = self.store.observe('synthetic-call', function or self.function, result)
        row = self.store.records[handle['ref']]
        raw = (self.work / row['full_record']).read_bytes()
        self.assertEqual(hashlib.sha256(raw).hexdigest(), Path(row['full_record']).stem)
        return row, json.loads(raw)

    @staticmethod
    def pointed(record, pointer):
        value = record
        for token in pointer.split('/')[1:]:
            value = value[token.replace('~1', '/').replace('~0', '~')]
        return value

    def test_producer_preserves_entire_raw_result_and_exact_preview_pointer(self):
        raw_value = dict(text='PRIVATE_DIALOGUE_' * 10000, counter=17)
        row, record = self.saved(raw_value)
        self.assertEqual(record['result'], raw_value)
        self.assertEqual(row['preview_target_pointer'], '/result')
        self.assertEqual(row['read_preview'], dict(root='work', path=row['full_record'], pointer='/result'))
        self.assertEqual(self.pointed(record, row['read_preview']['pointer']), raw_value)
        self.assertNotIn('PRIVATE_DIALOGUE_', json.dumps(row))

    def test_real_packet_inner_preview_and_read_pointer_match_without_context_starvation(self):
        payload = StructuredResultPreviewTests().large_object()
        packet = dict(result=payload, source_context={'body': 'PRIVATE_DIALOGUE_' * 10000},
            full_output='tool-results/' + '0' * 64 + '.json')
        row, record = self.saved(packet)
        self.assertEqual(record['result'], packet)
        self.assertEqual(row['preview_target_pointer'], '/result/result')
        self.assertEqual(row['read_preview']['pointer'], '/result/result')
        self.assertEqual(self.pointed(record, row['read_preview']['pointer']), payload)
        self.assertEqual([entry[0] for entry in row['preview']['key_inventory']], list(payload))
        self.assertNotIn('source_context', row['preview'])
        self.assertNotIn('PRIVATE_DIALOGUE_', json.dumps(row))

    def test_noninspect_result_key_does_not_change_layer(self):
        value = dict(result={'actual': 1}, source_context={'size': 10})
        row, record = self.saved(value, dict(name='read_json', arguments='{}'))
        self.assertEqual(row['preview_target_pointer'], '/result')
        self.assertEqual(self.pointed(record, row['read_preview']['pointer']), value)
        self.assertEqual(row['preview']['result'], {'actual': 1})

    def test_full_output_envelope_is_preserved_not_trusted_as_payload(self):
        payload = {'actual_tail_key': 9}
        archive = self.store._archive(payload)
        envelope = dict(truncated=True, full_output=archive, preview={'pretend_key': 'not raw data'})
        row, record = self.saved(envelope)
        self.assertEqual(row['preview_target_pointer'], '/result')
        self.assertEqual(record['result'], envelope)
        self.assertEqual(json.loads((self.work / archive).read_bytes()), payload)
        self.assertNotIn('actual_tail_key', row['preview'])
        self.assertNotIn('not raw data', json.dumps(row['preview']))


if __name__ == '__main__':
    unittest.main()
