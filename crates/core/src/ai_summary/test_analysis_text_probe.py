from pathlib import Path
import runpy
import tempfile
import unittest
import zlib

BASE=Path(__file__).resolve().parent
PROBE=runpy.run_path(str(BASE/'analysis_text_probe.py'))
READS=runpy.run_path(str(BASE/'analysis_reads.py'))
API=runpy.run_path(str(BASE/'agent_host.py'))


class TextProbeTests(unittest.TestCase):
    def test_common_compression_and_different_encodings_preserve_byte_identity(self):
        for encoding in ('utf-8','utf-16-le','utf-16-be','cp932','gb18030'):
            with self.subTest(encoding=encoding):
                text='「一緒に帰りましょう。」'
                body=b'chapter_03\0'+bytes(5)+text.encode(encoding)+bytes(3)
                raw=bytes(37)+zlib.compress(body)
                views=PROBE['probe'](raw)['views']
                self.assertEqual(views[1]['compressed_offset'],37)
                match=next(row for row in views[1]['text_candidates'] if row['text']==text and row['encoding']==encoding)
                self.assertEqual(body[match['offset']:match['offset']+match['length']].decode(encoding),text)
                self.assertEqual(views[1]['identifiers'][0]['text'],'chapter_03')

    def test_corrupt_compression_is_only_a_missing_candidate(self):
        result=PROBE['probe'](b'header'+zlib.compress(b'data')[:-2])
        self.assertEqual(len(result['views']),1)
        self.assertIn('never active-state proof',result['scope'])

    def test_auto_unknown_save_returns_registered_caption_hints(self):
        with tempfile.TemporaryDirectory() as temp:
            path=Path(temp)/'slot.dat'
            path.write_bytes(bytes(16)+zlib.compress(b'chapter_03\0'+'「一起回去吧，好吗？」'.encode('utf-8')))
            value=READS['read_one'](Path(temp),path,'auto',dict(probe_unknown=True),API)
            self.assertEqual(value['reader'],'probe')
            self.assertFalse(value['verified_current_node'])
            self.assertEqual(value['anchor_hints'][0]['value'],'「一起回去吧，好吗？」')

    def test_type_name_diagnostics_do_not_expose_reflection_or_class_creation(self):
        self.assertEqual(API['execute_pure']('def f(value): return type(value).__name__','f',{}),'dict')
        for source in ('def f(x): return x.__class__.__name__',
                       'def f(x): return type(x).__mro__',
                       'def f(x): return type("X", (), {})'):
            with self.subTest(source=source), self.assertRaises(ValueError):API['validate_program'](source)


if __name__=='__main__':unittest.main()
