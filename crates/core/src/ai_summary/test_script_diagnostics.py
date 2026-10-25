"""Script proof failures identify row/field without exposing dialogue or names."""
import json
import pathlib
import runpy
import struct
import unittest


checks = runpy.run_path(str(pathlib.Path(__file__).with_name('evidence.py')))


def span(offset, text, encoding='ascii'):
    return dict(offset=offset, length=len(text.encode(encoding)), encoding=encoding)


class ScriptDiagnosticsTests(unittest.TestCase):
    def length_array(self, key=7):
        texts = ['', 'First private line', 'Second private line']
        records = [(text + '\0').encode('ascii') for text in texts]
        pool = b''.join(records)
        data = struct.pack('<I3H', len(texts), *(len(record) for record in records))
        data += bytes(value ^ key for value in pool)
        table = dict(kind='length_array', offset=4, length=6, count=dict(offset=0),
                     length_width=2, base=10, pool_length=len(pool), encoding='ascii',
                     terminator_width=1, pool_view=[dict(op='xor', key=format(key, '02x'))])
        rows = [dict(position=index, text=texts[index], speaker=None,
                     evidence=dict(table='translated', index=index, reference=dict(mode='ordinal')))
                for index in (1, 2)]
        return data, dict(rows=rows, tables={'translated': table})

    def scoped(self):
        data, output = self.length_array()
        resources = {'messages:chapter': {
            'message-1': dict(id=1, text='Private displayed name', _source='chapter.lng'),
            'message-2': dict(id=2, text=None, _source='chapter.lng')}}
        for row in output['rows']:
            index = row['evidence']['index']
            row['speaker'] = resources['messages:chapter']['message-' + str(index)]['text']
            row['evidence']['speaker'] = dict(resource='messages:chapter', entry='message-' + str(index),
                                               reference=dict(mode='table_index'))
        return data, output, resources

    def failure(self, data, output, **kwargs):
        with self.assertRaises(checks['ScriptRowEvidenceError']) as caught:
            checks['script_rows'](data, output, **kwargs)
        error = caught.exception
        self.assertIsInstance(error, checks['SpeakerMetadataError'])
        self.assertEqual(error.diagnostic['row_index'], int(error.diagnostic['path'].split(']')[0][5:]))
        self.assertNotIn('entry_index', error.diagnostic)
        for secret in ('First private line', 'Second private line', 'Private displayed name'):
            self.assertNotIn(secret, str(error))
            self.assertNotIn(secret, json.dumps(error.diagnostic))
        return error

    def test_unlinked_raw_span_mismatch_points_to_table_link_and_pool_view(self):
        data, output = self.length_array()
        row = output['rows'][0]
        row['position'], row['evidence'] = 11, span(11, row['text'])
        error = self.failure(data, output)
        self.assertEqual(error.diagnostic['path'], 'rows[0].evidence')
        self.assertEqual(error.diagnostic['position'], 11)
        self.assertIn('evidence.table/index is missing', str(error))
        self.assertIn('table.pool_view is not automatically applied', str(error))
        self.assertIn('independently proved to be a table index', str(error))
        self.assertIn('reference:{mode:"ordinal"}', str(error))
        self.assertNotIn('stripping or normalization', str(error))
        self.assertNotIn('table', row['evidence'])  # Failed output is not repaired for the caller.

    def test_unlinked_raw_decode_failure_gets_same_actionable_context_without_bytes(self):
        data, output = self.length_array(key=128)
        row = output['rows'][0]
        row['position'], row['evidence'] = 11, span(11, row['text'])
        error = self.failure(data, output)
        self.assertEqual(error.diagnostic['path'], 'rows[0].evidence')
        self.assertIn('Invalid text byte evidence or encoding', str(error))
        self.assertIn('table.pool_view', str(error))
        self.assertNotIn('0xc6', str(error))

    def test_indexed_ordinal_still_passes_and_wrong_index_is_not_corrected(self):
        data, output = self.length_array()
        self.assertEqual(checks['script_rows'](data, output), output['rows'])
        output['rows'][0]['evidence']['index'] = 2
        error = self.failure(data, output)
        self.assertEqual(error.diagnostic['path'], 'rows[0].position')
        self.assertIn('verified table ordinal', str(error))

    def test_valid_direct_spans_with_declared_tables_still_pass(self):
        data, output = self.length_array()
        text = output['rows'][0]['text']
        raw = data + text.encode('ascii')
        row = dict(position=len(data), text=text, speaker=None, evidence=span(len(data), text))
        output['rows'] = [row]
        self.assertEqual(checks['script_rows'](raw, output), [row])

    def test_valid_direct_span_with_explicit_pool_view_still_passes(self):
        data, output = self.length_array()
        row = output['rows'][0]
        row['position'] = 1
        row['evidence'] = dict(span(1, row['text']), view=[
            dict(op='slice', offset=10, length=output['tables']['translated']['pool_length']),
            dict(op='xor', key='07')])
        output['rows'] = [row]
        self.assertEqual(checks['script_rows'](data, output), [row])

    def test_known_instruction_regions_keep_operand_requirement(self):
        text = 'First private line'
        data = struct.pack('<III', 0, 1, len(text)) + text.encode('ascii')
        table = dict(offset=4, length=len(data) - 4, fields=[dict(kind='text', encoding='ascii')])
        row = dict(position=12, text=text, speaker=None, evidence=span(12, text))
        output = dict(rows=[row], tables={'instructions': table})
        required = [dict(offset=4, length=len(data) - 4)]
        error = self.failure(data, output, required_table_regions=required)
        self.assertEqual(error.diagnostic['path'], 'rows[0].evidence')
        self.assertIn('actual instruction operand reference', str(error))
        self.assertIn('ordinal mode cannot replace it', str(error))
        self.assertNotIn('If the native message coordinate', str(error))
        row['position'] = 0
        row['evidence'] = dict(table='instructions', index=0, reference=dict(operand_offset=0))
        self.assertEqual(checks['script_rows'](data, output, required_table_regions=required), [row])
        row['evidence']['reference'] = dict(mode='ordinal')
        error = self.failure(data, output, required_table_regions=required)
        self.assertEqual(error.diagnostic['path'], 'rows[0].evidence.reference')

    def test_stale_scoped_resource_alias_is_reported_before_null_name(self):
        data, output, resources = self.scoped()
        row = output['rows'][0]
        row['speaker'] = None
        row['evidence']['speaker']['resource'] = 'messages'
        error = self.failure(data, output, speaker_resources=resources, source='chapter.lng')
        self.assertEqual(error.diagnostic['path'], 'rows[0].evidence.speaker.resource')
        self.assertIn('target_script', str(error))
        self.assertEqual(error.diagnostic['observed']['available_resources'], [dict(
            resource='messages:chapter', records=2, native_ids=[1, 2], id_types=['int'])])
        self.assertNotIn('do not silently replace', str(error))

    def test_other_script_resource_is_not_suggested_as_matching_scope(self):
        data, output, resources = self.scoped()
        resources['messages:other'] = {'message-1': dict(id=1, text='Other secret name', _source='other.lng')}
        output['rows'][0]['evidence']['speaker']['resource'] = 'messages:other'
        error = self.failure(data, output, speaker_resources=resources, source='chapter.lng')
        self.assertEqual(error.diagnostic['path'], 'rows[0].evidence.speaker.resource')
        self.assertEqual([r['resource'] for r in error.diagnostic['observed']['available_resources']], ['messages:chapter'])
        self.assertNotIn('Other secret name', str(error))

    def test_missing_entry_and_wrong_message_id_locate_entry(self):
        for entry in ('missing', 'message-2'):
            with self.subTest(entry=entry):
                data, output, resources = self.scoped()
                output['rows'][0]['evidence']['speaker']['entry'] = entry
                error = self.failure(data, output, speaker_resources=resources, source='chapter.lng')
                self.assertEqual(error.diagnostic['path'], 'rows[0].evidence.speaker.entry')
                if entry == 'message-2':
                    self.assertEqual(error.diagnostic['observed']['native_index'], 1)
                    self.assertEqual(error.diagnostic['observed']['cited_native_id'], 2)

    def test_scoped_named_and_actual_empty_narration_records_still_pass(self):
        data, output, resources = self.scoped()
        self.assertEqual(checks['script_rows'](data, output, speaker_resources=resources, source='chapter.lng'), output['rows'])
        output['rows'][0]['speaker'] = None
        error = self.failure(data, output, speaker_resources=resources, source='chapter.lng')
        self.assertEqual(error.diagnostic['path'], 'rows[0].speaker')

    def test_required_empty_target_scope_cannot_accept_unresolved_null_rows(self):
        data, output = self.length_array()
        error = self.failure(data, output, speaker_resources={'messages:chapter': {}},
                             source='chapter.lng', required_speaker_scope=True)
        self.assertEqual(error.category, 'speaker_metadata')
        self.assertEqual(error.diagnostic['category'], 'speaker_metadata')
        self.assertEqual(error.diagnostic['path'], 'rows[0].evidence.speaker')
        self.assertEqual(error.diagnostic['observed']['scoped_records'], 0)
        self.assertIn('Repair and reload', str(error))
        self.assertIn('this exact target_script', str(error))
        self.assertIn('actual empty-name NUL evidence', str(error))

    def test_other_script_and_global_names_cannot_mask_required_empty_scope(self):
        data, output = self.length_array()
        resources = {'messages:chapter': {},
                     'names': {'name-7': dict(id=7, text='Private displayed name')},
                     'messages:other': {'message-1': dict(id=1, text='Other secret name', _source='other.lng')}}
        for cite in (False, True):
            with self.subTest(cite=cite):
                if cite:
                    output['rows'][0]['speaker'] = 'Other secret name'
                    output['rows'][0]['evidence']['speaker'] = dict(resource='messages:other', entry='message-1',
                                                                     reference=dict(mode='table_index'))
                error = self.failure(data, output, speaker_resources=resources, source='chapter.lng',
                                     required_speaker_scope=True)
                self.assertEqual(error.category, 'speaker_metadata')
                self.assertNotIn('Other secret name', str(error))

    def test_required_empty_scope_cannot_be_bypassed_with_a_direct_span(self):
        row = dict(position=0, text='First private line', speaker=None, evidence=span(0, 'First private line'))
        error = self.failure(b'First private line', [row], speaker_resources={'messages:chapter': {}},
                             source='chapter.lng', required_speaker_scope=True)
        self.assertEqual(error.category, 'speaker_metadata')

    def test_required_nonempty_scope_preserves_named_and_proved_narration(self):
        data, output, resources = self.scoped()
        self.assertEqual(checks['script_rows'](data, output, speaker_resources=resources, source='chapter.lng',
                                             required_speaker_scope=True), output['rows'])
        output['rows'][1]['evidence'].pop('speaker')
        error = self.failure(data, output, speaker_resources=resources, source='chapter.lng', required_speaker_scope=True)
        self.assertEqual(error.diagnostic['path'], 'rows[1].evidence.speaker')
        self.assertNotIn('category', error.diagnostic)

    def test_loaded_scope_cannot_be_bypassed_by_switching_to_direct_spans(self):
        data, output, resources = self.scoped()
        text = output['rows'][0]['text']
        row = dict(position=len(data), text=text, speaker=None, evidence=span(len(data), text))
        for required in (False, True):
            with self.subTest(required=required):
                error = self.failure(data + text.encode(), dict(output, rows=[row]),
                                     speaker_resources=resources, source='chapter.lng', required_speaker_scope=required)
                self.assertIn('direct spans cannot bypass', str(error))
                self.assertNotIn('category', error.diagnostic)

    def test_without_a_required_scope_existing_unresolved_formats_still_pass(self):
        data, output = self.length_array()
        for resources in ({}, {'analysis:chapter': {}},
                          {'messages:other': {'message-1': dict(id=1, text='Other secret name', _source='other.lng')}}):
            with self.subTest(keys=list(resources)):
                self.assertEqual(checks['script_rows'](data, output, speaker_resources=resources, source='chapter.lng'),
                                 output['rows'])

    def test_scope_with_nonzero_first_id_excludes_auxiliary_without_renumbering(self):
        _, output, resources = self.scoped()
        texts = ['Private UI label', 'First private line', 'Second private line']
        records = [text.encode() + b'\0' for text in texts]
        pool = b''.join(records)
        data = struct.pack('<I3H', 3, *(len(record) for record in records)) + bytes(value ^ 7 for value in pool)
        output['tables']['translated']['pool_length'] = len(pool)
        self.assertEqual(checks['script_rows'](data, output, speaker_resources=resources, source='chapter.lng',
                                             required_speaker_scope=True), output['rows'])
        self.assertEqual([row['position'] for row in output['rows']], [1, 2])
        auxiliary = dict(position=0, text=texts[0], speaker=None,
                         evidence=dict(table='translated', index=0, reference=dict(mode='ordinal')))
        output['rows'].insert(0, auxiliary)
        error = self.failure(data, output, speaker_resources=resources, source='chapter.lng', required_speaker_scope=True)
        self.assertIn('Missing speaker/narration proof', str(error))
        self.assertEqual(error.diagnostic['observed']['native_index'], 0)

    def test_global_inline_scalar_id_still_passes_with_other_script_resources(self):
        data = b'\x07First private line'
        row = dict(position=1, text='First private line', speaker='Private displayed name',
                   evidence=dict(span(1, 'First private line'), speaker=dict(
                       resource='global', entry='name-7', reference=dict(offset=0, width=1))))
        resources = {'global': {'name-7': dict(id=7, text='Private displayed name')},
                     'messages:other': {'message-1': dict(id=1, text=None, _source='other.lng')}}
        self.assertEqual(checks['script_rows'](data, [row], speaker_resources=resources, source='chapter.lng'), [row])
        row['evidence']['speaker']['reference']['offset'] = 1
        error = self.failure(data, [row], speaker_resources=resources, source='chapter.lng')
        self.assertEqual(error.diagnostic['path'], 'rows[0].evidence.speaker.reference')

    def test_second_row_missing_reference_has_its_own_locator(self):
        data, output = self.length_array()
        output['rows'][1]['evidence'].pop('reference')
        error = self.failure(data, output)
        self.assertEqual(error.diagnostic['path'], 'rows[1].evidence.reference')
        self.assertEqual(error.diagnostic['position'], 2)

    def test_direct_position_error_expects_a_byte_coordinate(self):
        row = dict(position=1, text='First private line', speaker=None, evidence=span(0, 'First private line'))
        error = self.failure(b'First private line', [row])
        self.assertEqual(error.diagnostic['path'], 'rows[0].position')
        self.assertIn('direct evidence.offset byte coordinate', error.diagnostic['expected'])
        self.assertNotIn('text span {', error.diagnostic['expected'])

    def test_malformed_row_fields_have_precise_paths(self):
        for mutate, field in ((lambda r: r.update(position='native'), 'position'),
                              (lambda r: r.update(text=''), 'text'),
                              (lambda r: r.pop('speaker'), 'speaker'),
                              (lambda r: r.update(evidence=None), 'evidence'),
                              (lambda r: r['evidence'].update(index=999), 'evidence.index')):
            with self.subTest(field=field):
                data, output = self.length_array()
                mutate(output['rows'][0])
                error = self.failure(data, output)
                self.assertEqual(error.diagnostic['path'], 'rows[0].' + field)


if __name__ == '__main__':
    unittest.main()
