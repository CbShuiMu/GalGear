"""Synthetic indirect-index fixtures; no game data or game-specific constants."""
import struct
import unittest
from unittest import mock

from test_evidence import checks


class IndexedTextEvidenceTests(unittest.TestCase):
    def fixture(self, endian='little', encrypted=True):
        texts = ['', 'Name', 'Current', 'Unused']
        order = '<' if endian == 'little' else '>'
        records, pool = bytearray(), bytearray()
        for index, text in enumerate(texts):
            raw = text.encode('utf-16-le')
            record = bytearray(b'P' * 12)
            struct.pack_into(order + 'I', record, 2, len(pool) // 2)
            struct.pack_into(order + 'H', record, 6, len(raw) // 2)
            records.extend(record)
            key = (index * 13 + 7).to_bytes(2, endian)
            pool.extend(bytes(value ^ key[n % 2] for n, value in enumerate(raw)) if encrypted else raw)
        code = 4 + len(records)
        data = struct.pack(order + 'I', len(texts)) + records + struct.pack(order + 'I', 2) + pool
        schema = dict(kind='offset_length_array', offset=4, length=len(records),
            count=dict(offset=0, width=4, endian=endian), record_size=12,
            offset_field=dict(offset=2, width=4, endian=endian, unit=2),
            length_field=dict(offset=6, width=2, endian=endian, unit=2),
            base=code + 4, pool_length=len(pool), encoding='utf-16-le')
        if encrypted:
            schema['entry_xor'] = dict(width=2, multiplier=13, addend=7, endian=endian)
        output = dict(tables=dict(strings=schema), rows=[dict(position=code, text='Current', speaker=None,
            evidence=dict(table='strings', index=2, reference=dict(base=0, operand_offset=0, width=4, endian=endian)))])
        return bytes(data), output, texts

    def test_complete_native_index_and_ordinal_key_preserve_empty_and_unused_records(self):
        for endian in ('little', 'big'):
            with self.subTest(endian=endian):
                data, output, texts = self.fixture(endian)
                entries = checks['text_table'](data, output['tables']['strings'])
                self.assertEqual([text for text, _ in entries], texts)
                self.assertEqual([checks['span'](data, proof) for _, proof in entries], texts)
                rows = checks['script_rows'](data, output)
                self.assertEqual(rows[0]['text'], 'Current')

    def test_wrong_reference_or_transform_cannot_prove_a_message(self):
        for mutation in ('reference', 'key'):
            data, output, _ = self.fixture()
            if mutation == 'reference':
                output['rows'][0]['evidence']['index'] = 3
            else:
                output['tables']['strings']['entry_xor']['addend'] += 1
            with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                checks['script_rows'](data, output)

    def test_native_count_and_every_unused_entry_are_checked(self):
        for mutation in ('count', 'last_offset', 'last_length'):
            data, output, _ = self.fixture()
            changed = bytearray(data)
            if mutation == 'count':
                struct.pack_into('<I', changed, 0, 5)
            elif mutation == 'last_offset':
                struct.pack_into('<I', changed, 4 + 3 * 12 + 2, len(changed))
            else:
                struct.pack_into('<H', changed, 4 + 3 * 12 + 6, 65535)
            with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                checks['script_rows'](bytes(changed), output)

    def test_record_and_transform_schema_reject_overlap_unknown_fields_and_booleans(self):
        data, output, _ = self.fixture()
        schema = output['tables']['strings']
        for patch in (dict(record_size=True), dict(fields=[]), dict(encoding=[]),
                      dict(length_field=dict(offset=3, width=2)),
                      dict(entry_xor=dict(width=2, multiplier=True, addend=7, endian='little'))):
            with self.subTest(patch=patch), self.assertRaises(ValueError):
                checks['text_table'](data, dict(schema, **patch))

    def test_plain_pool_aliases_retain_distinct_native_ordinals(self):
        data, output, texts = self.fixture(encrypted=False)
        changed = bytearray(data)
        changed[4 + 3 * 12 + 2:4 + 3 * 12 + 8] = changed[4 + 2 * 12 + 2:4 + 2 * 12 + 8]
        entries = checks['text_table'](bytes(changed), output['tables']['strings'])
        self.assertEqual([text for text, _ in entries], texts[:3] + ['Current'])

    def test_indirect_schema_cannot_replace_a_known_inline_native_table(self):
        data, output, _ = self.fixture()
        schema = output['tables']['strings']
        with self.assertRaisesRegex(ValueError, 'interleaved'):
            checks['script_rows'](data, output, required_table_regions=[dict(offset=schema['offset'], length=schema['length'])])

    def test_decoded_size_budget_includes_unused_entries(self):
        data, output, _ = self.fixture()
        module = checks['INDEXED_TEXT']['text_table'].__globals__
        with mock.patch.dict(module, MAX_DECODED_BYTES=4), self.assertRaisesRegex(ValueError, 'budget'):
            checks['text_table'](data, output['tables']['strings'])


if __name__ == '__main__':
    unittest.main()
