"""Typed literal fields prove observations without establishing a current node."""
import copy
import pathlib
import runpy
import unittest

import test_preparation_unavailable as unavailable_fixtures
import test_save_preflight as preflight_fixtures


checks = runpy.run_path(str(pathlib.Path(__file__).with_name('evidence.py')))


def fixture():
    text = '(const)%["active"=>%["source"=>"chapter_demo","coordinate"=>73,'
    text += '"caption"=>"Synthetic caption","state"=>"Native title state"],'
    text += '"older"=>%["coordinate"=>0]]'
    return text.encode('utf-16-le'), text


def field(name, value_type='string', record=None):
    return dict(kind='structured_field', document=dict(syntax='tjs_const', encoding='utf-16-le', view=[]),
                record=['active'] if record is None else record, field=name, type=value_type)


def legacy_span(text, value):
    offset = len(text[:text.index(value)].encode('utf-16-le'))
    return dict(offset=offset, length=len(value.encode('utf-16-le')), encoding='utf-16-le')


class StructuredInputEvidenceTests(unittest.TestCase):
    def test_fixed_reader_returns_typed_token_identity_and_public_fresh_records(self):
        raw, _ = fixture()
        coordinate = checks['structured_field'](raw, field('coordinate', 'text_integer'))
        self.assertEqual((coordinate['type'], coordinate['value'], coordinate['token']), ('text_integer', 73, '73'))
        self.assertEqual((coordinate['record'], coordinate['field']), (['active'], 'coordinate'))
        self.assertEqual(checks['span'](raw, coordinate['span']), '73')
        coordinate['record'].clear()
        coordinate['span']['offset'] = 1
        self.assertEqual(checks['structured_field'](raw, field('coordinate', 'text_integer'))['record'], ['active'])

    def test_span_decodes_a_complete_typed_string_token_but_never_an_integer(self):
        raw = r'%["active"=>%["state"=>"Title\nstate"]]'.encode('utf-16-le')
        self.assertEqual(checks['span'](raw, field('state')), 'Title\nstate')
        raw, _ = fixture()
        with self.assertRaisesRegex(ValueError, 'text_integer is not a text observation'):
            checks['span'](raw, field('coordinate', 'text_integer'))
        with self.assertRaisesRegex(ValueError, 'actual quoted string token'):
            checks['span'](raw, field('coordinate', 'string'))

    def test_selected_object_identity_does_not_coerce_a_parent_or_other_record(self):
        raw, _ = fixture()
        self.assertEqual(checks['structured_field'](raw, field('coordinate', 'text_integer'))['value'], 73)
        self.assertEqual(checks['structured_field'](raw, field('coordinate', 'text_integer', ['older']))['value'], 0)
        for proof in (field('coordinate', 'text_integer', []), field('state', record=['older']),
                      field('active.coordinate', 'text_integer', [])):
            with self.subTest(field_type=proof['type']), self.assertRaises(ValueError):
                checks['structured_field'](raw, proof)

    def test_model_offset_and_width_cannot_replace_a_derived_integer_token(self):
        raw, text = fixture()
        digit = len(text[:text.index('73')].encode('utf-16-le'))
        self.assertEqual(checks['integer'](raw, digit + 1, 1), 0)
        for proof in (dict(field('coordinate', 'text_integer'), offset=digit + 1, width=1),
                      dict(field('coordinate', 'text_integer'), field='coordinate[0]')):
            with self.assertRaises(ValueError):
                checks['structured_field'](raw, proof)

    def test_typed_scene_only_diagnostic_does_not_claim_a_current_node(self):
        raw, _ = fixture()
        facts = dict(anchors=[], scene='Native title state', evidence=dict(scene=field('state')))
        result = checks['save_facts'](raw, facts, [])
        self.assertEqual(result['anchors'], [])
        self.assertEqual(result['scene'], 'Native title state')
        self.assertNotIn('line', result)

    def test_any_typed_current_field_blocks_a_mixed_legacy_binary_zero_match(self):
        raw, text = fixture()
        digit = len(text[:text.index('73')].encode('utf-16-le'))
        base = dict(source='chapter_demo.script', line=0, anchors=['Synthetic caption'], evidence=dict(
            source=legacy_span(text, 'chapter_demo'), anchor=legacy_span(text, 'Synthetic caption'),
            position=dict(offset=digit + 1, width=1)))
        row = dict(source=base['source'], position=0, text=base['anchors'][0], speaker=None)
        mutations = [('source', field('source')), ('anchor', field('caption')),
                     ('position', field('coordinate', 'text_integer')),
                     ('position', dict(field('coordinate', 'text_integer'), offset=digit + 1, width=1))]
        for name, proof in mutations:
            facts = copy.deepcopy(base)
            facts['evidence'][name] = proof
            with self.subTest(field=name), self.assertRaisesRegex(ValueError, 'unsupported.*audited state profile') as caught:
                checks['save_facts'](raw, facts, [row])
            self.assertIn('at evidence.' + name + '. Expected ', str(caught.exception))

    def test_typed_scope_or_coordinate_cannot_enter_legacy_call_return(self):
        raw, text = fixture()
        digit = len(text[:text.index('73')].encode('utf-16-le'))
        source = 'chapter_demo.script'
        row = dict(source=source, position=0, text='Synthetic caption', speaker=None, _native=[dict(
            version=1, kind='call_return', scope='chapter_demo', value=0, position=0,
            script_sha256='0' * 64, schema_sha256='1' * 64)])
        base = dict(source=source, line=0, anchors=[row['text']], evidence=dict(
            anchor=legacy_span(text, row['text']), node_link=dict(kind='call_return',
                scope=legacy_span(text, 'chapter_demo'), position=dict(offset=digit + 1, width=1))))
        for name, proof in (('scope', field('source')), ('position', field('coordinate', 'text_integer'))):
            facts = copy.deepcopy(base)
            facts['evidence']['node_link'][name] = proof
            with self.subTest(field=name), self.assertRaisesRegex(ValueError, 'unsupported.*audited state profile') as caught:
                checks['save_facts'](raw, facts, [row])
            self.assertIn('at evidence.node_link.' + name + '. Expected ', str(caught.exception))

    def test_plain_text_and_binary_scalar_primitives_keep_their_existing_contract(self):
        self.assertEqual(checks['span'](b'Text', dict(offset=0, length=4, encoding='ascii')), 'Text')
        self.assertEqual(checks['integer'](b'\x07\0\0\0', 0, 4), 7)
        self.assertEqual(checks['SAVE_VERIFICATION_VERSION'], 3)
        self.assertEqual(checks['SCRIPT_VERIFICATION_VERSION'], 3)


class StructuredPreflightTests(unittest.TestCase):
    def setUp(self):
        self.fixture = preflight_fixtures.SavePreflightTests()
        self.fixture.setUp()
        self.addCleanup(self.fixture.doCleanups)
        self.raw, _ = fixture()
        self.fixture.save.write_bytes(self.raw)
        self.store = self.fixture.new_store(structured_reader=checks['structured_field'])

    def arguments(self):
        return dict(path='current.bin', classification='progress_candidate',
            observations=[dict(role='native_scope', proof=field('source'), value='chapter_demo'),
                          dict(role='native_coordinate', proof=field('coordinate', 'text_integer'), value=73)],
            explanation='Typed native fields are available; current-state semantics still require an audited profile.')

    def test_typed_string_and_integer_are_replayed_without_verifying_a_current_node(self):
        receipt = self.store.record(self.arguments())
        self.assertEqual([item['value'] for item in receipt['observations']], ['chapter_demo', 73])
        self.assertIs(receipt['verified_current_node'], False)
        self.assertTrue(self.store.check_intent('script_sample')['allowed'])
        self.assertIs(self.store.check_intent('script_sample')['verified_current_node'], False)
        self.assertEqual(self.fixture.scalar_calls, 0)

    def test_changed_type_value_or_record_selector_cannot_create_a_receipt(self):
        for mutation in ('value', 'bool', 'record', 'type', 'offset'):
            arguments = self.arguments()
            item = arguments['observations'][1]
            if mutation == 'value': item['value'] = 0
            elif mutation == 'bool': item['value'] = True
            elif mutation == 'record': item['proof']['record'] = ['older']
            elif mutation == 'type': item['proof']['type'] = 'string'
            else: item['proof'].update(offset=1, width=1)
            with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                self.store.record(arguments)
            self.assertEqual(self.store.records, {})

    def test_full_document_and_canonical_token_are_required_before_receipt(self):
        for token in ('-0', '"73"', '1e2', '0x1.0p1', '18446744073709551616'):
            self.fixture.save.write_bytes(self.raw.replace('73'.encode('utf-16-le'), token.encode('utf-16-le')))
            with self.subTest(token_kind=token[:3]), self.assertRaises(ValueError):
                self.store.record(self.arguments())
            self.assertEqual(self.store.records, {})
        self.fixture.save.write_bytes(self.raw + ';call()'.encode('utf-16-le'))
        with self.assertRaises(ValueError):
            self.store.record(self.arguments())

    def test_no_structured_callback_cannot_fall_back_to_binary_scalar(self):
        store = self.fixture.new_store()
        with self.assertRaisesRegex(ValueError, 'trusted complete-document field replay'):
            store.record(self.arguments())
        self.assertEqual(self.fixture.scalar_calls, 0)
        with self.assertRaisesRegex(TypeError, 'trusted callback'):
            self.fixture.new_store(structured_reader={})

    def test_typed_receipts_are_replayed_on_restore_and_invalidate_when_bytes_change(self):
        self.store.record(self.arguments())
        saved = self.store.serialise()
        restored = self.fixture.new_store(structured_reader=checks['structured_field'])
        restored.restore(saved)
        self.assertTrue(restored.check_intent('script_sample')['allowed'])
        self.fixture.save.write_bytes(self.raw.replace('73'.encode('utf-16-le'), b'7\x004\x00'))
        with self.assertRaises(preflight_fixtures.preflight.PreflightRequired):
            restored.check_intent('script_sample')


class StructuredUnavailableTests(unittest.TestCase):
    def setUp(self):
        self.fixture = unavailable_fixtures.PreparationUnavailableTests()
        self.fixture.setUp()
        self.addCleanup(self.fixture.doCleanups)
        self.path = self.fixture.saves / 'current.sav'
        self.path.write_bytes(fixture()[0])
        self.fixture.agent.required = {str(self.path)}

    def test_unavailable_accepts_typed_text_state_only(self):
        result = self.fixture.report([dict(path=self.path.name, proof=field('state'), value='Native title state')],
                                     reason='missing_progress_save')
        self.assertEqual(result['status'], 'unavailable')
        self.assertIsNone(self.fixture.agent.done)

    def test_unavailable_rejects_a_typed_integer_even_as_a_numeric_string(self):
        for value in (73, '73'):
            with self.subTest(value_type=type(value).__name__), self.assertRaisesRegex(ValueError, 'text_integer|TEXT'):
                self.fixture.report([dict(path=self.path.name, proof=field('coordinate', 'text_integer'), value=value)],
                                    reason='missing_progress_save')
            self.assertIsNone(self.fixture.agent.unavailable)


if __name__ == '__main__':
    unittest.main()
