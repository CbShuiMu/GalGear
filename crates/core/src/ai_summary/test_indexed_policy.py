"""Offline checks that a known indexed container cannot bypass ordinal evidence."""
import copy
import pathlib
import runpy
import struct
import unittest


checks = runpy.run_path(str(pathlib.Path(__file__).with_name('evidence.py')))


class IndexedRegionPolicyTests(unittest.TestCase):
    def setUp(self):
        def pool(strings):
            return struct.pack('<I', len(strings)) + b''.join(
                struct.pack('<I', len(text)) + text + struct.pack('<I', 0) for text in strings)
        first = pool([b'', b'Earlier', b'Current'])
        second = pool([b'', b'Other'])
        unrelated = pool([b'Current'])
        code = struct.pack('<III', 2, 1, 0)
        self.data = code + first + second + unrelated
        self.first = dict(offset=len(code), length=len(first))
        self.second = dict(offset=len(code) + len(first), length=len(second))
        self.unrelated = dict(offset=len(code) + len(first) + len(second), length=len(unrelated))
        self.output = dict(rows=[dict(position=0, text='Current', speaker=None,
                                     evidence=dict(table='strings', index=2,
                                                   reference=dict(base=0, operand_offset=0, width=4, endian='little')))],
                           tables=dict(strings=self.schema(self.first)))

    def schema(self, region):
        return dict(region, count_width=4, endian='little',
                    fields=[dict(kind='text', length_width=4, encoding='utf-8'),
                            dict(kind='skip', count_width=4, unit=4)])

    def verify(self, output=None, regions=None, data=None):
        return checks['script_rows'](self.data if data is None else data,
                                     self.output if output is None else output,
                                     [self.first] if regions is None else regions)

    def direct_row(self):
        offset = self.data.index(b'Current')
        return dict(position=offset, text='Current', speaker=None,
                    evidence=dict(offset=offset, length=7, encoding='utf-8'))

    def test_exact_complete_table_preserves_empty_ordinals_and_checks_actual_operand(self):
        rows = self.verify()
        self.assertEqual(rows[0]['evidence']['index'], 2)
        self.assertEqual(rows[0]['text'], 'Current')
        changed = struct.pack('<I', 1) + self.data[4:]
        with self.assertRaisesRegex(ValueError, 'Instruction string index'):
            self.verify(data=changed)

    def test_direct_spans_cannot_replace_indexed_output_even_with_a_valid_table(self):
        row = self.direct_row()
        for output in ([row], dict(rows=[row]), dict(rows=[row], tables=self.output['tables'])):
            with self.subTest(output=output), self.assertRaisesRegex(ValueError, 'indexed|table'):
                self.verify(output)

    def test_every_required_region_needs_an_exact_table_schema(self):
        for change in (dict(offset=self.first['offset'] + 1, length=self.first['length'] - 1),
                       dict(offset=self.first['offset'], length=self.first['length'] - 1)):
            output = copy.deepcopy(self.output)
            output['tables']['strings'].update(change)
            with self.subTest(change=change), self.assertRaisesRegex(ValueError, 'exactly matching every required'):
                self.verify(output)
        with self.assertRaisesRegex(ValueError, 'exactly matching every required'):
            self.verify(regions=[self.first, self.second])
        output = copy.deepcopy(self.output)
        output['tables']['other'] = self.schema(self.second)
        output['rows'].append(dict(position=4, text='Other', speaker=None,
                                   evidence=dict(table='other', index=1, reference=dict(operand_offset=0))))
        self.assertEqual([row['text'] for row in self.verify(output, [self.first, self.second])],
                         ['Current', 'Other'])

    def test_required_region_must_be_walked_to_its_exact_end_including_empty_records(self):
        changed = (self.data[:self.first['offset']] + struct.pack('<I', 2)
                   + self.data[self.first['offset'] + 4:])
        with self.assertRaisesRegex(ValueError, 'exactly at its declared boundary'):
            self.verify(data=changed)
        output = copy.deepcopy(self.output)
        output['rows'][0]['evidence']['index'] = 1
        with self.assertRaisesRegex(ValueError, 'Instruction string index'):
            self.verify(output)

    def test_valid_unrelated_table_cannot_substitute_for_required_region(self):
        output = copy.deepcopy(self.output)
        output['tables']['unrelated'] = self.schema(self.unrelated)
        output['rows'][0]['position'] = 8
        output['rows'][0]['evidence'].update(table='unrelated', index=0)
        self.assertEqual(checks['script_rows'](self.data, output)[0]['text'], 'Current')
        with self.assertRaisesRegex(ValueError, 'matching a required complete region'):
            self.verify(output)

    def test_known_indexed_regions_require_real_operand_evidence_and_reject_ordinal_mode(self):
        for reference in (None, {}, dict(mode='ordinal'), dict(mode='ordinal', operand_offset=0)):
            output = copy.deepcopy(self.output)
            output['rows'][0]['evidence']['reference'] = reference
            if isinstance(reference, dict) and reference.get('mode') == 'ordinal':
                output['rows'][0]['position'] = 2
                self.assertEqual(checks['script_rows'](self.data, output)[0]['position'], 2)
            with self.subTest(reference=reference), self.assertRaisesRegex(ValueError, 'actual operand_offset'):
                self.verify(output)
        output = copy.deepcopy(self.output)
        del output['rows'][0]['evidence']['reference']
        with self.assertRaisesRegex(ValueError, 'actual operand_offset'):
            self.verify(output)

    def test_required_raw_regions_cannot_be_reinterpreted_through_output_view(self):
        output = copy.deepcopy(self.output)
        output['view'] = [dict(op='slice', offset=0, length=len(self.data))]
        self.assertEqual(checks['script_rows'](self.data, output)[0]['text'], 'Current')
        with self.assertRaisesRegex(ValueError, 'original byte view'):
            self.verify(output)

    def test_absent_or_empty_policy_preserves_direct_and_native_ordinal_support(self):
        direct = [self.direct_row()]
        for policy in (None, []):
            with self.subTest(policy=policy):
                self.assertEqual(checks['script_rows'](self.data, direct, policy), direct)
                output = copy.deepcopy(self.output)
                output['rows'][0]['position'] = 2
                output['rows'][0]['evidence']['reference'] = dict(mode='ordinal')
                self.assertEqual(checks['script_rows'](self.data, output, policy)[0]['position'], 2)

    def test_invalid_required_regions_fail_closed_with_a_clear_value_error(self):
        for policy in ('invalid', [None], [{}], [dict(offset=-1, length=1)],
                       [dict(offset=0, length=0)], [dict(offset=True, length=4)],
                       [dict(offset=0, length=len(self.data) + 1)]):
            with self.subTest(policy=policy), self.assertRaisesRegex(ValueError, 'indexed table region'):
                checks['script_rows'](self.data, self.output, policy)


if __name__ == '__main__':
    unittest.main()
