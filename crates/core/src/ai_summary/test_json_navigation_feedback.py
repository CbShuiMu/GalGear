"""Actual read_json navigation on synthetic JSON; no model/parser/game access."""
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest


SPEC = importlib.util.spec_from_file_location('json_feedback_host', Path(__file__).with_name('agent_host.py'))
host = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(host)


class JsonNavigationFeedbackTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        # Exercise the actual dispatch branch without unrelated preparation,
        # session/cache restore or opening any production game directory.
        self.agent = object.__new__(host.Agent)
        self.agent.path = lambda root, name: self.root / name

    def read(self, value, **args):
        (self.root / 'synthetic.json').write_text(json.dumps(value, ensure_ascii=False), encoding='utf-8')
        return self.agent._dispatch('read_json', dict(root='work', path='synthetic.json', **args))

    def navigation_error(self, value, category, **args):
        with self.assertRaises(ValueError) as raised:
            self.read(value, **args)
        prefix = 'JSON navigation error: '
        message = str(raised.exception)
        self.assertTrue(message.startswith(prefix), message)
        detail = json.loads(message[len(prefix):])
        self.assertEqual(detail['category'], category)
        self.assertIn('pointer=""', detail['root_pointer_hint'])
        self.assertLessEqual(len(message[len(prefix):].encode('utf-8')), host.JSON_NAVIGATION_FEEDBACK_BYTES)
        return detail

    def test_root_is_empty_or_omitted_and_wrapper_is_not_unwrapped(self):
        value = dict(tool_call_id='synthetic-call', function=dict(name='synthetic'), result={'alpha': 1})
        self.assertEqual(self.read(value), dict(value=value))
        self.assertEqual(self.read(value, pointer=''), dict(value=value))
        self.assertEqual(self.read(value, pointer='/result'), dict(value={'alpha': 1}))

    def test_slash_reads_a_real_empty_key_instead_of_becoming_root(self):
        value = {'': {'': 'empty key payload'}, 'other': 'root payload'}
        self.assertEqual(self.read(value, pointer='/'), dict(value={'': 'empty key payload'}))
        self.assertEqual(self.read(value, pointer='//'), dict(value='empty key payload'))

    def test_slash_missing_empty_key_explains_actual_failure_and_root(self):
        detail = self.navigation_error({'alpha': 'SECRET_PAYLOAD'}, 'missing_key', pointer='/')
        self.assertEqual(detail['failed_token'], '')
        self.assertEqual(detail['parent_pointer'], '')
        self.assertEqual(detail['parent_type'], 'object')
        self.assertEqual(detail['available_keys'], ['alpha'])
        self.assertNotIn('SECRET_PAYLOAD', json.dumps(detail))

    def test_escaped_keys_use_standard_decoding_once(self):
        value = {'a/b': {'m~n': {'~1': 'escaped payload'}}}
        self.assertEqual(self.read(value, pointer='/a~1b/m~0n/~01'), dict(value='escaped payload'))
        detail = self.navigation_error(value, 'missing_key', pointer='/a~1b/m~0n/absent')
        self.assertEqual(detail['failed_token'], 'absent')
        self.assertEqual(detail['parent_pointer'], '/a~1b/m~0n')
        self.assertEqual(detail['available_keys'], ['~1'])

    def test_bad_pointer_type_syntax_or_escape_is_explicit_without_values(self):
        for pointer in (None, 0, False, ['SECRET_PAYLOAD'], {'hidden': 'SECRET_PAYLOAD'}):
            with self.subTest(type=type(pointer).__name__):
                detail = self.navigation_error({}, 'pointer_type', pointer=pointer)
                self.assertNotIn('SECRET_PAYLOAD', json.dumps(detail))
        self.navigation_error({}, 'pointer_syntax', pointer='alpha')
        for pointer in ('/alpha~', '/alpha~2'):
            with self.subTest(pointer=pointer):
                self.navigation_error({}, 'pointer_escape', pointer=pointer)

    def test_array_indices_report_parent_and_length(self):
        value = {'rows': [{'alpha': 1}, {'alpha': 2}]}
        self.assertEqual(self.read(value, pointer='/rows/1/alpha'), dict(value=2))
        detail = self.navigation_error(value, 'array_index_out_of_range', pointer='/rows/2')
        self.assertEqual(detail['failed_token'], '2')
        self.assertEqual(detail['parent_pointer'], '/rows')
        self.assertEqual(detail['parent_type'], 'array')
        self.assertEqual(detail['array_length'], 2)
        self.navigation_error([], 'array_index_out_of_range', pointer='/0')

    def test_invalid_array_tokens_are_not_coerced_to_another_index(self):
        for token in ('-1', '01', '+1', '1.0', ' ', '-', 'alpha', '\u0661'):
            with self.subTest(token=token):
                detail = self.navigation_error([1, 2], 'array_index_syntax', pointer='/' + token)
                self.assertEqual(detail['failed_token'], token)
                self.assertEqual(detail['array_length'], 2)
        self.assertEqual(self.read({'01': 'object key'}, pointer='/01'), dict(value='object key'))

    def test_very_large_array_index_has_bounded_actionable_error(self):
        detail = self.navigation_error([0], 'array_index_out_of_range', pointer='/' + '9' * 10000)
        self.assertTrue(detail['failed_token_truncated'])
        self.assertEqual(detail['array_length'], 1)

    def test_traversal_through_each_scalar_never_exposes_its_value(self):
        for value, expected in (('SECRET_PAYLOAD', 'string'), (123456789, 'number'),
                                (True, 'boolean'), (None, 'null')):
            with self.subTest(type=expected):
                detail = self.navigation_error({'scalar': value}, 'scalar_traversal', pointer='/scalar/child')
                self.assertEqual(detail['parent_pointer'], '/scalar')
                self.assertEqual(detail['parent_type'], expected)
                self.assertNotIn('SECRET_PAYLOAD', json.dumps(detail))
                self.assertNotIn('123456789', json.dumps(detail))

    def test_exchange_record_hint_uses_actual_keys_and_does_not_guess_unwrap(self):
        record = dict(tool_call_id='synthetic', function={'arguments': 'SECRET_ARGUMENTS'}, result={'alpha': 1})
        detail = self.navigation_error(record, 'missing_key', pointer='/alpha')
        self.assertEqual(detail['available_keys'], ['tool_call_id', 'function', 'result'])
        self.assertEqual(detail['exchange_result_pointer'], '/result')
        self.assertNotIn('SECRET_ARGUMENTS', json.dumps(detail))
        nested = self.navigation_error({'records': [record]}, 'missing_key', pointer='/records/0/alpha')
        self.assertEqual(nested['exchange_result_pointer'], '/records/0/result')
        ordinary = self.navigation_error({'result': {}}, 'missing_key', pointer='/alpha')
        self.assertNotIn('exchange_result_pointer', ordinary)

    def test_partial_missing_fields_preserves_existing_values_and_reports_absence(self):
        value = {'alpha': {'kept': 1}, 'other': 'UNSELECTED_SECRET'}
        result = self.read(value, fields=['alpha', 'absent'])
        self.assertEqual(result['value'], {'alpha': {'kept': 1}})
        feedback = result['field_feedback']
        self.assertEqual(feedback['diagnostic_count'], 1)
        detail = feedback['diagnostics'][0]
        self.assertEqual(detail['missing_fields'], ['absent'])
        self.assertEqual(detail['available_keys'], ['alpha', 'other'])
        self.assertEqual(detail['parent_pointer'], '')
        self.assertNotIn('UNSELECTED_SECRET', json.dumps(result))
        self.assertNotIn('kept', json.dumps(feedback))

    def test_all_missing_fields_is_not_a_silent_empty_object(self):
        result = self.read({'alpha': 'SECRET_PAYLOAD'}, fields=['absent', 'other'])
        self.assertEqual(result['value'], {})
        self.assertEqual(result['field_feedback']['diagnostics'][0]['missing_fields'], ['absent', 'other'])
        self.assertEqual(result['field_feedback']['diagnostics'][0]['available_keys'], ['alpha'])
        self.assertNotIn('SECRET_PAYLOAD', json.dumps(result))

    def test_valid_or_explicit_empty_field_selection_keeps_existing_shape(self):
        self.assertEqual(self.read({'alpha': 1}, fields=['alpha']), dict(value={'alpha': 1}))
        self.assertEqual(self.read({'alpha': 1}, fields=[]), dict(value={}))
        self.assertEqual(self.read([{'alpha': 1}], fields=['alpha']),
            dict(items=[{'alpha': 1}], total_entries=1, offset=0, next_offset=None))

    def test_list_pagination_reports_absolute_entry_indices_and_nested_pointer(self):
        value = {'rows': [{'alpha': 0}, {'alpha': 1}, {'alpha': 2}, {'other': 'SECRET_PAYLOAD'}, 42]}
        result = self.read(value, pointer='/rows', offset=2, limit=3, fields=['alpha'])
        self.assertEqual(result['items'], [{'alpha': 2}, {}, 42])
        self.assertEqual(result['offset'], 2)
        self.assertEqual(result['next_offset'], None)
        diagnostics = result['field_feedback']['diagnostics']
        self.assertEqual([row['entry_index'] for row in diagnostics], [3, 4])
        self.assertEqual([row['parent_pointer'] for row in diagnostics], ['/rows/3', '/rows/4'])
        self.assertEqual(diagnostics[0]['missing_fields'], ['alpha'])
        self.assertEqual(diagnostics[1]['category'], 'fields_require_object')
        self.assertEqual(diagnostics[1]['parent_type'], 'number')
        self.assertNotIn('SECRET_PAYLOAD', json.dumps(result))

    def test_fields_on_scalar_returns_explicit_type_diagnostic(self):
        result = self.read('scalar payload', fields=['alpha'])
        self.assertEqual(result['value'], 'scalar payload')
        detail = result['field_feedback']['diagnostics'][0]
        self.assertEqual(detail['category'], 'fields_require_object')
        self.assertEqual(detail['parent_type'], 'string')
        self.assertNotIn('scalar payload', json.dumps(detail))

    def test_invalid_fields_and_pagination_types_report_metadata_only(self):
        for fields in ('alpha', {'alpha': 'SECRET_PAYLOAD'}, [1], list(map(str, range(21)))):
            with self.subTest(fields_type=type(fields).__name__):
                detail = self.navigation_error({}, 'fields_type', fields=fields)
                self.assertNotIn('SECRET_PAYLOAD', json.dumps(detail))
        for key in ('offset', 'limit'):
            for value in (True, None, 'SECRET_PAYLOAD', 1.5):
                with self.subTest(key=key, type=type(value).__name__):
                    detail = self.navigation_error([], 'pagination_type', **{key: value})
                    self.assertEqual(detail['argument'], key)
                    self.assertNotIn('SECRET_PAYLOAD', json.dumps(detail))

    def test_large_keys_and_parent_paths_have_bounded_metadata_without_values(self):
        value = {'prefix': {'\U0001f600' * 3000 + str(index): 'SECRET_PAYLOAD' * 100 for index in range(200)}}
        detail = self.navigation_error(value, 'missing_key', pointer='/prefix/' + '\U0001f600' * 3000)
        self.assertEqual(detail['total_keys'], 200)
        self.assertLessEqual(len(detail['available_keys']), 6)
        self.assertGreater(detail['available_keys_omitted'], 0)
        self.assertNotIn('SECRET_PAYLOAD', json.dumps(detail))
        long = '\U0001f600' * 3000
        detail = self.navigation_error({long: {}}, 'missing_key', pointer='/' + long + '/absent')
        self.assertTrue(detail['parent_pointer_truncated'])

    def test_field_diagnostics_have_total_budget_and_explicit_omitted_indices(self):
        long = '\U0001f600' * 3000
        rows = [{long + str(index): 'SECRET_PAYLOAD'} for index in range(24)]
        fields = [long + 'missing-' + str(index) for index in range(20)]
        result = self.read(rows, fields=fields, offset=2, limit=20)
        self.assertEqual(result['items'], [{}] * 20)
        feedback = result['field_feedback']
        self.assertEqual(feedback['diagnostic_count'], 20)
        self.assertLessEqual(len(json.dumps(feedback, ensure_ascii=False, separators=(',', ':')).encode()),
                             host.JSON_FIELD_FEEDBACK_BYTES)
        observed = [row['entry_index'] for row in feedback['diagnostics']]
        omitted = feedback.get('omitted_entry_indices', [])
        self.assertEqual(sorted(observed + omitted), list(range(2, 22)))
        self.assertEqual(feedback['omitted_diagnostics'], len(omitted))
        self.assertNotIn('SECRET_PAYLOAD', json.dumps(feedback))

    def test_tool_description_distinguishes_root_wrapper_and_archive_path(self):
        description = next(row['function']['description'] for row in host.TOOLS
                           if row['function']['name'] == 'read_json')
        for fragment in ('pointer=""', '"/" selects an empty-string key', '/result', 'full_output',
                         'filename, not its JSON body', 'No wrapper or pointer is automatically changed'):
            self.assertIn(fragment, description)


if __name__ == '__main__':
    unittest.main()
