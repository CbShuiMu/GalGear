import bz2
import lzma
import pathlib
import runpy
import unittest
import zlib

codecs = runpy.run_path(str(pathlib.Path(__file__).with_name('bytecodecs.py')))


class ByteCodecTests(unittest.TestCase):
    def test_standard_codecs_replay_exact_bytes(self):
        raw = ('Verified byte evidence. ' * 100).encode()
        raw_deflater = zlib.compressobj(wbits=-15)
        for name, packed in (('zlib', zlib.compress(raw)), ('bz2', bz2.compress(raw)),
                             ('lzma', lzma.compress(raw)),
                             ('deflate', raw_deflater.compress(raw) + raw_deflater.flush())):
            with self.subTest(codec=name):
                self.assertEqual(codecs['decode'](name, packed, len(raw)), raw)
                for broken in (packed[:-1], packed + b'trailing'):
                    with self.assertRaises(ValueError):
                        codecs['decode'](name, broken)

    def test_view_reads_output_size_from_native_bytes(self):
        raw = b'coordinate\0' * 30
        packed = zlib.compress(raw)
        data = len(raw).to_bytes(4, 'little') + packed
        steps = [dict(op='zlib', offset=4, length=len(packed), size_from=dict(offset=0, width=4))]
        self.assertEqual(codecs['view'](data, steps), raw)
        with self.assertRaises(ValueError):
            codecs['view']((len(raw) - 1).to_bytes(4, 'little') + packed, steps)

    def test_invalid_sizes_transforms_and_bomb_limits_are_rejected(self):
        packed = zlib.compress(b'a' * 100)
        for size in (True, -1, 33 * 1024 * 1024, 99, 101):
            with self.subTest(size=size), self.assertRaises(ValueError):
                codecs['decode']('zlib', packed, size)
        for steps in ([dict(op='slice', offset=True, length=1)], [dict(op='xor', key='')],
                      [dict(op='erisa_nemesis')], [dict(op='shell', command='anything')],
                      [dict(op='zlib')] * 9):
            with self.subTest(steps=steps), self.assertRaises(ValueError):
                codecs['view'](b'fixture', steps)

    def test_fixed_adjacent_bit_permutation_preserves_full_input(self):
        original = bytes(range(256))
        # Independently pair the textual bits rather than reusing the masks.
        expected = bytes(int(''.join(bits[index + 1] + bits[index]
                             for index in range(0, 8, 2)), 2)
                         for bits in (format(value, '08b') for value in original))
        steps = [dict(op='swap_adjacent_bits')]
        encoded = codecs['view'](original, steps)
        self.assertEqual(encoded, expected)
        self.assertEqual(codecs['view'](encoded, steps), original)
        self.assertEqual(len(encoded), len(original))
        with self.assertRaises(ValueError):
            codecs['view'](original, [dict(op='swap_adjacent_bits', offset=1)])

    def test_all_views_enforce_the_declared_input_bound(self):
        with self.assertRaisesRegex(ValueError, 'input byte limit'):
            codecs['view'](b'x' * (codecs['MAX_BYTES'] + 1), [])

    def test_generated_adapters_can_use_the_same_decoder(self):
        host = runpy.run_path(str(pathlib.Path(__file__).with_name('agent_host.py')))
        program = ('import galgear_codecs\n'
                   'def extract(data, metadata):\n'
                   '    return {"text": galgear_codecs.decode("zlib", data).decode("utf-8")}\n')
        self.assertEqual(host['run_parser'](program, zlib.compress(b'evidence'), {}), dict(text='evidence'))

    def test_adapter_cannot_modify_verifier_dialogue(self):
        host = runpy.run_path(str(pathlib.Path(__file__).with_name('agent_host.py')))
        program = '''def extract(data, metadata):
    metadata["dialogue"][0].update({"source":"route.lng", "position":12, "text":"Current"})
    return {"source":"route.lng", "line":12, "anchors":["Current"], "evidence":{
        "anchor":{"offset":0,"length":7,"encoding":"ascii"},
        "source":{"offset":8,"length":5,"encoding":"ascii"},
        "position":{"offset":14,"width":4}}}
'''
        with self.assertRaisesRegex(RuntimeError, 'ambiguous or disagrees'):
            host['run_parser'](program, b'Current|route|' + (12).to_bytes(4, 'little'),
                               dict(dialogue=[dict(source='other.lng', position=9, text='Older')]), True)

    def test_adapter_cannot_replace_the_shared_decoder(self):
        host = runpy.run_path(str(pathlib.Path(__file__).with_name('agent_host.py')))
        for mutation in ('zlib.decompressobj = lambda: zlib', 'del zlib.decompressobj',
                         'galgear_codecs.decode = lambda *args: b"forged"'):
            program = 'import zlib\nimport galgear_codecs\n' + mutation + '\ndef extract(data, metadata):\n    return {}\n'
            with self.subTest(mutation=mutation), self.assertRaisesRegex(ValueError, 'cannot modify attributes'):
                host['run_parser'](program, b'fixture', {})


if __name__ == '__main__':
    unittest.main()
