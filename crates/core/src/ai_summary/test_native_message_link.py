"""Synthetic native command links; no game/cache/model/generated program access."""
import copy
import importlib.util
from pathlib import Path
import struct
import unittest


spec = importlib.util.spec_from_file_location('native_link_evidence', Path(__file__).with_name('evidence.py'))
checks = importlib.util.module_from_spec(spec)
spec.loader.exec_module(checks)
PROFILE = 'willplus_ws2_name_message_config1_cp932'


class NativeMessageLinkTests(unittest.TestCase):
    def fixture(self, token='', native=0, message='X', prefix=b'prefix'):
        name_bytes = token.encode('cp932')
        name_offset = len(prefix)
        message_offset = name_offset + len(name_bytes) + 3
        data = prefix + b'\x15' + name_bytes + b'\0\0\x14' + struct.pack('<I', native)
        data += b'layer\0' + message.encode('cp932') + b'\0\0'
        record = dict(id=native, text=token or None,
            id_evidence=dict(offset=message_offset + 1, width=4, endian='little'),
            evidence=dict(offset=name_offset + 1, length=len(name_bytes) if token else 1,
                          encoding='cp932' if token else 'ascii'),
            native_link=dict(kind=PROFILE, name_command_offset=name_offset, message_command_offset=message_offset))
        return data, record, dict(values={}, speakers={'entry': record})

    def verify(self, data, output, resources=None):
        return checks.speaker_metadata(data, output, resources, require_native_link=True)[1]

    def rejected(self, data, output, fragment='native_link', resources=None):
        with self.assertRaisesRegex(ValueError, fragment) as caught:
            self.verify(data, output, resources)
        message = str(caught.exception)
        self.assertNotIn('PRIVATE_', message)
        self.assertLess(len(message.encode('utf-8')), 2048)
        return caught.exception

    def test_explicit_empty_name_accepts_id_zero_short_text_and_nonzero_ids(self):
        for native in (0, 81, 2**32 - 1):
            with self.subTest(native=native):
                data, _, output = self.fixture(native=native, message='Y')
                self.assertEqual(self.verify(data, output), {'entry': dict(id=native, text=None)})

    def test_native_prefix_token_keeps_distinct_verified_localized_name(self):
        data, record, output = self.fixture(token='%LFPRIVATE_NATIVE_TOKEN', native=39)
        resources = {'names': {'n': dict(id='%LFPRIVATE_NATIVE_TOKEN', text='PRIVATE_LOCALIZED_NAME')}}
        reference = record['evidence']
        record['text'] = 'PRIVATE_LOCALIZED_NAME'
        record['evidence'] = dict(resource='names', entry='n', reference=reference)
        self.assertEqual(self.verify(data, output, resources)['entry']['text'], 'PRIVATE_LOCALIZED_NAME')
        # Stripping a token prefix is not the same field and must fail.
        record['evidence']['reference']['offset'] += 3
        record['evidence']['reference']['length'] -= 3
        self.rejected(data, output, resources=resources)

    def test_scoped_null_and_named_records_cannot_omit_or_declare_unknown_link(self):
        for token in ('', 'PRIVATE_NAME'):
            for mutation in ('missing', 'unknown', 'wrong_fields'):
                with self.subTest(token=token, mutation=mutation):
                    data, record, output = self.fixture(token=token)
                    if mutation == 'missing':
                        del record['native_link']
                    elif mutation == 'unknown':
                        record['native_link']['kind'] = 'PRIVATE_' * 10000
                    else:
                        record['native_link']['arbitrary_value'] = 'PRIVATE_STORY'
                    self.rejected(data, output)

    def test_global_name_table_keeps_existing_proof_and_no_native_link_requirement(self):
        data = b'idPRIVATE_NAME'
        output = dict(values={}, speakers={'n': dict(id='id', text='PRIVATE_NAME',
            id_evidence=dict(offset=0, length=2, encoding='ascii'),
            evidence=dict(offset=2, length=12, encoding='ascii'))})
        self.assertEqual(checks.speaker_metadata(data, output)[1]['n']['text'], 'PRIVATE_NAME')
        self.rejected(data, output)
        self.assertEqual(checks.speaker_metadata(data, {'ordinary': 1}), ({'ordinary': 1}, {}))
        with self.assertRaisesRegex(ValueError, 'needs_proof'):
            self.verify(data, {'ordinary': 1})

    def test_arbitrary_nul_and_integer_without_name_command_do_not_become_narration(self):
        data = b'\x14' + struct.pack('<I', 0) + b'\0AB\0\0'
        record = dict(id=0, text=None, id_evidence=dict(offset=1, width=4, endian='little'),
            evidence=dict(offset=5, length=1, encoding='ascii'))
        output = dict(values={}, speakers={'entry': record})
        # This is the old independent-span shape; it is still usable globally.
        self.assertEqual(checks.speaker_metadata(data, output)[1]['entry']['text'], None)
        self.rejected(data, output)
        record['native_link'] = dict(kind=PROFILE, name_command_offset=0, message_command_offset=0)
        self.rejected(data, output)

    def test_name_and_message_must_be_the_same_adjacent_config_zero_chain(self):
        data, record, output = self.fixture(token='PRIVATE_NAME')
        for offset, byte in ((record['native_link']['name_command_offset'], 0x16),
                             (record['native_link']['message_command_offset'], 0x13),
                             (record['native_link']['message_command_offset'] - 1, 1),
                             (len(data) - 1, 1)):
            with self.subTest(offset=offset):
                corrupted = data[:offset] + bytes([byte]) + data[offset + 1:]
                self.rejected(corrupted, output)
        second, other, _ = self.fixture(token='PRIVATE_OTHER', native=1, prefix=b'')
        joined = data + second
        bad = copy.deepcopy(output)
        bad['speakers']['entry']['native_link']['message_command_offset'] = len(data) + other['native_link']['message_command_offset']
        self.rejected(joined, bad)

    def test_message_id_proof_is_exact_raw_dword_and_cannot_be_spliced(self):
        data, _, original = self.fixture(native=7)
        mutations = [dict(offset=0), dict(width=1), dict(width=4.0), dict(endian='big'),
                     dict(view=[dict(op='slice', offset=0, length=len(data))]), dict(encoding='ascii')]
        for patch in mutations:
            with self.subTest(patch=patch):
                bad = copy.deepcopy(original)
                bad['speakers']['entry']['id_evidence'].update(patch)
                self.rejected(data, bad)
        bad = copy.deepcopy(original)
        bad['speakers']['entry']['id'] = 8
        self.rejected(data, bad)

    def test_null_requires_its_actual_empty_field_and_named_reference_requires_full_field(self):
        for token in ('', 'PRIVATE_NAME'):
            data, record, original = self.fixture(token=token)
            for patch in (dict(offset=len(data) - 1), dict(length=0), dict(length=9999), dict(encoding='utf-16-le')):
                with self.subTest(token=token, patch=patch):
                    bad = copy.deepcopy(original)
                    bad['speakers']['entry']['evidence'].update(patch)
                    self.rejected(data, bad)
            bad = copy.deepcopy(original)
            bad['speakers']['entry']['text'] = 'PRIVATE_UNPROVEN' if not token else None
            self.rejected(data, bad)

    def test_raw_coordinate_space_cannot_be_replaced_by_transformed_output(self):
        data, record, output = self.fixture()
        output['view'] = [dict(op='slice', offset=0, length=len(data))]
        self.rejected(data, output)
        output.pop('view')
        record['evidence']['view'] = [dict(op='slice', offset=0, length=len(data))]
        self.rejected(data, output)

    def test_truncated_unterminated_oversized_and_empty_message_fields_are_rejected(self):
        data, _, output = self.fixture()
        for raw in (data[:-1], data[:-2], data[:-3], data[:10]):
            with self.subTest(length=len(raw)):
                self.rejected(raw, output)
        for token, message in (('A' * 1025, 'Y'), ('', 'A' * 32769), ('', '')):
            with self.subTest(name_length=len(token), text_length=len(message)):
                raw, _, bad = self.fixture(token=token, message=message)
                if len(token) > 1024:
                    bad['speakers']['entry']['text'] = 'PRIVATE_LOCALIZED_LABEL'
                self.rejected(raw, bad)
        raw, _, bad = self.fixture()
        raw = raw.replace(b'layer\0', b'L' * 257 + b'\0', 1)
        self.rejected(raw, bad)

    def test_offsets_must_be_native_nonnegative_integers_not_bool_or_outside_input(self):
        data, _, output = self.fixture()
        for key in ('name_command_offset', 'message_command_offset'):
            for value in (True, -1, 1.5, len(data), 'PRIVATE_OFFSET'):
                with self.subTest(key=key, value=value):
                    bad = copy.deepcopy(output)
                    bad['speakers']['entry']['native_link'][key] = value
                    self.rejected(data, bad)

    def test_explicit_bad_link_is_not_ignored_outside_scoped_mode(self):
        data, record, output = self.fixture()
        record['native_link']['kind'] = 'PRIVATE_UNKNOWN'
        with self.assertRaisesRegex(ValueError, 'Unsupported native_link'):
            checks.speaker_metadata(data, output)


if __name__ == '__main__':
    unittest.main()
