"""Synthetic declarative native-scope/return links; no game or model calls."""
import copy
import importlib.util
import pathlib
import unittest
from unittest import mock


SPEC = importlib.util.spec_from_file_location('adapter_evidence_tests', pathlib.Path(__file__).with_name('adapter_evidence.py'))
adapter = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(adapter)


def u32(value):
    return value.to_bytes(4, 'little')


def integer(data, offset, width=4, endian='little'):
    if (type(offset) is not int or type(width) is not int or width not in (1, 2, 3, 4, 8)
            or endian not in ('little', 'big') or not 0 <= offset <= len(data) - width):
        raise ValueError('Integer evidence is outside the supplied bytes')
    return int.from_bytes(data[offset:offset + width], endian)


def view(data, steps):
    if steps:
        raise ValueError('Synthetic fixture uses raw bytes')
    return data


def span(data, proof):
    if not isinstance(proof, dict):
        raise ValueError('Supply text byte evidence')
    raw = view(data, proof.get('view', []))
    offset, size = proof.get('offset'), proof.get('length')
    if type(offset) is not int or type(size) is not int or not 0 <= offset <= len(raw) - size or size < 0:
        raise ValueError('Text evidence is outside the supplied bytes')
    return raw[offset:offset + size].decode(proof['encoding'])


READERS = dict(span=span, integer=integer, view=view)


class AdapterEvidenceTests(unittest.TestCase):
    def fixture(self, duplicate_scope=False, qualified=False):
        instruction = b'\x11' + u32(7) + b'\x20\x00'
        code = instruction * 3
        data = b'CODE' + u32(len(code)) + code
        table_start, names, raw_table = len(data), [], u32(2)
        for address, name in ((0, 'function_a'), (14, 'function_a' if duplicate_scope else 'function_b')):
            if qualified:
                name = 'namespace' + (qualified if isinstance(qualified, str) else '::') + name
            encoded = name.encode('utf-16-le')
            raw_table += u32(1) + u32(address) + u32(0xffffffff) + u32(0) + u32(len(encoded) // 2)
            names.append(dict(offset=table_start + len(raw_table), length=len(encoded), encoding='utf-16-le'))
            raw_table += encoded
        data += raw_table
        table = dict(offset=table_start, length=len(raw_table), count_width=4, fields=[
            dict(kind='uint', name='flags', width=4), dict(kind='uint', name='address', width=4),
            dict(kind='uint', name='unknown_length', width=4), dict(kind='uint', name='reserved', width=4),
            dict(kind='text', name='scope', length_width=4, unit=2, encoding='utf-16-le')])
        rows = []
        for position in (0, 7, 14):
            selected = 0 if position < 14 else 1
            native = dict(kind='call_return', scope=names[selected],
                scope_range=dict(table=table, index=selected, start_field='address', scope_field='scope', base=8),
                sequence=dict(offset=8 + position, fields=[dict(kind='bytes', hex='11'),
                    dict(kind='table_ref', name='text_index', table='strings', width=4, endian='little'),
                    dict(kind='bytes', hex='2000')]), text_operand='text_index')
            native['code_region'] = dict(offset=0, tag_hex='434f4445', length_width=4)
            rows.append(dict(position=position, text='Current', speaker=None,
                evidence=dict(table='strings', index=7, reference=dict(base=8, operand_offset=1), native=native)))
        return data, rows

    def attest(self, data, rows):
        return adapter.attest_native_nodes(data, rows, **READERS)

    def canonical(self, data, rows, source='translated.script'):
        records = self.attest(data, rows)
        return [dict(source=source, position=row['position'], text=row['text'], speaker=row['speaker'],
                     _native=records.get(row['position'], [])) for row in rows]

    def saved(self, scope='function_a', position=7):
        text = scope.encode('utf-8')
        return text + u32(position), dict(kind='call_return', scope=dict(offset=0, length=len(text), encoding='utf-8'),
                                         position=dict(offset=len(text), width=4, endian='little'))

    def test_return_coordinate_is_derived_from_exact_sequence_and_resolves_function_scope(self):
        data, rows = self.fixture()
        canonical = self.canonical(data, rows)
        self.assertEqual([row['_native'][0]['value'] for row in canonical], [7, 14, 21])
        save, link = self.saved()
        resolved = adapter.resolve_native_node(save, link, canonical, **READERS)
        self.assertEqual((resolved['source'], resolved['position']), ('translated.script', 0))
        self.assertNotEqual(resolved['source'], 'function_a')

    def test_complete_scope_table_is_walked_once_for_many_rows(self):
        data, rows = self.fixture()
        with mock.patch.object(adapter, '_table', wraps=adapter._table) as walk:
            records = self.attest(data, rows)
        self.assertEqual(len(records), 3)
        walk.assert_called_once()

    def test_explicit_leaf_projection_joins_native_short_scope(self):
        for separator in ('::', '@'):
            with self.subTest(separator=separator):
                data, rows = self.fixture(qualified=separator)
                for row in rows:
                    row['evidence']['native']['scope_range']['scope_transform'] = dict(kind='last_component', separator=separator)
                canonical = self.canonical(data, rows)
                self.assertEqual(canonical[0]['_native'][0]['scope'], 'function_a')
                save, link = self.saved('function_a', 7)
                self.assertEqual(adapter.resolve_native_node(save, link, canonical, **READERS)['position'], 0)

    def test_short_scope_does_not_match_without_declared_projection(self):
        data, rows = self.fixture(qualified=True)
        save, link = self.saved('function_a', 7)
        with self.assertRaisesRegex(ValueError, 'matches=0'):
            adapter.resolve_native_node(save, link, self.canonical(data, rows), **READERS)

    def test_scope_projection_still_needs_the_complete_native_name_span(self):
        data, rows = self.fixture(qualified=True)
        native = rows[0]['evidence']['native']
        native['scope_range']['scope_transform'] = dict(kind='last_component', separator='::')
        native['scope']['offset'] += len('namespace::'.encode('utf-16-le'))
        native['scope']['length'] -= len('namespace::'.encode('utf-16-le'))
        with self.assertRaisesRegex(ValueError, 'same address record'):
            self.attest(data, rows[:1])

    def test_scope_projection_cannot_choose_by_caption_when_aliases_collide(self):
        data, rows = self.fixture(qualified=True)
        for row in rows:
            row['evidence']['native']['scope_range']['scope_transform'] = dict(kind='last_component', separator='::')
        canonical = self.canonical(data, rows, 'one.script') + self.canonical(data, rows, 'two.script')
        save, link = self.saved('function_a', 7)
        with self.assertRaisesRegex(ValueError, 'matches=2'):
            adapter.resolve_native_node(save, link, canonical, **READERS)

    def test_scope_projection_rejects_unknown_operations_and_unbounded_separators(self):
        data, rows = self.fixture(qualified=True)
        for transform in ({'kind':'lower','separator':'::'}, {'kind':'last_component','separator':'function'},
                          {'kind':'last_component','separator':''}, {'kind':'last_component','separator':'\0'},
                          {'kind':'last_component','separator':':'*9}, {'kind':'last_component','separator':'::','value':'invented'}):
            candidate = copy.deepcopy(rows[:1])
            candidate[0]['evidence']['native']['scope_range']['scope_transform'] = transform
            with self.subTest(transform=transform), self.assertRaises(ValueError):
                self.attest(data, candidate)

    def test_shared_profiles_derive_scope_and_sequence_without_per_row_schema(self):
        data, rows = self.fixture()
        expected = self.attest(data, rows)
        template = copy.deepcopy(rows[-1]['evidence']['native'])
        template.pop('scope')
        template['scope_range'].pop('index')
        template['sequence'].pop('offset')
        for row in rows:
            row['evidence']['native'] = dict(profile='display', scope_index=0 if row['position'] < 14 else 1)
        with mock.patch.object(adapter, '_table', wraps=adapter._table) as walk:
            actual = adapter.attest_native_nodes(data, rows, profiles={'display': template}, **READERS)
        walk.assert_called_once()
        for position in expected:
            for key in ('scope', 'value', 'position', 'script_sha256'):
                self.assertEqual(actual[position][0][key], expected[position][0][key])

    def test_shared_profiles_cannot_supply_host_derived_fields_or_unknown_ids(self):
        data, rows = self.fixture()
        template = copy.deepcopy(rows[-1]['evidence']['native'])
        template.pop('scope')
        template['scope_range'].pop('index')
        template['sequence'].pop('offset')
        rows[0]['evidence']['native'] = dict(profile='display', scope_index=0)
        with self.assertRaisesRegex(ValueError, 'Unknown native proof profile'):
            adapter.attest_native_nodes(data, rows[:1], **READERS)
        template['sequence']['offset'] = 8
        with self.assertRaisesRegex(ValueError, 'leave scope, index and sequence offset'):
            adapter.attest_native_nodes(data, rows[:1], profiles={'display': template}, **READERS)

    def test_adapter_cannot_supply_return_address_or_host_records_directly(self):
        data, rows = self.fixture()
        rows[0]['_native'] = [dict(kind='call_return', value=999)]
        self.assertEqual(self.attest(data, rows)[0][0]['value'], 7)
        for where in ('native', 'sequence'):
            bad = copy.deepcopy(rows)
            target = bad[0]['evidence']['native']
            if where == 'sequence':
                target = target['sequence']
            target['return_address'] = 999
            with self.subTest(where=where), self.assertRaisesRegex(ValueError, 'unsupported fields'):
                self.attest(data, bad)

    def test_scope_name_and_address_must_be_from_the_same_complete_table_record(self):
        data, rows = self.fixture(duplicate_scope=True)
        rows[0]['evidence']['native']['scope'] = rows[-1]['evidence']['native']['scope']
        with self.assertRaisesRegex(ValueError, 'same address record'):
            self.attest(data, rows)

    def test_partial_or_malformed_scope_tables_cannot_supply_function_boundaries(self):
        data, rows = self.fixture()
        for mutation in ('truncate', 'count', 'fields'):
            bad, raw = copy.deepcopy(rows), data
            table = bad[0]['evidence']['native']['scope_range']['table']
            if mutation == 'truncate':
                table['length'] -= 1
            elif mutation == 'count':
                raw = data[:table['offset']] + u32(3) + data[table['offset'] + 4:]
            else:
                table['fields'].reverse()
            with self.subTest(mutation=mutation), self.assertRaises((ValueError, UnicodeError)):
                self.attest(raw, bad)

    def test_separate_scalar_spans_cannot_replace_record_table_proof(self):
        data, rows = self.fixture()
        rows[0]['evidence']['native']['scope_range'] = dict(base=8, start=dict(offset=0), end=dict(offset=4))
        with self.assertRaisesRegex(ValueError, 'Native scope range'):
            self.attest(data, rows)

    def test_unverified_ordinal_or_equal_operand_at_another_address_is_rejected(self):
        data, rows = self.fixture()
        bad = copy.deepcopy(rows)
        bad[0]['evidence']['reference'] = dict(mode='ordinal')
        with self.assertRaisesRegex(ValueError, 'verified table operand'):
            self.attest(data, bad)
        # A second operand happens to contain the same integer in the next
        # instruction, but cannot replace the first row's proved reference.
        bad = copy.deepcopy(rows[:1])
        bad[0]['evidence']['native']['sequence']['fields'] = [
            dict(kind='bytes', hex=data[8:16].hex()),
            dict(kind='uint', name='text_index', width=4),
            dict(kind='bytes', hex='2000')]
        with self.assertRaisesRegex(ValueError, 'same verified table reference'):
            self.attest(data, bad)

    def test_sequence_cannot_move_its_start_or_cross_native_function_boundary(self):
        data, rows = self.fixture()
        bad = copy.deepcopy(rows[:1])
        bad[0]['evidence']['native']['sequence']['offset'] += 7
        with self.assertRaisesRegex(ValueError, 'verified instruction position'):
            self.attest(data, bad)
        bad = copy.deepcopy(rows[:1])
        bad[0]['evidence']['native']['sequence']['fields'][-1]['hex'] = data[13:29].hex()
        with self.assertRaisesRegex(ValueError, 'inside its function scope'):
            self.attest(data, bad)

    def test_every_function_needs_native_code_region_end_and_correct_base(self):
        data, rows = self.fixture()
        for selected in (0, -1):
            bad = [copy.deepcopy(rows[selected])]
            bad[0]['evidence']['native'].pop('code_region')
            with self.subTest(selected=selected), self.assertRaisesRegex(ValueError, 'require.*code_region'):
                self.attest(data, bad)
        bad = copy.deepcopy(rows[-1:])
        bad[0]['evidence']['native']['code_region']['tag_hex'] = '41424344'
        with self.assertRaisesRegex(ValueError, 'region tag'):
            self.attest(data, bad)
        bad = copy.deepcopy(rows[:1])
        bad[0]['evidence']['native']['scope_range']['base'] = 0
        with self.assertRaisesRegex(ValueError, 'verified instruction base'):
            self.attest(data, bad)

    def test_scope_table_addresses_cannot_extend_sequences_past_actual_code_payload(self):
        data, rows = self.fixture()
        # Keep a non-final scope and all source bytes present, but shorten the
        # native code payload. Whole-file bounds would incorrectly admit it.
        raw = data[:4] + u32(6) + data[8:]
        with self.assertRaisesRegex(ValueError, 'actual function scope range'):
            self.attest(raw, rows[:1])

    def test_unknown_kinds_invalid_fields_and_boolean_offsets_are_rejected(self):
        data, rows = self.fixture()
        for mutation in ('kind', 'field', 'offset'):
            bad = copy.deepcopy(rows[:1])
            native = bad[0]['evidence']['native']
            if mutation == 'kind':
                native['kind'] = 'unknown_engine'
            elif mutation == 'field':
                native['sequence']['fields'][1]['kind'] = 'execute'
            else:
                native['sequence']['offset'] = True
            with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                self.attest(data, bad)

    def test_saved_scope_and_coordinate_must_uniquely_match_host_attestation(self):
        data, rows = self.fixture()
        canonical = self.canonical(data, rows)
        for scope, position in (('function_b', 7), ('function_a', 0), ('missing', 7)):
            save, link = self.saved(scope, position)
            with self.subTest(scope=scope, position=position), self.assertRaisesRegex(ValueError, 'matches=0'):
                adapter.resolve_native_node(save, link, canonical, **READERS)
        save, link = self.saved()
        duplicate = copy.deepcopy(canonical[0])
        duplicate['source'] = 'another.script'
        with self.assertRaisesRegex(ValueError, 'matches=2'):
            adapter.resolve_native_node(save, link, canonical + [duplicate], **READERS)

    def test_host_record_cannot_be_reused_on_another_dialogue_position(self):
        data, rows = self.fixture()
        canonical = self.canonical(data, rows)
        canonical[0]['position'] = 999
        save, link = self.saved()
        with self.assertRaisesRegex(ValueError, 'bound to another row'):
            adapter.resolve_native_node(save, link, canonical, **READERS)

    def test_rows_without_native_proofs_preserve_old_contract(self):
        data, rows = self.fixture()
        for row in rows:
            row['evidence'].pop('native')
        self.assertEqual(self.attest(data, rows), {})


if __name__ == '__main__':
    unittest.main()
