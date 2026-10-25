"""Byte-backed companion failures locate the exact field without leaking names."""
import copy
import pathlib
import runpy
import struct
import unittest


checks = runpy.run_path(str(pathlib.Path(__file__).with_name('evidence.py')))


def span(offset, value, encoding='ascii'):
    return dict(offset=offset, length=len(value.encode(encoding)), encoding=encoding)


class SpeakerDiagnosticsTests(unittest.TestCase):
    def global_name(self):
        native, display = 'NATIVE_TOKEN', '本地化姓名'
        data = native.encode('ascii') + b'|' + display.encode('utf-8')
        entry = dict(id=native, text=display, id_evidence=span(0, native),
                     evidence=span(len(native) + 1, display, 'utf-8'))
        values, records = checks['speaker_metadata'](data, dict(values={'format': 'toy'}, speakers={'name-0': entry}))
        self.assertEqual(values, {'format': 'toy'})
        return native, display, records

    def chain(self):
        native, display, records = self.global_name()
        raw = struct.pack('<I', 11) + native.encode('ascii') + b'\0'
        entry = dict(id=11, text=display, id_evidence=dict(offset=0, width=4),
                     evidence=dict(resource='global-names', entry='name-0', reference=span(4, native)))
        return raw, entry, {'global-names': records}, native, display

    def failure(self, raw, entry, resources=None, key='message-11'):
        with self.assertRaises(ValueError) as caught:
            checks['speaker_metadata'](raw, dict(values={}, speakers={key: entry}), resources)
        error = caught.exception
        self.assertIsInstance(error, checks['SpeakerMetadataError'])
        return error

    def test_direct_names_and_localized_global_to_message_chain_keep_verified_values(self):
        direct = dict(id=7, text='Displayed', id_evidence=dict(offset=0, width=1), evidence=span(1, 'Displayed'))
        _, records = checks['speaker_metadata'](b'\x07Displayed', dict(values={}, speakers={'name-7': direct}))
        self.assertEqual(records, {'name-7': dict(id=7, text='Displayed')})
        raw, entry, resources, _, display = self.chain()
        _, messages = checks['speaker_metadata'](raw, dict(values={}, speakers={'message-11': entry}), resources)
        self.assertEqual(messages, {'message-11': dict(id=11, text=display)})

    def test_empty_narration_requires_actual_nul_and_native_message_scalar(self):
        raw = struct.pack('<I', 12) + b'\0'
        entry = dict(id=12, text=None, evidence=dict(offset=4, length=1, encoding='ascii'),
                     id_evidence=dict(offset=0, width=4))
        _, records = checks['speaker_metadata'](raw, dict(values={}, speakers={'message-12': entry}))
        self.assertEqual(records['message-12'], dict(id=12, text=None))
        invalid = copy.deepcopy(entry)
        invalid['evidence']['length'] = 0
        error = self.failure(raw, invalid)
        self.assertIn('Narration needs the actual NUL', str(error))
        self.assertEqual(error.diagnostic['path'], 'speakers["message-11"].evidence')
        self.assertIn('zero-length span', error.diagnostic['expected'])

    def test_localized_name_cannot_use_direct_original_name_bytes_and_error_explains_chain(self):
        raw, entry, resources, native, display = self.chain()
        entry['evidence'] = span(4, native)
        error = self.failure(raw, entry, resources)
        self.assertIn('Companion speaker name does not match its native bytes', str(error))
        self.assertEqual(error.diagnostic['path'], 'speakers["message-11"].evidence')
        self.assertIn('localized name-table entry', str(error))
        self.assertIn('reference:<native token scalar or text span', str(error))
        self.assertIn('metadata["speaker_resources"][resource][entry]={id,text}', str(error))
        self.assertNotIn(native, str(error))
        self.assertNotIn(display, str(error))

    def test_native_integer_uses_width_not_a_fake_text_encoding(self):
        raw, entry, resources, native, display = self.chain()
        entry['id_evidence'] = dict(offset=0, length=4, encoding='uint32-le')
        error = self.failure(raw, entry, resources)
        self.assertIn('Invalid text byte evidence or encoding', str(error))
        self.assertEqual(error.diagnostic['path'], 'speakers["message-11"].id_evidence.encoding')
        self.assertIn('width:1|2|3|4|8', str(error))
        self.assertIn('never uint32-le', str(error))
        self.assertNotIn(native, str(error))
        self.assertNotIn(display, str(error))

    def test_prose_is_not_byte_proof_and_missing_id_proof_has_its_own_field(self):
        raw = struct.pack('<I', 11) + b'Displayed'
        entry = dict(id=11, text='Displayed', evidence='name at byte four', id_evidence=dict(offset=0, width=4))
        error = self.failure(raw, entry)
        self.assertEqual(error.diagnostic['path'], 'speakers["message-11"].evidence')
        self.assertIn('text span {offset:', str(error))
        entry['evidence'] = span(4, 'Displayed')
        entry['id_evidence'] = 'native integer at zero'
        error = self.failure(raw, entry)
        self.assertEqual(error.diagnostic['path'], 'speakers["message-11"].id_evidence')
        self.assertIn('native scalar', str(error))

    def test_chain_failures_identify_resource_entry_or_native_reference_without_names(self):
        raw, original, resources, native, display = self.chain()
        for field, value, suffix in (('resource', 'unloaded-resource', 'evidence.resource'),
                                     ('entry', 'missing-entry', 'evidence.entry'),
                                     ('reference', dict(offset=4, length=1, encoding='ascii'), 'evidence.reference'),
                                     ('reference', dict(mode='table_index'), 'evidence.reference.mode')):
            with self.subTest(field=field, suffix=suffix):
                entry = copy.deepcopy(original)
                entry['evidence'][field] = value
                error = self.failure(raw, entry, resources)
                self.assertEqual(error.diagnostic['path'], 'speakers["message-11"].' + suffix)
                self.assertIn('there is no extra speakers nesting', str(error))
                self.assertNotIn(native, str(error))
                self.assertNotIn(display, str(error))

    def test_bad_table_ordinal_retains_primary_error_and_identifies_index(self):
        name = 'Displayed'
        raw = struct.pack('<II', 1, len(name)) + name.encode('ascii')
        output = dict(values={}, tables={'names': dict(offset=0, length=len(raw),
                      fields=[dict(kind='text', encoding='ascii')])},
                      speakers={'name-0': dict(id=0, text=name, evidence=dict(table='names', index=2))})
        with self.assertRaises(ValueError) as caught:
            checks['speaker_metadata'](raw, output)
        self.assertIn('valid complete-table ordinal', str(caught.exception))
        self.assertEqual(caught.exception.diagnostic['path'], 'speakers["name-0"].evidence.index')

    def test_name_keys_are_redacted_but_record_index_and_field_remain_precise(self):
        raw, entry, resources, native, display = self.chain()
        entry['evidence'] = span(4, native)
        error = self.failure(raw, entry, resources, key=display)
        self.assertEqual(error.diagnostic['entry_index'], 0)
        self.assertIn('speakers[entry_index=0,key_sha256=', error.diagnostic['path'])
        self.assertTrue(error.diagnostic['path'].endswith('].evidence'))
        self.assertNotIn(display, str(error))
        self.assertNotIn(native, str(error))

    def test_multiple_entries_locate_the_actual_failed_record(self):
        raw = struct.pack('<I', 0) + b'\0'
        correct = dict(id=0, text=None, id_evidence=dict(offset=0, width=4),
                       evidence=dict(offset=4, length=1, encoding='ascii'))
        invalid = copy.deepcopy(correct)
        invalid['id_evidence']['offset'] = 100
        with self.assertRaises(ValueError) as caught:
            checks['speaker_metadata'](raw, dict(values={}, speakers={'message-0': correct, 'message-1': invalid}))
        error = caught.exception
        self.assertEqual(error.diagnostic['entry_index'], 1)
        self.assertEqual(error.diagnostic['path'], 'speakers["message-1"].id_evidence')
        self.assertEqual(error.diagnostic['observed']['offset'], 100)


if __name__ == '__main__':
    unittest.main()
