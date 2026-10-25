"""Independent synthetic PSB/TJS documents; no installed game or model calls."""
import copy
import pathlib
import runpy
import struct
import unittest

import typed_evidence as typed


EVIDENCE = runpy.run_path(str(pathlib.Path(__file__).with_name('evidence.py')))


def packed(values):
    count_width = max(1, (len(values).bit_length()+7)//8)
    width = max(1, (max(values, default=0).bit_length()+7)//8)
    return bytes([12+count_width])+len(values).to_bytes(count_width, 'little')+bytes([12+width])+b''.join(
        value.to_bytes(width, 'little') for value in values)


def synthetic_psb(value, version=3):
    """Small writer intentionally separate from the reader's parsing machinery."""
    keys, strings = [], []
    def inventory(item):
        if isinstance(item, dict):
            for key, child in item.items():
                if key not in keys:
                    keys.append(key)
                inventory(child)
        elif isinstance(item, list):
            for child in item:
                inventory(child)
        elif isinstance(item, str) and item not in strings:
            strings.append(item)
    inventory(value)
    trie = {}
    for key in keys:
        node = trie
        for byte in key.encode():
            node = node.setdefault(byte, {})
        node[0] = None
    bases, parents = [0], [0]
    terminals = {}
    def grow(index):
        if index >= len(bases):
            bases.extend([0]*(index-len(bases)+1))
            parents.extend([0]*(index-len(parents)+1))
    def write_trie(node, index, prefix):
        base = len(bases)+1
        bases[index] = base
        grow(base+max(node, default=0))
        for byte, child in node.items():
            target = base+byte
            parents[target] = index
            if byte == 0:
                terminals[prefix.decode()] = target
            else:
                write_trie(child, target, prefix+bytes([byte]))
    if trie:
        write_trie(trie, 0, b'')
    names = packed(bases)+packed(parents)+packed([terminals[key] for key in keys])
    def serialize(item):
        if item is None:
            return b'\x01'
        if type(item) is bool:
            return b'\x03' if item else b'\x02'
        if type(item) is int:
            if item == 0:
                return b'\x04'
            width = next(width for width in range(1, 9) if -(2**(width*8-1)) <= item < 2**(width*8-1))
            return bytes([4+width])+item.to_bytes(width, 'little', signed=True)
        if isinstance(item, str):
            index = strings.index(item)
            width = max(1, (index.bit_length()+7)//8)
            return bytes([20+width])+index.to_bytes(width, 'little')
        sequence = item.values() if isinstance(item, dict) else item
        parts, offsets, cursor = [], [], 0
        for child in sequence:
            part = serialize(child)
            offsets.append(cursor)
            parts.append(part)
            cursor += len(part)
        if isinstance(item, dict):
            return b'\x21'+packed([keys.index(key) for key in item])+packed(offsets)+b''.join(parts)
        return b'\x20'+packed(offsets)+b''.join(parts)
    body = serialize(value)
    offsets, pool = [], b''
    for string in strings:
        offsets.append(len(pool))
        pool += string.encode()+b'\0'
    header_length = 44 if version == 3 else 40
    names_start = header_length
    root_start = names_start+len(names)
    strings_start = root_start+len(body)
    string_offsets = packed(offsets)
    pool_start = strings_start+len(string_offsets)
    chunks_start = pool_start+len(pool)
    empty = packed([])
    lengths_start = chunks_start+len(empty)
    payload_start = lengths_start+len(empty)
    header = b'PSB\0'+struct.pack('<HH8I', version, 0, header_length, names_start, strings_start,
                                  pool_start, chunks_start, lengths_start, payload_start, root_start)
    if version == 3:
        header += b'\0'*4
    return header+names+body+string_offsets+pool+empty+empty


def field(path, kind='string', document=None):
    return dict(kind='typed_field', document=document or dict(syntax='psb', view=[]), path=path, type=kind)


class TypedEvidenceTests(unittest.TestCase):
    def transcript(self, value=None):
        value = value or {'messages': [{'scope': 'branch_one', 'coordinate': 17,
                                        'words': 'Repeated text', 'name': 'Reader'},
                                       {'scope': 'branch_two', 'coordinate': 29,
                                        'words': 'Repeated text', 'name': None}]}
        raw = synthetic_psb(value)
        rows = []
        for index, record in enumerate(value['messages']):
            path = ['messages', index]
            proof = field(path+['words'])
            proof['speaker'] = field(path+['name'])
            proof['native'] = dict(kind='typed_record', record=path,
                scope=path+['scope'], coordinate=path+['coordinate'], text=path+['words'], speaker=path+['name'])
            selected = typed.replay_field(raw, field(path+['words']))
            rows.append(dict(position=selected['span']['offset'], text=record['words'],
                             speaker=record['name'], evidence=proof))
        return raw, rows

    def test_complete_psb_versions_types_and_unicode_names(self):
        value = {'chapters': [{'说话人': 'Person', 'words': 'Example', 'coordinate': 310}],
                 'nested': [False, True, None, -123, 0, 2**50]}
        for version in (2, 3):
            with self.subTest(version=version):
                raw = synthetic_psb(value, version)
                parsed = typed.parse_document(raw, dict(syntax='psb'))
                self.assertEqual(typed.plain(parsed['root']), value)
                self.assertEqual(typed.replay_field(raw, field(['chapters', 0, 'coordinate'], 'integer'))['value'], 310)

    def test_equal_strings_have_distinct_native_reference_coordinates(self):
        raw = synthetic_psb({'messages': [['Same'], ['Same']]})
        first = typed.replay_field(raw, field(['messages', 0, 0]))
        second = typed.replay_field(raw, field(['messages', 1, 0]))
        self.assertEqual(first['value'], second['value'])
        self.assertEqual(first['node']['string_span'], second['node']['string_span'])
        self.assertNotEqual(first['span'], second['span'])
        self.assertNotEqual(first['span']['offset'], first['node']['string_span']['offset'])

    def test_tjs_and_psb_share_typed_paths_but_not_scalar_coercions(self):
        raw = b'%["records"=>[%["location"=>7,"text"=>"Plain"]]]'
        document = dict(syntax='tjs_const', encoding='ascii')
        self.assertEqual(typed.replay_field(raw, field(['records', 0, 'text'], document=document))['value'], 'Plain')
        for path in (['records', True, 'text'], ['records', '0', 'text'], ['missing']):
            with self.subTest(path=path), self.assertRaises(ValueError):
                typed.replay_field(raw, field(path, document=document))
        with self.assertRaises(ValueError):
            typed.replay_field(raw, field(['records', 0, 'location'], document=document))

    def test_no_partial_root_or_pool_field_can_replace_typed_reference(self):
        raw = synthetic_psb({'message': 'Literal'})
        proof = field(['message'])
        for change in (dict(offset=0), dict(path=['pool', 0]), dict(type='integer')):
            with self.subTest(change=change), self.assertRaises(ValueError):
                typed.replay_field(raw, dict(proof, **change))
        self.assertRaises(ValueError, typed.parse_document, raw[:-2], dict(syntax='psb'))

    def test_unknown_or_encrypted_headers_and_conflicting_offsets_fail(self):
        raw = synthetic_psb({'message': 'Native'})
        for offset, replacement in ((4, b'\x04\0'), (6, b'\x01\0'), (8, b'\xff'*4),
                                    (36, b'\0'*4)):
            changed = raw[:offset]+replacement+raw[offset+len(replacement):]
            with self.subTest(offset=offset), self.assertRaises(ValueError):
                typed.parse_document(changed, dict(syntax='psb'))

    def test_mutating_returned_tree_cannot_change_future_host_parse(self):
        raw = synthetic_psb({'message': 'Original'})
        tree = typed.parse_document(raw, dict(syntax='psb'))
        tree['root']['value']['message']['value'] = 'Altered'
        selected = typed.replay_field(raw, field(['message']))
        selected['node']['value'] = 'Altered again'
        self.assertEqual(typed.replay_field(raw, field(['message']))['value'], 'Original')

    def test_bounded_xor_range_preserves_unselected_bytes_and_rejects_malformed_ranges(self):
        codecs = runpy.run_path(str(pathlib.Path(__file__).with_name('bytecodecs.py')))
        raw = synthetic_psb({'message': 'Synthetic'})
        step = dict(op='xor_range', offset=9, length=len(raw)-15, key='1234')
        encoded = codecs['view'](raw, [step])
        self.assertEqual(encoded[:9], raw[:9])
        self.assertEqual(encoded[-6:], raw[-6:])
        self.assertEqual(codecs['view'](encoded, [step]), raw)
        for change in (dict(offset=True), dict(length=-1), dict(length=0), dict(offset=len(raw)),
                       dict(key=''), dict(extra=1)):
            with self.subTest(change=change), self.assertRaises(ValueError):
                codecs['view'](raw, [dict(step, **change)])

    def test_script_rows_verify_typed_text_and_speaker_references(self):
        raw, rows = self.transcript()
        self.assertEqual(EVIDENCE['script_rows'](raw, dict(rows=rows)), rows)
        changed = copy.deepcopy(rows)
        changed[0]['speaker'] = 'Invented'
        with self.assertRaisesRegex(ValueError, 'Speaker name lacks matching'):
            EVIDENCE['script_rows'](raw, dict(rows=changed))
        changed = copy.deepcopy(rows)
        selected = typed.replay_field(raw, field(['messages', 0, 'words']))
        changed[0]['position'] = selected['node']['string_span']['offset']
        with self.assertRaisesRegex(ValueError, 'reference-node offset'):
            EVIDENCE['script_rows'](raw, dict(rows=changed))

    def test_psb_cannot_downgrade_to_string_pool_span_ordinal_or_sliced_text(self):
        raw, rows = self.transcript()
        selected = typed.replay_field(raw, field(['messages', 0, 'words']))
        direct = dict(position=selected['node']['string_span']['offset'], text='Repeated text', speaker=None,
                      evidence=selected['node']['string_span'])
        with self.assertRaisesRegex(ValueError, 'string-pool byte span'):
            EVIDENCE['script_rows'](raw, [direct])
        sliced = dict(rows=[dict(position=0, text='Repeated text', speaker=None,
                                evidence=dict(offset=0, length=13, encoding='utf-8'))],
                      view=[dict(op='slice', offset=selected['node']['string_span']['offset'], length=13)])
        with self.assertRaisesRegex(ValueError, 'complete checksum-verified member'):
            EVIDENCE['script_rows'](raw, sliced)

    def test_full_member_filter_then_typed_binding_uses_one_coordinate_space(self):
        raw, rows = self.transcript()
        steps = [dict(op='xor_range', offset=0, length=17, key='35'),
                 dict(op='xor_range', offset=17, length=len(raw)-17, key='7e19')]
        encoded = EVIDENCE['view'](raw, steps)
        self.assertEqual(EVIDENCE['script_rows'](encoded, dict(rows=rows, member_filter=steps)), rows)

    def test_independent_script_attestation_and_saved_tuple_join_preserve_recorded_role(self):
        raw, rows = self.transcript()
        EVIDENCE['script_rows'](raw, dict(rows=rows))
        native = EVIDENCE['ADAPTERS']['attest_native_nodes'](raw, rows, span=EVIDENCE['span'],
            integer=EVIDENCE['integer'], view=EVIDENCE['view'])
        canonical = [dict(source='chapter.script', position=row['position'], text=row['text'], speaker=row['speaker'],
                          _native=native[row['position']]) for row in rows]
        save = b'%["snapshot"=>%["scope"=>"branch_two","point"=>29,"caption"=>"Repeated text"]]'
        link = dict(kind='typed_record', document=dict(syntax='tjs_const', encoding='ascii'), record=['snapshot'],
                    scope=['scope'], coordinate=['point'], caption=['caption'], state_role='recorded_node')
        result = typed.resolve_record(save, link, canonical)
        self.assertEqual(result['row'], canonical[1])
        self.assertEqual(result['state_role'], 'recorded_node')
        self.assertFalse(result['current_verified'])
        facts = dict(source='chapter.script', line=canonical[1]['position'], anchors=['Repeated text'],
                     evidence=dict(node_link=link))
        with self.assertRaisesRegex(ValueError, 'engine current/choice/resume role is unverified'):
            EVIDENCE['save_facts'](save, facts, canonical)
        for role in ('current', 'current_record', 'resume', 'last_displayed', 'unknown', True):
            with self.subTest(role=role), self.assertRaisesRegex(ValueError, 'explicit recorded_node'):
                typed.resolve_record(save, dict(link, state_role=role), canonical)
        with self.assertRaisesRegex(ValueError, 'matches=0'):
            typed.resolve_record(save.replace(b'29', b'28'), link, canonical)
        with self.assertRaisesRegex(ValueError, 'matches=2'):
            typed.resolve_record(save, link, canonical+[copy.deepcopy(canonical[1])])

    def test_spliced_typed_fields_or_model_native_attestation_are_not_accepted(self):
        raw, rows = self.transcript()
        declaration = copy.deepcopy(rows[0]['evidence']['native'])
        declaration['coordinate'] = ['messages', 1, 'coordinate']
        with self.assertRaisesRegex(ValueError, 'same complete native record'):
            typed.attest_record(raw, rows[0], declaration)
        declaration = copy.deepcopy(rows[0]['evidence']['native'])
        declaration['text'] = ['messages', 1, 'words']
        with self.assertRaises(ValueError):
            typed.attest_record(raw, rows[0], declaration)
        row = copy.deepcopy(rows[0])
        row['_native'] = [dict(kind='typed_record', scope='forged', value=99)]
        result = EVIDENCE['ADAPTERS']['attest_native_nodes'](raw, [row], span=EVIDENCE['span'],
            integer=EVIDENCE['integer'], view=EVIDENCE['view'])
        self.assertEqual(result[row['position']][0]['scope'], 'branch_one')


if __name__ == '__main__':
    unittest.main()
