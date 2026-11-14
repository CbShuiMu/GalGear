"""Offline reader-boundary tests; no games, credentials or provider calls."""
import importlib.util
import json
from pathlib import Path
import struct
import tempfile
import unittest
from unittest import mock

spec = importlib.util.spec_from_file_location('skill_readers', Path(__file__).with_name('agent_skill_readers.py'))
readers = importlib.util.module_from_spec(spec)
spec.loader.exec_module(readers)


class SkillReaderTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        self.input = self.root / 'sample.ast'
        self.input.write_bytes(b'ast={text={}}')
        script = self.root / 'scripts/parse_ast_tables.py'
        script.parent.mkdir()
        script.write_text('def extract(path, name, allow_duplicate_keys=False):\n    return [], []\n'
                          'def parse(text, allow_duplicate_keys=False):\n    return {"input_text":text}\n', encoding='utf-8')
        self.config = dict(skill=str(self.root), input=str(self.input), reader='ast_tables')

    def test_block_local_tables_expose_choices_and_keep_commands_uninterpreted(self):
        project=readers.runpy.run_path(str(Path(readers.__file__).with_name('ast_observations.py')))['project']
        tree={'block_7': {1:{1:'select', 'text':'select', 'file':'left', 'exp':'x=x+1'},
            'select':{'ja':{1:'先询问同伴',2:'独自前进'}},
            'text':{'ja':{1:{1:'抵达港口。',2:{1:'ruby',2:'unused'},'name':{1:'未确认身份'}}}},
            'line':12,'linknext':'block_8'}, 'label':{'start':'block_7'}}
        result=project(tree,'scene.ast')
        self.assertEqual([r['text'] for r in result['choice_texts']],['先询问同伴','独自前进'])
        self.assertEqual(result['dialogue'][0]['text'],'抵达港口。')
        self.assertIsNone(result['dialogue'][0]['speaker'])
        self.assertEqual(result['choice_texts'][0]['pointer'],'/ast_tree/block_7')
        self.assertEqual(result['block_observations'][0]['commands'][0]['arguments']['exp'],'x=x+1')
        self.assertFalse(result['choice_texts'][0]['runtime_verified'])

    def test_installed_reader_observations_never_grant_node_validation(self):
        result = readers.worker(self.config)
        self.assertTrue(result['analysis_only'])
        self.assertFalse(result['verified_current_node'])
        self.assertEqual(result['reader'], 'ast_tables')
        self.assertEqual(len(result['source_sha256']), 64)

    def test_reader_cannot_select_arbitrary_code_or_options(self):
        for changed in (dict(reader='../other.py'), dict(options={'source': 'print(1)'}),
                        dict(options={'allow_duplicate_keys': 'yes'})):
            with self.subTest(changed=changed), self.assertRaises(ValueError):
                readers.worker(dict(self.config, **changed))

    def test_declarative_ast_header_preserves_original_byte_mapping(self):
        header = 'astver = 2.0\r\nastname = "ast"\r\n'
        data = b'\xef\xbb\xbf' + (header + 'ast={text={}}').encode()
        self.input.write_bytes(data)
        result = readers.worker(self.config)
        self.assertEqual(result['ast_tree']['input_text'], 'ast={text={}}')
        self.assertEqual(result['input_view'], dict(offset=3 + len(header), length=13))
        self.input.write_bytes(b'run_game()\nast={text={}}')
        result = readers.worker(self.config)
        self.assertTrue(result['ast_tree']['input_text'].startswith('run_game()'))
        self.assertEqual(result['input_view']['offset'], 0)

    def test_wrong_scene_format_is_rejected_before_loading_the_reader(self):
        script = self.root / 'scripts/parse_siglus_flow.py'
        script.touch()
        self.input.write_bytes(b'PSB\0' + bytes(200))
        with self.assertRaisesRegex(ValueError, 'inspect_structured'), mock.patch.object(readers.runpy, 'run_path') as run:
            readers.worker(dict(self.config, reader='siglus_scene'))
        run.assert_not_called()

    def test_parent_uses_fixed_worker_mode_and_short_timeout(self):
        with mock.patch.object(readers.subprocess, 'run', return_value=mock.Mock(returncode=0, stdout=b'{}')) as run:
            readers.run(['host'], self.root, self.input, 'ast_tables', timeout=12)
        self.assertEqual(run.call_args.args[0], ['host', '--skill-reader'])
        self.assertEqual(run.call_args.kwargs['timeout'], 12)
        self.assertEqual(json.loads(run.call_args.kwargs['input'])['input'], str(self.input))

    def test_save_recognition_requires_native_header_fields(self):
        self.assertEqual(readers.save_reader(b'BOWS' + struct.pack('<II', 1002, 1) + bytes(4)), 'artemis_save')
        self.assertIsNone(readers.save_reader(b'BOWS' + bytes(20)))
        head = struct.pack('<10i', 1, 0, 2025, 5, 3, 6, 12, 0, 0, 0)
        self.assertEqual(readers.save_reader(head), 'siglus_save')
        self.assertIsNone(readers.save_reader(head[:30]))

    def test_script_suggestions_use_headers_and_remain_format_candidates(self):
        self.assertEqual(readers.script_reader(b'\xef\xbb\xbfastver = 2.0\r\nastname="ast"\r\n'), 'ast_tables')
        self.assertEqual(readers.script_reader(b' ast = {text={}}'), 'ast_tables')
        self.assertEqual(readers.script_reader(struct.pack('<I', 132)+bytes(128)), 'siglus_scene')
        self.assertIsNone(readers.script_reader(b'PSB\0'+bytes(128)))
        self.assertIsNone(readers.script_reader(b'ordinary data'))

    def test_bitmap_text_hint_uses_pixel_extent_and_rejects_compression(self):
        header = bytearray(54)
        header[:2] = b'BM'
        struct.pack_into('<I', header, 10, 54)
        struct.pack_into('<IiiHHI', header, 14, 40, 1, 1, 1, 24, 0)
        data = header + bytes(4) + b'\xfe\xfe\x02\xff\xfe'
        document = readers.text_container(data)
        self.assertEqual(document['view'][0], dict(op='slice', offset=58, length=5))
        self.assertEqual(document['view'][1], dict(op='tjs_text_stream'))
        struct.pack_into('<I', data, 30, 1)
        self.assertIsNone(readers.text_container(data))


if __name__ == '__main__':
    unittest.main()
