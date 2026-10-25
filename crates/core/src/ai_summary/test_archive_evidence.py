"""Synthetic archive checks bind decoder output to real native metadata."""
import pathlib
import runpy
import struct
import tempfile
import unittest
import zlib

import test_agent_host as fixtures

native = runpy.run_path(str(pathlib.Path(__file__).with_name('archive_evidence.py')))
host = fixtures.host


def chunk(tag, value):
    return tag + struct.pack('<Q', len(value)) + value


def archive_bytes(plain, *, stored=None, name='chapter.scn', checksum=None,
                  compressed_index=False, duplicate=False, size=None, compress_member=False):
    stored = plain if stored is None else stored
    raw_size = len(stored)
    stored = zlib.compress(stored) if compress_member else stored
    encoded = name.encode('utf-16-le')
    info = struct.pack('<IQQH', 0x80000000, len(plain) if size is None else size, len(stored), len(encoded) // 2) + encoded
    segments = struct.pack('<IQQQ', int(compress_member), 19, raw_size, len(stored))
    checksum = zlib.adler32(plain) & 0xffffffff if checksum is None else checksum
    index = chunk(b'File', chunk(b'info', info) + chunk(b'segm', segments) + chunk(b'adlr', struct.pack('<I', checksum)))
    if duplicate:
        index += index.replace(encoded, duplicate.encode('utf-16-le')) if isinstance(duplicate, str) else index
    packet = (b'\1' + struct.pack('<QQ', len(zlib.compress(index)), len(index)) + zlib.compress(index)
              if compressed_index else b'\0' + struct.pack('<Q', len(index)) + index)
    return native['MAGIC'] + struct.pack('<Q', 19 + len(stored)) + stored + packet


def mixed_archive(specifications):
    """Independent synthetic File records; no production archive sample bytes."""
    payload, index = b'', b''
    for spec in specifications:
        raw = spec['raw']
        packed = zlib.compress(raw) if spec.get('compressed') else raw
        if 'name_tail' in spec:
            tail = spec['name_tail']
        else:
            name = spec['name'].encode('utf-16-le')
            tail = struct.pack('<H', len(name)//2) + name
        info = spec.get('info', struct.pack('<IQQ', spec.get('flags', 0x80000000),
            spec.get('info_size', len(raw)), spec.get('info_packed', len(packed))) + tail)
        segments = struct.pack('<IQQQ', int(bool(spec.get('compressed'))), 19+len(payload), len(raw), len(packed))
        segments = spec.get('segments', segments)
        checksum = spec.get('checksum', struct.pack('<I', zlib.adler32(spec.get('restored', raw)) & 0xffffffff))
        index += chunk(b'File', chunk(b'info', info) + chunk(b'segm', segments) + chunk(b'adlr', checksum))
        payload += packed
    return native['MAGIC'] + struct.pack('<Q', 19+len(payload)) + payload + b'\0' + struct.pack('<Q', len(index)) + index


class NativeArchiveEvidenceTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = pathlib.Path(temporary.name)
        self.archive = self.root / 'data.xp3'
        self.plain = b'Complete synthetic resource bytes'

    def test_plain_and_compressed_indices_bind_full_member_size_and_native_checksum(self):
        for packed in (False, True):
            with self.subTest(compressed_index=packed):
                self.archive.write_bytes(archive_bytes(self.plain, compressed_index=packed))
                receipt = native['verify_member'](self.archive, 'chapter.scn', self.plain)
                self.assertTrue(receipt['verified'])
                self.assertEqual(receipt['adler32'], zlib.adler32(self.plain) & 0xffffffff)
                self.assertEqual(receipt['bytes'], len(self.plain))

    def test_partial_readable_output_and_forged_checksum_cannot_pass(self):
        self.archive.write_bytes(archive_bytes(self.plain))
        for output in (self.plain[:8], self.plain[:-1] + b'X', self.plain + b'X'):
            with self.subTest(length=len(output)), self.assertRaisesRegex(ValueError, 'whole restored member'):
                native['verify_member'](self.archive, 'chapter.scn', output)
        self.archive.write_bytes(archive_bytes(self.plain, checksum=7))
        with self.assertRaisesRegex(ValueError, 'whole restored member'):
            native['verify_member'](self.archive, 'chapter.scn', self.plain)

    def test_ambiguous_names_truncated_index_and_segment_size_disagreement_fail_closed(self):
        variants = [archive_bytes(self.plain, duplicate=True), archive_bytes(self.plain)[:-1],
                    archive_bytes(self.plain, size=len(self.plain) + 1)]
        for data in variants:
            with self.subTest(length=len(data)):
                self.archive.write_bytes(data)
                with self.assertRaises(ValueError):
                    native['verify_member'](self.archive, 'chapter.scn', self.plain)

    def test_extracted_member_identity_must_exist_in_the_actual_native_index(self):
        self.archive.write_bytes(archive_bytes(self.plain))
        with self.assertRaisesRegex(ValueError, 'exact native index name'):
            native['verify_member'](self.archive, 'another.scn', self.plain)

    def test_mapped_names_need_unique_complete_raw_segment_equality(self):
        raw = bytes(value ^ 0x42 for value in self.plain)
        self.archive.write_bytes(archive_bytes(self.plain, stored=raw, name='0123456789abcdef', compress_member=True))
        receipt = native['verify_member'](self.archive, 'readable.scn', self.plain, original=raw)
        self.assertEqual(receipt['member'], '0123456789abcdef')
        self.assertEqual(receipt['member_identity'], 'unique_complete_native_segment_bytes')
        with self.assertRaisesRegex(ValueError, 'exact native index name'):
            native['verify_member'](self.archive, 'readable.scn', self.plain, original=raw[:-1] + b'X')
        self.archive.write_bytes(archive_bytes(self.plain, stored=raw, name='aaaaaaaaaaaaaaaa', duplicate='bbbbbbbbbbbbbbbb'))
        with self.assertRaisesRegex(ValueError, 'multiple native members'):
            native['verify_member'](self.archive, 'readable.scn', self.plain, original=raw)

    def test_host_applies_model_transform_but_reads_expected_checksum_from_archive(self):
        (self.root / 'game.exe').write_bytes(b'fixture')
        agent = host.Agent(dict(game=str(self.root), save=str(self.root), skill=str(self.root),
                                work=str(self.root / '.galgear-ai-summary' / 'fresh')))
        encrypted = bytes(value ^ 0x42 for value in self.plain)
        self.archive.write_bytes(archive_bytes(self.plain, stored=encrypted))
        extracted = agent.work / 'unpacked'
        extracted.mkdir()
        target = extracted / 'chapter.scn'
        target.write_bytes(encrypted)
        agent.extracted_from[str(extracted)] = str(self.archive)
        decoded, receipt = agent.verify_member_filter(target, encrypted, dict(member_filter=[dict(op='xor', key='42')]))
        self.assertEqual(decoded, self.plain)
        self.assertTrue(receipt['verified'])
        with self.assertRaisesRegex(ValueError, 'whole restored member'):
            agent.verify_member_filter(target, encrypted, dict(member_filter=[dict(op='xor', key='43')], checksum=zlib.adler32(encrypted)))
        with self.assertRaisesRegex(ValueError, 'top-level member_filter'):
            agent.verify_member_filter(target, encrypted, dict(rows=[dict(evidence=dict(kind='typed_field',
                document=dict(syntax='psb', view=[dict(op='xor', key='42')])))]))

    def inspection_fixture(self):
        (self.root / 'game.exe').write_bytes(b'fixture')
        agent = host.Agent(dict(game=str(self.root), save=str(self.root), skill=str(self.root),
                                work=str(self.root / '.galgear-ai-summary' / 'fresh'), language='en-US'))
        raw = bytes(value ^ 0x42 for value in self.plain)
        self.archive.write_bytes(archive_bytes(self.plain, stored=raw, name='0123456789abcdef', compress_member=True))
        extracted = agent.work / 'unpacked'
        extracted.mkdir()
        target = extracted / 'readable.scn'
        target.write_bytes(raw)
        agent.extracted_from[str(extracted)] = str(self.archive)
        agent.extraction_stamps[str(extracted)] = agent.file_fingerprint(self.archive)
        args = dict(archive='data.xp3', root='work', path='unpacked/readable.scn')
        return agent, target, args

    def test_metadata_tool_exposes_native_data_with_a_full_byte_identity_receipt(self):
        agent, target, args = self.inspection_fixture()
        result = agent.dispatch('inspect_archive_member', args)
        self.assertEqual(result['native']['adler32'], zlib.adler32(self.plain) & 0xffffffff)
        self.assertEqual(result['native']['size'], len(self.plain))
        self.assertEqual(result['binding']['member_identity'], 'unique_complete_native_segment_bytes')
        self.assertTrue(result['binding']['original_member_bound'])
        self.assertFalse(result['restored_member_verified'])
        self.assertFalse(result['host_verified_current_node'])
        self.assertEqual(result['segment_count'], 1)
        reference = result['full_metadata']
        self.assertTrue((agent.work / reference['path']).is_file())
        self.assertTrue(agent.dispatch('inspect_archive_member', args)['reused'])

    def test_metadata_tool_rejects_guessed_provenance_stale_archives_and_changed_raw_bytes(self):
        agent, target, args = self.inspection_fixture()
        for invalid in (dict(args, root='game'), dict(args, path='source-inventory.json'), dict(args, archive='other.xp3')):
            with self.subTest(arguments=invalid), self.assertRaises((ValueError, OSError)):
                agent.dispatch('inspect_archive_member', invalid)
        original = target.read_bytes()
        target.write_bytes(original[:-1] + b'X')
        with self.assertRaisesRegex(ValueError, 'native index name'):
            agent.dispatch('inspect_archive_member', args)
        target.write_bytes(original)
        self.archive.write_bytes(self.archive.read_bytes() + b'changed archive bytes')
        with self.assertRaisesRegex(ValueError, 'Source archive changed'):
            agent.dispatch('inspect_archive_member', args)

    def test_pure_candidate_checksum_is_available_without_any_verification_privilege(self):
        result = host.run_parser('import zlib\ndef extract(data, metadata): return zlib.adler32(data)', self.plain, {})
        self.assertEqual(result, zlib.adler32(self.plain) & 0xffffffff)

    def test_unrelated_opaque_name_tails_bind_only_by_complete_native_segment_bytes(self):
        fixtures = [(b'\x06\0a\0b\0c\0d\0e\0f\0extension', b'Independent fixture one.\n', 0x2a),
                    (b'CUSTOM\xff\x80\0encoded-name', b'*branch|A separate fixture\nAnother message.\n', 0x91)]
        for tail, plain, key in fixtures:
            with self.subTest(tail_bytes=len(tail)):
                raw = bytes(value ^ key for value in plain)
                self.archive.write_bytes(mixed_archive([dict(name_tail=tail, raw=raw, restored=plain, compressed=True)]))
                parsed = native['read_xp3_index'](self.archive)
                self.assertEqual(parsed['members'], {})
                entry = parsed['entries'][0]
                self.assertIsNone(entry['name'])
                self.assertEqual(entry['name_kind'], 'opaque_native_tail')
                self.assertEqual(entry['name_tail_hex'], tail.hex())
                self.assertEqual(len(entry['entry_id']), 64)
                bound = native['identify_member'](self.archive, 'readable-output.bin', original=raw)
                self.assertEqual(bound['native']['entry_id'], entry['entry_id'])
                self.assertEqual(bound['member_identity'], 'unique_complete_native_segment_bytes')
                receipt = native['verify_member'](self.archive, 'readable-output.bin', plain, original=raw)
                self.assertTrue(receipt['verified'])
                self.assertEqual(receipt['entry_id'], entry['entry_id'])
                with self.assertRaisesRegex(ValueError, 'native index name'):
                    native['identify_member'](self.archive, 'readable-output.bin')

    def test_one_opaque_name_does_not_block_a_different_known_native_member(self):
        self.archive.write_bytes(mixed_archive([dict(name='known.txt', raw=b'Known bytes'),
            dict(name_tail=b'\xffOPAQUE-NAME-VARIANT', raw=b'Other independent bytes', compressed=True)]))
        parsed = native['read_xp3_index'](self.archive)
        self.assertEqual(len(parsed['entries']), 2)
        self.assertEqual(list(parsed['members']), ['known.txt'])
        self.assertNotEqual(parsed['entries'][0]['entry_id'], parsed['entries'][1]['entry_id'])
        self.assertTrue(native['verify_member'](self.archive, 'known.txt', b'Known bytes', original=b'Known bytes')['verified'])

    def test_complete_size_disagreement_is_quarantined_without_blocking_other_members(self):
        bad = dict(name='unrelated.bin', raw=b'Other bytes', flags=0, info_size=47, info_packed=31)
        good = dict(name='known.txt', raw=b'A complete independent member')
        self.archive.write_bytes(mixed_archive([bad, good]))
        parsed = native['read_xp3_index'](self.archive)
        invalid, valid = parsed['entries']
        self.assertFalse(invalid['size_consistent'])
        self.assertEqual((invalid['size'], invalid['packed']), (47, 31))
        self.assertEqual((invalid['segment_size'], invalid['segment_packed']), (11, 11))
        self.assertTrue(valid['size_consistent'])
        self.assertTrue(native['verify_member'](self.archive, good['name'], good['raw'], original=good['raw'])['verified'])
        for original in (None, bad['raw']):
            with self.subTest(original=original), self.assertRaisesRegex(ValueError, 'quarantined.*cannot issue a receipt'):
                native['identify_member'](self.archive, bad['name'], original=original)
        with self.assertRaisesRegex(ValueError, 'quarantined'):
            native['verify_member'](self.archive, bad['name'], bad['raw'], original=bad['raw'])

    def test_quarantined_opaque_raw_candidate_cannot_be_ignored_for_unique_identity(self):
        raw = b'Exact bytes shared by different native entries'
        bad = dict(name_tail=b'opaque-layout-alpha', raw=raw, compressed=True, info_size=3, info_packed=2)
        good = dict(name='native-hash', raw=raw)
        self.archive.write_bytes(mixed_archive([bad, good]))
        with self.assertRaisesRegex(ValueError, 'multiple native members'):
            native['identify_member'](self.archive, 'readable.bin', original=raw)
        self.archive.write_bytes(mixed_archive([bad]))
        with self.assertRaisesRegex(ValueError, 'quarantined'):
            native['identify_member'](self.archive, 'readable.bin', original=raw)
        # Both info length directions must retain the actual raw candidate.
        self.archive.write_bytes(mixed_archive([dict(bad, info_size=len(raw)+1000), good]))
        with self.assertRaisesRegex(ValueError, 'multiple native members'):
            native['identify_member'](self.archive, 'readable.bin', original=raw)

    def test_valid_and_quarantined_duplicate_known_names_always_reject_the_complete_index(self):
        good = dict(name='same.txt', raw=b'First independent bytes')
        bad = dict(name='SAME.TXT', raw=b'Other bytes', info_size=1)
        for entries in ([bad, good], [good, bad], [bad, dict(bad, name='same.txt')]):
            with self.subTest(order=[entry['name'] for entry in entries]):
                self.archive.write_bytes(mixed_archive(entries))
                with self.assertRaisesRegex(ValueError, 'duplicate native member name'):
                    native['read_xp3_index'](self.archive)

    def test_quarantine_does_not_relax_complete_index_or_segment_boundaries(self):
        good = dict(name='valid.txt', raw=b'Valid member')
        bad = dict(name='quarantined.bin', raw=b'Independent member', info_size=2)
        outside = struct.pack('<IQQQ', 0, 1 << 40, len(bad['raw']), len(bad['raw']))
        for broken in (dict(segments=outside), dict(segments=b'X'*27), dict(info=b'X'*19), dict(checksum=b'X'*3)):
            with self.subTest(fields=list(broken)):
                self.archive.write_bytes(mixed_archive([good, dict(bad, **broken)]))
                with self.assertRaises(ValueError):
                    native['identify_member'](self.archive, good['name'], original=good['raw'])
        self.archive.write_bytes(mixed_archive([good, bad])[:-1])
        with self.assertRaisesRegex(ValueError, 'index read exceeds'):
            native['identify_member'](self.archive, good['name'], original=good['raw'])

    def test_complete_multi_segment_identity_never_discards_later_segments(self):
        raw = b'OneTwoOther'
        segments = struct.pack('<IQQQ', 0, 19, 3, 3) + struct.pack('<IQQQ', 0, 22, len(raw)-3, len(raw)-3)
        spec = dict(name_tail=b'opaque-layout-beta', raw=raw, segments=segments)
        self.archive.write_bytes(mixed_archive([spec]))
        bound = native['identify_member'](self.archive, 'readable.bin', original=raw)
        self.assertEqual(len(bound['native']['segments']), 2)
        self.assertTrue(native['verify_member'](self.archive, 'readable.bin', raw, original=raw)['verified'])
        with self.assertRaisesRegex(ValueError, 'exact native index name'):
            native['identify_member'](self.archive, 'readable.bin', original=raw[:3])
        self.archive.write_bytes(mixed_archive([dict(spec, info_size=3, info_packed=3)]))
        with self.assertRaisesRegex(ValueError, 'quarantined'):
            native['identify_member'](self.archive, 'readable.bin', original=raw)

    def test_opaque_and_known_raw_ambiguity_and_duplicate_known_names_remain_rejected(self):
        opaque = dict(name_tail=b'\xffopaque-v2', raw=b'Shared raw bytes')
        self.archive.write_bytes(mixed_archive([dict(name='first.txt', raw=b'Shared raw bytes'), opaque]))
        with self.assertRaisesRegex(ValueError, 'multiple native members'):
            native['identify_member'](self.archive, 'mapped-output.txt', original=b'Shared raw bytes')
        self.archive.write_bytes(mixed_archive([dict(name='same.txt', raw=b'One'),
            opaque, dict(name='same.txt', raw=b'Two')]))
        with self.assertRaisesRegex(ValueError, 'duplicate native member name'):
            native['read_xp3_index'](self.archive)

    def test_opaque_tail_does_not_relax_fixed_fields_segments_checksum_or_tamper_checks(self):
        base = dict(name_tail=b'\xffopaque', raw=b'Native raw bytes')
        for broken in (dict(info=b'\0'*19), dict(segments=b'\0'*27), dict(checksum=b'\0'*3)):
            with self.subTest(fields=list(broken)):
                self.archive.write_bytes(mixed_archive([dict(base, **broken)]))
                with self.assertRaises(ValueError):
                    native['read_xp3_index'](self.archive)
        self.archive.write_bytes(mixed_archive([base]))
        with self.assertRaisesRegex(ValueError, 'native index name'):
            native['identify_member'](self.archive, 'mapped.bin', original=b'Altered raw data')
        with self.assertRaisesRegex(ValueError, 'whole restored member'):
            native['verify_member'](self.archive, 'mapped.bin', b'Altered raw data', original=base['raw'])

    def test_opaque_metadata_preview_stays_bounded_with_full_tail_preserved_locally(self):
        agent, target, args = self.inspection_fixture()
        raw = target.read_bytes()
        tail = b'\xff'*5000
        self.archive.write_bytes(mixed_archive([dict(name_tail=tail, raw=raw, restored=self.plain)]))
        directory = str(target.parent)
        agent.extraction_stamps[directory] = agent.file_fingerprint(self.archive)
        result = agent.dispatch('inspect_archive_member', args)
        self.assertIsNone(result['native']['name'])
        self.assertEqual(result['native']['name_tail']['bytes'], len(tail))
        self.assertLess(len(host.CONTEXT['encode'](result)), host.CONTEXT['MAX_TOOL_BYTES'])
        reference = result['full_metadata']
        stored = host.json.loads((agent.work/reference['path']).read_text('utf-8'))
        self.assertEqual(stored['native']['name_tail_hex'], tail.hex())


if __name__ == '__main__':
    unittest.main()
