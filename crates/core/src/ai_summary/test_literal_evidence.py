"""Synthetic typed literal proofs; no game paths, user saves or model calls."""
import copy
import pathlib
import runpy
import unittest
from unittest import mock

import literal_evidence as literal


existing = runpy.run_path(str(pathlib.Path(__file__).with_name('evidence.py')))


def proof(field='coordinate', value_type='text_integer', record=None, encoding='utf-16-le'):
    return dict(kind='structured_field', document=dict(syntax='tjs_const', encoding=encoding, view=[]),
                record=['active'] if record is None else record, field=field, type=value_type)


def document(token='73', encoding='utf-16-le'):
    text = '(const)%["active"=>(const)%["source"=>"chapter_demo","coordinate"=>'
    text += token + ',"caption"=>"Synthetic caption"],"other"=>[void,-4,0x1.4000000000000p3/*10*/]]'
    return text.encode(encoding)


class LiteralEvidenceDesignTests(unittest.TestCase):
    def test_complete_document_and_native_integer_field_replay(self):
        raw = document()
        value = literal.replay_field(raw, proof())
        self.assertEqual(value['value'], 73)
        self.assertEqual(value['token'], '73')
        self.assertEqual(existing['span'](raw, value['span']), '73')
        parsed = literal.parse_document(raw, proof()['document'])
        self.assertEqual(parsed['root']['type'], 'object')
        other = parsed['root']['value']['other']['value']
        self.assertEqual([node['type'] for node in other], ['void', 'integer', 'real'])

    def test_coordinate_zero_and_maximum_uint64_remain_exact(self):
        for token in ('0', '9', str(2**64 - 1)):
            with self.subTest(token=token):
                self.assertEqual(literal.read_structured_field(document(token), proof()), int(token))

    def test_selected_coordinate_rejects_noncanonical_or_noninteger_values(self):
        tokens = ('-0', '-1', '00', '+1', '01', '1.0', '1e2', '0x1.0p1',
                  '"73"', 'void', str(2**64), '\uff17\uff13', '\u0667\u0663', '1_0')
        for token in tokens:
            with self.subTest(token_kind=token[:4]), self.assertRaises(ValueError):
                literal.replay_field(document(token), proof())

    def test_whitespace_is_structure_and_is_not_part_of_the_derived_integer_token(self):
        raw = document(' \t73\r\n')
        result = literal.replay_field(raw, proof())
        self.assertEqual(result['token'], '73')
        self.assertEqual(existing['span'](raw, result['span']), '73')

    def test_partial_numeric_token_and_model_supplied_offsets_are_rejected(self):
        for token in ('73suffix', '73 9', '73+1'):
            with self.subTest(token=token), self.assertRaises(ValueError):
                literal.replay_field(document(token), proof())
        for extra in (dict(offset=1), dict(width=1), dict(text=dict(offset=1, length=2, encoding='utf-16-le'))):
            with self.subTest(extra=list(extra)), self.assertRaisesRegex(ValueError, 'byte offsets are not selectors'):
                literal.replay_field(document(), dict(proof(), **extra))

    def test_string_field_replays_quotes_and_confirmed_c_escapes(self):
        text = r'(const)%["active"=>%["caption"=>"It\'s \"quoted\"\\\nnext\rline"]]'
        raw = text.encode('utf-16-le')
        result = literal.replay_field(raw, proof('caption', 'string'))
        self.assertEqual(result['value'], 'It\'s "quoted"\\\nnext\rline')
        token = existing['span'](raw, result['span'])
        self.assertTrue(token.startswith('"') and token.endswith('"'))
        self.assertNotEqual(token[1:-1], result['value'])

    def test_record_selector_is_typed_and_field_is_an_immediate_member(self):
        raw = document()
        for field in ('active.coordinate', 'missing'):
            with self.subTest(field=field), self.assertRaises(ValueError):
                literal.replay_field(raw, proof(field))
        for record in ([], ['other'], ['other', 0], ['active', 'coordinate'], [True], ['active', '..']):
            with self.subTest(record_type=type(record[-1]).__name__ if record else 'root'), self.assertRaises(ValueError):
                literal.replay_field(raw, proof(record=record))

    def test_array_index_can_select_an_object_record_but_cannot_be_boolean(self):
        raw = b'%["records"=>[%["coordinate"=>7],%["coordinate"=>19]]]'
        field = proof(record=['records', 1], encoding='ascii')
        self.assertEqual(literal.read_structured_field(raw, field), 19)
        field['record'][-1] = True
        with self.assertRaises(ValueError):
            literal.read_structured_field(raw, field)

    def test_duplicate_keys_unknown_grammar_trailing_code_and_unterminated_comments_fail_closed(self):
        inputs = [b'%["coordinate"=>7,"coordinate"=>9]', b'%["x"=>1]();',
                  b'%["x"=>call()]', b'%["x"=>1,]', b'[1,]', b'%["x"=>1]/*',
                  b'%["x"=>NaN]', b'%["x"=>"\\u0030"]', b"%['x'=>1]", b'(const)7']
        for raw in inputs:
            with self.subTest(size=len(raw)), self.assertRaises(ValueError):
                literal.parse_document(raw, dict(syntax='tjs_const', encoding='ascii', view=[]))
        for change in (dict(syntax='custom'), dict(grammar='.*'), dict(offset=4)):
            with self.subTest(fields=list(change)), self.assertRaises(ValueError):
                literal.parse_document(document(), dict(proof()['document'], **change))

    def test_utf16_non_bmp_prefix_does_not_shift_native_token_spans(self):
        text = '%["prefix"=>"\U0001f642","active"=>%["coordinate"=>73,"caption"=>"\U0001f680"]]'
        for encoding in ('utf-8', 'utf-16-le', 'utf-16-be'):
            with self.subTest(encoding=encoding):
                raw = text.encode(encoding)
                result = literal.replay_field(raw, proof(encoding=encoding))
                expected = len(text[:text.index('73')].encode(encoding))
                self.assertEqual(result['span']['offset'], expected)
                self.assertEqual(existing['span'](raw, result['span']), '73')
                caption = literal.replay_field(raw, proof('caption', 'string', encoding=encoding))
                self.assertEqual(caption['value'], '\U0001f680')

    def test_supported_encodings_bom_and_strict_invalid_utf16(self):
        for encoding in literal.ENCODINGS:
            with self.subTest(encoding=encoding):
                raw = document(encoding=encoding)
                self.assertEqual(literal.read_structured_field(raw, proof(encoding=encoding)), 73)
                if encoding != 'ascii':
                    bom = '\ufeff'.encode(encoding)
                    self.assertEqual(literal.read_structured_field(bom + raw, proof(encoding=encoding)), 73)
        for raw in (b'\x00', b'\x00\xd8'):
            with self.assertRaises(UnicodeError):
                literal.parse_document(raw, proof()['document'])

    def test_standard_view_is_replayed_before_complete_document_parse(self):
        payload = document()
        raw = b'PREFIX' + payload + b'SUFFIX'
        field = proof()
        field['document']['view'] = [dict(op='slice', offset=6, length=len(payload))]
        result = literal.replay_field(raw, field)
        self.assertEqual(result['value'], 73)
        self.assertEqual(existing['span'](raw, result['span']), '73')
        with self.assertRaises(TypeError):
            literal.replay_field(raw, field, view=lambda data, steps: document('0'))

    def test_windows_hex_escapes_use_one_through_four_utf16_hex_digits(self):
        cases = [(r'"\x41"', 'A'), (r'"\X0041B"', 'AB'),
                 (r'"\x41B"', '\u041b'), (r'"\xD83D\xDE80"', '\U0001f680')]
        for token, expected in cases:
            raw = ('%["active"=>%["caption"=>' + token + ']]').encode('ascii')
            with self.subTest(token_bytes=len(token)):
                result = literal.replay_field(raw, proof('caption', 'string', encoding='ascii'))
                self.assertEqual(result['value'], expected)
                self.assertEqual(existing['span'](raw, result['span']), token)
        for token in (r'"\x"', r'"\u0041"', r'"\xD800"', r'"\xDC00"', r'"\xD800\x0041"'):
            with self.subTest(token_bytes=len(token)), self.assertRaises(ValueError):
                literal.parse_document(token.encode('ascii'), dict(syntax='tjs_const', encoding='ascii'))

    def test_adjacent_quoted_tokens_replay_as_one_string_with_full_native_span(self):
        token = r'"\x22" "A\nB"'
        raw = ('%["active"=>%["caption"=>' + token + ']]').encode('ascii')
        result = literal.replay_field(raw, proof('caption', 'string', encoding='ascii'))
        self.assertEqual(result['value'], '"A\nB')
        self.assertEqual(existing['span'](raw, result['span']), token)

    def test_decoded_hex_keys_cannot_hide_duplicates(self):
        with self.assertRaisesRegex(ValueError, 'duplicate decoded object key'):
            literal.parse_document(br'%["a"=>1,"\x0061"=>2]', dict(syntax='tjs_const', encoding='ascii'))

    def test_serializer_null_and_nested_block_comments_are_structural_data(self):
        raw = b'(const) [null /* object /* nested */ comment */, void]'
        parsed = literal.parse_document(raw, dict(syntax='tjs_const', encoding='ascii'))
        self.assertEqual([node['type'] for node in parsed['root']['value']], ['null', 'void'])
        for suffix in (b'/* nested /* unclosed */', b'/*' * 66 + b'*/' * 66):
            with self.subTest(bytes=len(suffix)), self.assertRaises(ValueError):
                literal.parse_document(b'void' + suffix, dict(syntax='tjs_const', encoding='ascii'))

    def test_no_public_api_accepts_a_user_decoder_callable(self):
        for function, argument in ((literal.parse_document, proof()['document']),
                                   (literal.replay_field, proof()), (literal.read_structured_field, proof())):
            with self.subTest(api=function.__name__), self.assertRaises(TypeError):
                function(document(), argument, view=lambda *args: document('0'))

    def test_byte_depth_node_string_and_number_budgets_fail_closed(self):
        schema = dict(syntax='tjs_const', encoding='ascii', view=[])
        with mock.patch.object(literal, 'MAX_BYTES', 5), self.assertRaises(ValueError):
            literal.parse_document(b'%["x"=>1]', schema)
        cases = [(b'[' * 66 + b'0' + b']' * 66, 'depth'),
                 (b'[' + b'9' * 129 + b']', 'numeric token')]
        for raw, keyword in cases:
            with self.subTest(budget=keyword), self.assertRaisesRegex(ValueError, keyword):
                literal.parse_document(raw, schema)
        with mock.patch.object(literal, 'MAX_NODES', 2), self.assertRaisesRegex(ValueError, 'node budget'):
            literal.parse_document(b'[1,2,3]', schema)
        with mock.patch.object(literal, 'MAX_STRING_CHARACTERS', 2), self.assertRaisesRegex(ValueError, 'string character'):
            literal.parse_document(b'"long-string"', schema)

    def test_public_results_cannot_mutate_cached_evidence_or_caller_schema(self):
        raw, field = document(), proof()
        original = copy.deepcopy(field)
        first = literal.parse_document(raw, field['document'])
        first['root']['value']['active']['value']['coordinate']['value'] = 0
        first['document']['encoding'] = 'ascii'
        replay = literal.replay_field(raw, field)
        replay['span']['offset'] = 1
        replay['document']['view'].append(dict(op='slice', offset=1, length=1))
        self.assertEqual(literal.read_structured_field(raw, field), 73)
        self.assertEqual(literal.parse_document(raw, field['document'])['root']['value']['active']['value']['coordinate']['value'], 73)
        self.assertEqual(field, original)


class ExistingBinaryBoundaryTests(unittest.TestCase):
    def test_existing_binary_zero_proof_cannot_establish_a_text_coordinate(self):
        text = '%["source"=>"chapter_demo","coordinate"=>73,"caption"=>"Synthetic caption"]'
        raw = text.encode('utf-16-le')
        offset = len(text[:text.index('73')].encode('utf-16-le'))
        self.assertEqual(existing['span'](raw, dict(offset=offset, length=4, encoding='utf-16-le')), '73')
        self.assertNotEqual(existing['integer'](raw, offset, 4), 73)
        self.assertEqual(existing['integer'](raw, offset + 1, 1), 0)
        # This deliberately demonstrates the existing generic binary path's
        # missing field/type association, not a proposed acceptance rule.
        def span(value):
            begin = text.index(value)
            return dict(offset=len(text[:begin].encode('utf-16-le')),
                        length=len(value.encode('utf-16-le')), encoding='utf-16-le')
        facts = dict(source='chapter_demo.script', line=0, anchors=['Synthetic caption'], evidence=dict(
            source=span('chapter_demo'), anchor=span('Synthetic caption'), position=dict(offset=offset + 1, width=1)))
        rows = [dict(source='chapter_demo.script', position=0, text='Synthetic caption', speaker=None)]
        self.assertEqual(existing['save_facts'](raw, facts, rows)['line'], 0)
        field = proof(record=[])
        self.assertEqual(literal.read_structured_field(raw, field), 73)
        with self.assertRaises(ValueError):
            literal.read_structured_field(raw, dict(field, offset=offset + 1, width=1))


if __name__ == '__main__':
    unittest.main()
