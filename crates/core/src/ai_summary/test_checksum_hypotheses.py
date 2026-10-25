"""Synthetic-only checks against independently recomputed zlib Adler-32."""
import json
import pathlib
import random
import runpy
import tempfile
import unittest
import zlib

BASE = pathlib.Path(__file__).parent
CHECKSUM = runpy.run_path(str(BASE / 'checksum_hypotheses.py'))
HOST = runpy.run_path(str(BASE / 'agent_host.py'))
scan = CHECKSUM['adler32_xor_prefix_lengths']


def brute(data, key, target):
    # Deliberately constructs and checks each independent full input. It does
    # not share the candidate's delta arithmetic or incremental checksum state.
    return [length for length in range(len(data) + 1)
            if zlib.adler32(bytes(value ^ key for value in data[:length]) + data[length:]) & 0xffffffff == target]


class ChecksumHypothesesTests(unittest.TestCase):
    def assert_brute(self, data, key, target, cap=64):
        expected = brute(data, key, target)
        self.assertEqual(scan(data, key, target, cap), dict(
            match_count=len(expected), lengths=expected[:cap],
            truncated=len(expected) > cap, checked_prefixes=len(data) + 1))

    def test_small_inputs_all_prefix_targets_against_full_zlib_brute_force(self):
        rng = random.Random(1950)
        for size in range(25):
            data = bytes(rng.randrange(256) for _ in range(size))
            for key in (1, 127, 128, 255):
                targets = {zlib.adler32(bytes(value ^ key for value in data[:length]) + data[length:]) & 0xffffffff
                           for length in range(size + 1)}
                targets.add(rng.randrange(1 << 32))
                for target in targets:
                    with self.subTest(size=size, key=key, target=target):
                        self.assert_brute(data, key, target, size % 7)

    def test_modular_sum_wrap_and_negative_deltas_against_brute_force(self):
        data = bytes([255]) * 300
        for length in (0, 1, 37, 257, 299, 300):
            target = zlib.adler32(bytes([0]) * length + data[length:]) & 0xffffffff
            self.assert_brute(data, 255, target)
        self.assert_brute(data, 255, 0xffffffff)
        data = bytes(300)
        for length in (0, 1, 37, 257, 299, 300):
            target = zlib.adler32(bytes([255]) * length + data[length:]) & 0xffffffff
            self.assert_brute(data, 255, target)

    def test_every_legal_key_on_short_synthetic_inputs(self):
        for data in (b'', bytes([0, 1, 1, 0]), bytes(range(11))):
            for key in range(1, 256):
                for length in range(len(data) + 1):
                    target = zlib.adler32(bytes(value ^ key for value in data[:length]) + data[length:]) & 0xffffffff
                    self.assert_brute(data, key, target, 2)

    def test_weight_crosses_modulus_and_count_continues_after_truncation(self):
        # The four deltas are +1,-1,-1,+1: both Adler changes cancel.
        # n > 65521 also exercises weights on both sides of the modulus.
        data = bytes([0, 1, 1, 0]) * 16382
        target = zlib.adler32(data) & 0xffffffff
        expected_count = len(data) // 4 + 1
        self.assertEqual(scan(data, 1, target, 3), dict(
            match_count=expected_count, lengths=[0, 4, 8], truncated=True,
            checked_prefixes=len(data) + 1))
        self.assertEqual(scan(data, 1, target, 0), dict(
            match_count=expected_count, lengths=[], truncated=True,
            checked_prefixes=len(data) + 1))

    def test_empty_input_and_full_prefix_inclusive(self):
        self.assertEqual(scan(b'', 17, 1), dict(match_count=1, lengths=[0], truncated=False, checked_prefixes=1))
        self.assertEqual(scan(b'', 17, 0), dict(match_count=0, lengths=[], truncated=False, checked_prefixes=1))
        data, key = b'synthetic boundary', 93
        self.assert_brute(data, key, zlib.adler32(bytes(value ^ key for value in data)) & 0xffffffff)

    def test_bytearray_is_unchanged_and_matches_bytes(self):
        data = bytearray(range(256))
        original = bytes(data)
        target = zlib.adler32(original) & 0xffffffff
        self.assertEqual(scan(data, 129, target), scan(original, 129, target))
        self.assertEqual(data, original)

    def test_strict_key_target_candidate_cap_and_data_types(self):
        for key in (0, -1, 256, True, False, 1.0, '01', None):
            with self.subTest(key=key), self.assertRaises(ValueError):
                scan(b'x', key, 1)
        for target in (-1, 1 << 32, True, False, 1.0, '1', None):
            with self.subTest(target=target), self.assertRaises(ValueError):
                scan(b'x', 1, target)
        for cap in (-1, 4097, True, False, 1.0, '1', None):
            with self.subTest(cap=cap), self.assertRaises(ValueError):
                scan(b'x', 1, 1, cap)
        class Derived(bytes):
            pass
        for data in ('x', [0], None, True, memoryview(b'x'), Derived(b'x')):
            with self.subTest(data_type=type(data)), self.assertRaises(TypeError):
                scan(data, 1, 1)
        with self.assertRaisesRegex(ValueError, '32 MiB'):
            scan(bytes(CHECKSUM['MAX_BYTES'] + 1), 1, 1)
        with self.assertRaises(TypeError):
            scan(b'x', 1, 1, unknown=True)

    def test_result_contains_only_numeric_hypotheses(self):
        result = scan(b'synthetic', 7, 1)
        self.assertEqual(set(result), {'match_count', 'lengths', 'truncated', 'checked_prefixes'})
        self.assertTrue(all(type(length) is int for length in result['lengths']))
        self.assertNotIn('adler32_xor_prefix_lengths', HOST['CODECS']['capabilities']()['transforms'])

    def test_original_host_child_sandbox_import_forms_and_external_access_denial(self):
        data = bytes([0, 1, 1, 0]) * 5
        metadata = {'key': 1, 'target': zlib.adler32(data) & 0xffffffff}
        programs = (
            'import galgear_codecs\ndef extract(data, metadata):\n'
            '    return galgear_codecs.adler32_xor_prefix_lengths(data,metadata["key"],metadata["target"],2)\n',
            'from galgear_codecs import adler32_xor_prefix_lengths\ndef extract(data, metadata):\n'
            '    return adler32_xor_prefix_lengths(data,metadata["key"],metadata["target"],2)\n')
        for program in programs:
            self.assertEqual(HOST['run_parser'](program, data, metadata), scan(data, 1, metadata['target'], 2))
        for forbidden in ('import pathlib', 'import socket', 'import subprocess', 'open("outside", "rb")',
                          'import galgear_codecs\ngalgear_codecs.adler32_xor_prefix_lengths = None'):
            with self.subTest(forbidden=forbidden), self.assertRaises((ValueError, RuntimeError)):
                HOST['run_parser'](forbidden + '\ndef extract(data, metadata):\n    return {}\n', data, {})
        with self.assertRaisesRegex(RuntimeError, 'nonzero byte key'):
            HOST['run_parser']('import galgear_codecs\ndef extract(data, metadata):\n'
                               '    return galgear_codecs.adler32_xor_prefix_lengths(data,0,1)\n', b'x', {})

    def test_initial_contract_and_focused_full_contract_discover_helper(self):
        self.assertIn('adler32_xor_prefix_lengths', HOST['BYTE_EVIDENCE_CONTRACT']['checksum_hypotheses'])
        caps = HOST['codec_capabilities']()
        self.assertIn('Caller-supplied checksum is not native archive evidence', caps['checksum_hypotheses']['scope'])
        with tempfile.TemporaryDirectory(prefix='synthetic-checksum-hypotheses-') as folder:
            game = pathlib.Path(folder) / 'synthetic-game'
            game.mkdir()
            (game / 'fixture.exe').write_bytes(b'synthetic')
            skill = pathlib.Path(folder) / 'empty-skill'
            skill.mkdir()
            agent = HOST['Agent'](dict(game=str(game), save=str(game), skill=str(skill), work=str(game / 'work'),
                                       provider='synthetic', model='none', title='Synthetic', language='en', max_turns=0))
            for topic in ('script', 'save', 'adapter'):
                contract = agent.dispatch('verification_contract', dict(topic=topic))
                self.assertIn('adler32_xor_prefix_lengths', contract['byte_codecs']['checksum_hypotheses'])
                self.assertIn('not native archive evidence', contract['byte_codecs']['checksum_hypotheses'])
            exported = agent.dispatch('verification_contract', dict(topic='all'))
            full = json.loads((game / 'work' / exported['full_contract']).read_text('utf-8'))
            self.assertEqual(full['byte_codecs']['checksum_hypotheses'], caps['checksum_hypotheses'])
            self.assertEqual(agent.requests_sent, 0)


if __name__ == '__main__':
    unittest.main()
