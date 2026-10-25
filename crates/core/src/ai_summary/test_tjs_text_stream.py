"""Public text-stream framing only: synthetic inputs, no games or model calls."""
import copy
import pathlib
import runpy
import tempfile
import unittest
import zlib


BASE = pathlib.Path(__file__).parent
CODECS = runpy.run_path(str(BASE / 'bytecodecs.py'))
LITERAL = runpy.run_path(str(BASE / 'literal_evidence.py'))
TYPED = runpy.run_path(str(BASE / 'typed_evidence.py'))
EVIDENCE = runpy.run_path(str(BASE / 'evidence.py'))
KAG = runpy.run_path(str(BASE / 'kag_evidence.py'))
HOST = runpy.run_path(str(BASE / 'agent_host.py'))
STREAM = [dict(op='tjs_text_stream')]


def wrap(raw, mode):
    """Synthetic framing, not an engine-native producer or a mode0 encoder."""
    if mode is None:
        return b'\xff\xfe' + raw
    if mode == 2:
        packed = zlib.compress(raw)
        return b'\xfe\xfe\x02\xff\xfe' + len(packed).to_bytes(8, 'little') + len(raw).to_bytes(8, 'little') + packed
    if mode == 1:
        # Independently swap pairs in textual bit representations.
        body = bytes(int(''.join(bits[i + 1] + bits[i] for i in range(0, 8, 2)), 2)
                     for bits in (format(value, '08b') for value in raw))
    else:
        raise ValueError('mode0 tests supply encoded units directly')
    return b'\xfe\xfe\x01\xff\xfe' + body


def literal_proof(view=STREAM):
    return dict(kind='structured_field', document=dict(syntax='tjs_const', encoding='utf-16-le', view=copy.deepcopy(view)),
                record=['active'], field='coordinate', type='text_integer')


class TjsTextStreamTests(unittest.TestCase):
    def test_plaintext_and_modes1_2_return_exact_utf16_body(self):
        for text in ('', 'Synthetic Latin\r\n', '合成・テスト 😀\n\t"\\', '\0\x01\x1f'):
            raw = text.encode('utf-16-le')
            for mode in (None, 1, 2):
                with self.subTest(mode=mode, size=len(raw)):
                    data = wrap(raw, mode)
                    self.assertEqual(CODECS['decode']('tjs_text_stream', data, len(raw)), raw)
                    self.assertEqual(CODECS['view'](data, STREAM), raw)

    def test_mode0_uses_reader_condition_and_preserves_encoded_controls(self):
        # Encoded 0x0203 becomes 0x0002, whereas encoded 0x0002 stays 0x0002.
        # Consequently a generic encode/decode roundtrip would be a false oracle.
        encoded = b''.join(unit.to_bytes(2, 'little') for unit in (*range(32), 0x0203, 0x4040, 0x1fff))
        expected = b''.join(unit.to_bytes(2, 'little') for unit in (*range(32), 2, 0x41, 0xe1fe))
        self.assertEqual(CODECS['decode']('tjs_text_stream', b'\xfe\xfe\0\xff\xfe' + encoded), expected)
        self.assertNotEqual(CODECS['decode']('tjs_text_stream', b'\xfe\xfe\0\xff\xfe' + expected), encoded)

    def test_mode0_static_synthetic_literal_replays_without_an_encoder(self):
        # Constructed preimages, independently checked by the compiled public
        # uint16 reader and bundled GARbro NoCrypt; not an engine-produced save.
        data = bytes.fromhex('fefe00fffe24245a5a232262626e6e6e6e7372656468686f6e6060757464642322'
                             '3c3c3f3e363632322d2c2322626260607170757468686e6e6f6e23223c3c3f3e'
                             '2322525278786f6e75746968646475746868626223225c5c')
        self.assertEqual(CODECS['decode']('tjs_text_stream', data).decode('utf-16-le'),
                         '%["coordinate"=>73,"caption"=>"Synthetic"]')
        self.assertEqual(LITERAL['read_structured_field'](data, dict(literal_proof(), record=[])), 73)

    def test_bytecodec_preserves_units_and_leaves_text_validity_to_strict_parser(self):
        raw = b'\x00\xd8'  # Unpaired surrogate is still one complete wire unit.
        self.assertEqual(CODECS['decode']('tjs_text_stream', wrap(raw, 2)), raw)
        with self.assertRaises(ValueError):
            LITERAL['parse_document'](wrap(raw, 2), literal_proof()['document'])

    def test_headers_original_bom_and_incomplete_units_fail_closed(self):
        bad = (b'', b'\xff', b'\xfe\xff', b'plain text', b'\xef\xbb\xbftext',
               b'\xfe\xfe', b'\xfe\xfe\0', b'\xfe\xfe\x03\xff\xfe',
               b'\xfe\xfe\0\0\0', b'\xfe\xfe\x01\xfe\xff')
        for data in (*bad, wrap(b'A', None), b'\xfe\xfe\0\xff\xfeA', b'\xfe\xfe\x01\xff\xfeA'):
            with self.subTest(header=data[:5].hex()), self.assertRaises(ValueError):
                CODECS['decode']('tjs_text_stream', data)
        complete = wrap(b'A\0', 2)
        for length in range(5, 21):
            with self.subTest(truncated_header=length), self.assertRaises(ValueError):
                CODECS['decode']('tjs_text_stream', complete[:length])

    def test_mode2_native_sizes_zlib_eof_and_checksum_are_independent_requirements(self):
        raw = ('Synthetic frame. ' * 8).encode('utf-16-le')
        valid = wrap(raw, 2)
        for offset, value in ((5, len(valid)-22), (5, len(valid)-20), (13, len(raw)-2),
                              (13, len(raw)+2), (13, len(raw)-1), (5, 2**64-1), (13, 2**64-1)):
            data = valid[:offset] + value.to_bytes(8, 'little') + valid[offset+8:]
            with self.subTest(offset=offset, value=value), self.assertRaises(ValueError):
                CODECS['decode']('tjs_text_stream', data)
        packed = valid[21:]
        broken_streams = (packed[:-1], packed+b'junk', packed+zlib.compress(b'Other\0'),
                          packed[:-1]+bytes([packed[-1]^1]), packed[2:-4])
        for packed in broken_streams:
            data = valid[:5] + len(packed).to_bytes(8, 'little') + len(raw).to_bytes(8, 'little') + packed
            with self.subTest(packed_size=len(packed)), self.assertRaises(ValueError):
                CODECS['decode']('tjs_text_stream', data)
        with self.assertRaises(ValueError):
            CODECS['decode']('tjs_text_stream', valid+b'\0')

    def test_expected_size_and_global_input_output_budgets_are_enforced(self):
        data = wrap(b'A\0', 2)
        for size in (True, -1, '2', 1, 4, CODECS['MAX_BYTES']+1):
            with self.subTest(size=size), self.assertRaises(ValueError):
                CODECS['decode']('tjs_text_stream', data, size)
        too_large = CODECS['MAX_BYTES'] + 2
        data = b'\xfe\xfe\x02\xff\xfe' + (8).to_bytes(8, 'little') + too_large.to_bytes(8, 'little') + zlib.compress(b'')
        with self.assertRaises(ValueError):
            CODECS['decode']('tjs_text_stream', data)
        with self.assertRaises(ValueError):
            CODECS['decode']('tjs_text_stream', b'\xff\xfe' + b'\0' * CODECS['MAX_BYTES'])

    def test_no_model_keys_paths_modes_offsets_or_unknown_fields(self):
        data = wrap(b'A\0', 1)
        for field, value in (('key', '00'), ('path', 'external'), ('mode', 1), ('offset', 2),
                             ('length', len(data)), ('size_from', dict(offset=5)), ('anything', None)):
            with self.subTest(field=field), self.assertRaises(ValueError):
                CODECS['view'](data, [dict(op='tjs_text_stream', **{field:value})])
        with self.assertRaisesRegex(ValueError, 'no key parameter'):
            CODECS['decode']('tjs_text_stream', data, key='00')

    def test_bmp_requires_explicit_bounded_slice_and_never_autodetects(self):
        stream = wrap(b'%["active"=>%["coordinate"=>73]]'.decode().encode('utf-16-le'), 2)
        thumbnail = b'BM' + (54).to_bytes(4, 'little') + b'\0' * 48
        data = thumbnail + stream
        with self.assertRaises(ValueError):
            CODECS['view'](data, STREAM)
        view = [dict(op='slice', offset=len(thumbnail), length=len(stream)), *STREAM]
        self.assertEqual(LITERAL['read_structured_field'](data, literal_proof(view)), 73)
        with self.assertRaises(ValueError):
            CODECS['view'](data, [dict(op='slice', offset=len(thumbnail)+1, length=len(stream)-1), *STREAM])

    def test_literal_typed_and_span_verifiers_replay_the_same_complete_bytes(self):
        raw = '%["active"=>%["coordinate"=>73,"caption"=>"合成 caption"],"suffix"=>9]'.encode('utf-16-le')
        for mode in (None, 1, 2):
            data = wrap(raw, mode)
            with self.subTest(mode=mode):
                selected = LITERAL['replay_field'](data, literal_proof())
                self.assertEqual(selected['value'], 73)
                self.assertEqual(EVIDENCE['span'](data, selected['span']), '73')
                field = dict(kind='typed_field', document=literal_proof()['document'],
                             path=['active', 'caption'], type='string')
                self.assertEqual(TYPED['replay_field'](data, field)['value'], '合成 caption')
                self.assertEqual(EVIDENCE['span'](data, field), '合成 caption')
                with self.assertRaises(ValueError):
                    LITERAL['parse_document'](wrap(raw+b'X\0', mode), literal_proof()['document'])

    def test_kag_label_index_replays_view_but_codec_never_attests_a_role(self):
        raw = ';Synthetic\r\n*entry|Caption\nReal text after the marker.'.encode('utf-16-le')
        for mode in (None, 1, 2):
            data = wrap(raw, mode)
            labels = KAG['label_index'](data, dict(encoding='utf-16-le', view=STREAM))['labels']
            self.assertEqual([label['alias'] for label in labels], ['*entry'])
            self.assertEqual(EVIDENCE['span'](data, labels[0]['evidence']), '*entry|Caption')
            # Decoding alone emits no signed node. A model receipt is discarded.
            facts = EVIDENCE['save_facts'](data, dict(anchors=[], resume=dict(state_role='current')), [])
            self.assertNotIn('resume', facts)

    def test_parser_child_sandbox_exposes_codec_and_denies_external_access(self):
        data = wrap('Synthetic parser text.'.encode('utf-16-le'), 2)
        program = ('import galgear_codecs\ndef extract(data, metadata):\n'
                   '    return {"text":galgear_codecs.decode("tjs_text_stream",data).decode("utf-16-le"),'
                   '"view":galgear_codecs.view(data,[{"op":"tjs_text_stream"}]).hex(),'
                   '"caps":galgear_codecs.capabilities()}\n')
        result = HOST['run_parser'](program, data, {})
        self.assertEqual(result['text'], 'Synthetic parser text.')
        self.assertIn('tjs_text_stream', result['caps']['codecs'])
        for forbidden in ('import pathlib', 'import socket', 'import subprocess', 'open("external", "rb")'):
            with self.subTest(forbidden=forbidden), self.assertRaises((ValueError, RuntimeError)):
                HOST['run_parser'](forbidden+'\ndef extract(data, metadata):\n    return {}\n', data, {})
        with self.assertRaisesRegex(RuntimeError, 'no keys, paths or layout'):
            HOST['run_parser']('import galgear_codecs\ndef extract(data, metadata):\n'
                               '    return galgear_codecs.view(data,[{"op":"tjs_text_stream","key":"00"}])\n', data, {})

    def test_contract_export_and_full_contract_include_fixed_replay_capability(self):
        with tempfile.TemporaryDirectory(prefix='synthetic-textstream-') as folder:
            game = pathlib.Path(folder) / 'synthetic-game'
            game.mkdir()
            (game / 'fixture.exe').write_bytes(b'synthetic')
            skill = pathlib.Path(folder) / 'empty-skill'
            skill.mkdir()
            agent = HOST['Agent'](dict(game=str(game), save=str(game), skill=str(skill), work=str(game/'work'),
                                       provider='synthetic', model='none', title='Synthetic', language='en', max_turns=0))
            for topic in ('script', 'save', 'adapter'):
                contract = agent.dispatch('verification_contract', dict(topic=topic))
                self.assertEqual(contract['byte_codecs']['version'], 3)
                self.assertIn('tjs_text_stream', contract['byte_codecs']['codecs'])
                self.assertIn('no external mode/key/path', contract['byte_codecs']['tjs_text_stream']['framing'])
            exported = agent.dispatch('verification_contract', dict(topic='all'))
            # The full export is host-written only under this synthetic work root.
            import json
            saved = json.loads((game/'work'/exported['full_contract']).read_text(encoding='utf-8'))
            self.assertIn('tjs_text_stream', saved['byte_codecs']['transforms'])
            self.assertEqual(agent.requests_sent, 0)


if __name__ == '__main__':
    unittest.main()
