"""The parser sandbox can read typed values without altering shared readers."""
import pathlib
import runpy
import unittest


host = runpy.run_path(str(pathlib.Path(__file__).with_name('agent_host.py')))
DOCUMENT = dict(syntax='tjs_const', encoding='ascii')
PROOF = dict(kind='structured_field', document=DOCUMENT, record=['box'], field='counter', type='text_integer')
DATA = b'(const) %["box" => (const) %["counter" => 0, "caption" => "Line\\nTwo"]]'


class LiteralHostTests(unittest.TestCase):
    def test_same_fixed_reader_is_available_to_generated_adapters(self):
        program = ('import galgear_literals\n'
                   'def extract(data, metadata):\n'
                   '    return galgear_literals.replay_field(data, ' + repr(PROOF) + ')\n')
        output = host['run_parser'](program, DATA, {})
        self.assertEqual(output['value'], 0)
        self.assertEqual(output['token'], '0')
        self.assertEqual(output['span']['length'], 1)

    def test_public_tree_mutation_cannot_poison_subsequent_reads(self):
        program = ('import galgear_literals\n'
                   'def extract(data, metadata):\n'
                   '    first = galgear_literals.parse_document(data, ' + repr(DOCUMENT) + ')\n'
                   '    first["root"]["value"]["box"]["value"]["counter"]["value"] = 999\n'
                   '    return {"counter":galgear_literals.read_structured_field(data, ' + repr(PROOF) + ')}\n')
        self.assertEqual(host['run_parser'](program, DATA, {}), dict(counter=0))

    def test_module_attributes_and_decoder_callbacks_cannot_be_replaced(self):
        mutation = 'import galgear_literals\ngalgear_literals.read_structured_field = lambda *args: 7\n'
        with self.assertRaisesRegex(ValueError, 'cannot modify attributes'):
            host['validate_program'](mutation + 'def extract(data, metadata):\n    return {}\n')
        program = ('import galgear_literals\n'
                   'def extract(data, metadata):\n'
                   '    return galgear_literals.parse_document(data, ' + repr(DOCUMENT) + ', view=lambda *args: data)\n')
        with self.assertRaisesRegex(RuntimeError, 'unexpected keyword argument'):
            host['run_parser'](program, DATA, {})


if __name__ == '__main__':
    unittest.main()
