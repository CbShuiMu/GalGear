"""Per-input native declarations from synthetic archives, never game data."""
import json
import pathlib
import tempfile
import unittest
import zlib
from unittest import mock

import test_archive_evidence as fixtures


host, native = fixtures.host, fixtures.native


class NativeArchiveMetadataTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = pathlib.Path(temporary.name)
        self.game = self.root / 'synthetic-game'
        self.game.mkdir()
        (self.game / 'fixture.exe').write_bytes(b'Synthetic executable, never run')
        self.agent = host.Agent(dict(game=str(self.game), save=str(self.game), skill=str(self.game),
            work=str(self.game/'work'), language='en-US', title='Synthetic native metadata'))
        self.archive = self.game / 'resources.xp3'
        self.output = self.agent.work / 'unpacked'
        self.output.mkdir()

    def extract(self, specs, names=None):
        self.archive.write_bytes(fixtures.mixed_archive(specs))
        paths = []
        for spec, name in zip(specs, names or [spec['name'] for spec in specs]):
            path = self.output / name
            path.write_bytes(spec['raw'])
            paths.append(path)
            self.agent.resource_selections[path.name] = dict(selected=str(self.archive),
                input_stamps={str(self.archive):self.agent.file_fingerprint(self.archive)})
        self.agent.extracted_from[str(self.output)] = str(self.archive)
        self.agent.extraction_stamps[str(self.output)] = self.agent.file_fingerprint(self.archive)
        return paths

    def test_one_generic_parser_receives_two_independent_native_hashes_and_filters(self):
        specs = []
        for name, text, key in (('one.scn', b'Independent synthetic chapter Alpha.', 0x2a),
                                ('two.scn', b'Another separate synthetic chapter Beta!', 0x91)):
            specs.append(dict(name=name, raw=bytes(value ^ key for value in text), restored=text,
                              compressed=True))
        paths = self.extract(specs)
        # Same pure parser for both files; no filename, checksum or per-file key
        # appears in it. Bounded candidate enumeration is only a synthetic test.
        self.agent.programs['script'] = '''import zlib
def extract(data, metadata):
    native = metadata["native_archive"]
    for key in range(256):
        restored = bytes(value ^ key for value in data)
        if len(restored) == native["size"] and zlib.adler32(restored) & 0xffffffff == native["adler32"]:
            return {"native_archive_echo":native,"member_filter":[{"op":"xor","key":bytes([key]).hex()}],
                    "rows":[{"position":0,"text":restored.decode("ascii"),"speaker":None,
                             "evidence":{"offset":0,"length":len(restored),"encoding":"ascii"}}]}
    raise ValueError("No bounded synthetic candidate matches actual native declaration")
'''
        seen = []
        parser = host.run_parser
        def actual_parser(source, data, metadata, **kwargs):
            seen.append(json.loads(json.dumps(metadata)))
            return parser(source, data, metadata, **kwargs)
        for path, spec in zip(paths, specs):
            expected = native['native_member_metadata'](self.archive,path.name,spec['raw'])
            helper = self.agent.metadata_input(path)
            actual_helper = parser('def extract(data, metadata): return metadata["native_archive"]', spec['raw'], helper)
            self.assertEqual(actual_helper, expected)
            with mock.patch.object(host,'run_parser',side_effect=actual_parser), mock.patch.object(self.agent,'require_script_family'):
                result = self.agent.dispatch('test_program',dict(kind='script',root='work',path=path.relative_to(self.agent.work).as_posix()))
            self.assertEqual(result['count'], 1)
            self.assertEqual(seen[-1]['native_archive'], actual_helper)
            self.assertEqual(self.agent.parts[str(path)][0]['text'],spec['restored'].decode('ascii'))
            self.assertEqual(expected['adler32'],zlib.adler32(spec['restored']) & 0xffffffff)
            self.assertEqual(expected['scope'],'native_declaration_only')
            self.assertEqual(set(expected),{'format','adler32','size','packed','flags','entry_id','index_sha256','scope'})
            self.assertNotIn('verified',expected)
        self.assertNotEqual(seen[0]['native_archive']['adler32'],seen[1]['native_archive']['adler32'])
        self.assertNotEqual(seen[0]['native_archive']['entry_id'],seen[1]['native_archive']['entry_id'])
        self.assertEqual(seen[0]['native_archive']['index_sha256'],seen[1]['native_archive']['index_sha256'])
        self.assertEqual(self.agent.requests_sent,0)

    def test_raw_binding_is_mandatory_and_no_checksum_or_decoder_input_is_accepted(self):
        spec = dict(name='one.scn',raw=b'Actual complete raw member')
        path, = self.extract([spec])
        for invalid in (None,bytearray(spec['raw']),spec['raw'][:-1],b'Changed bytes'):
            with self.subTest(kind=type(invalid).__name__), self.assertRaises(ValueError):
                native['native_member_metadata'](self.archive,path.name,invalid)
        with self.assertRaises(TypeError):
            native['native_member_metadata'](self.archive,path.name,spec['raw'],checksum=1)
        with self.assertRaises(TypeError):
            native['native_member_metadata'](self.archive,path.name,spec['raw'],decoder='external')

    def test_quarantined_or_ambiguous_members_never_get_optional_metadata(self):
        raw = b'Independent complete member'
        variants = [([dict(name='one.scn',raw=raw,info_size=3)],['one.scn']),
                    ([dict(name='opaque-a',raw=raw),dict(name='opaque-b',raw=raw)],['mapped-a.scn','mapped-b.scn'])]
        for specs,names in variants:
            with self.subTest(members=len(specs)):
                paths = self.extract(specs,names)
                with self.assertRaises(ValueError):
                    native['native_member_metadata'](self.archive,paths[0].name,raw)
                self.assertIsNone(self.agent.native_archive_metadata(paths[0]))
                self.assertNotIn('native_archive',self.agent.metadata_input(paths[0]))

    def test_already_decrypted_tool_output_and_unsupported_archives_keep_old_no_filter_behavior(self):
        plain = b'Legitimate decoded tool output'
        raw = bytes(value ^ 0x55 for value in plain)
        path, = self.extract([dict(name='one.scn',raw=raw,restored=plain)])
        path.write_bytes(plain)
        self.assertNotIn('native_archive',self.agent.metadata_input(path))
        self.assertEqual(self.agent.verify_member_filter(path,plain,dict(rows=[])),(plain,None))
        self.archive.write_bytes(b'Another engine archive, not XP3')
        self.agent.extraction_stamps[str(self.output)] = self.agent.file_fingerprint(self.archive)
        self.agent.resource_selections.clear()
        self.assertNotIn('native_archive',self.agent.metadata_input(path))
        self.assertEqual(self.agent.verify_member_filter(path,plain,dict(rows=[])),(plain,None))
        # A standard signature with unsupported index encoding is optional too.
        self.archive.write_bytes(native['MAGIC']+(19).to_bytes(8,'little')+b'\x02'+b'\0'*8)
        self.agent.extraction_stamps[str(self.output)] = self.agent.file_fingerprint(self.archive)
        self.assertNotIn('native_archive',self.agent.metadata_input(path))

    def test_current_extraction_stamp_and_cached_raw_input_are_rechecked(self):
        raw = b'Complete synthetic raw input'
        path, = self.extract([dict(name='one.scn',raw=raw)])
        self.assertIsNotNone(self.agent.native_archive_metadata(path))
        path.write_bytes(raw[:-1]+b'X')
        self.assertIsNone(self.agent.native_archive_metadata(path,data=raw))
        self.assertIsNone(self.agent.native_archive_metadata(path))
        path.write_bytes(raw)
        self.archive.write_bytes(self.archive.read_bytes()+b'changed-source')
        with self.assertRaisesRegex(ValueError,'Source archive changed'):
            self.agent.metadata_input(path)
        self.agent.programs['script'] = 'def extract(data, metadata): return []'
        with self.assertRaisesRegex(ValueError,'Source archive changed'):
            self.agent.dispatch('test_program',dict(kind='script',root='work',path=path.relative_to(self.agent.work).as_posix()))

    def test_no_loose_file_outside_origin_or_overlapping_extractions_get_metadata(self):
        raw = b'Origin isolated bytes'
        path, = self.extract([dict(name='one.scn',raw=raw)])
        loose = self.agent.work / 'loose.scn'
        loose.write_bytes(raw)
        self.assertIsNone(self.agent.native_archive_metadata(loose))
        self.assertIsNone(self.agent.native_archive_metadata(self.game/'fixture.exe'))
        self.agent.extracted_from[str(self.agent.work)] = str(self.archive)
        self.agent.extraction_stamps[str(self.agent.work)] = self.agent.file_fingerprint(self.archive)
        self.assertIsNone(self.agent.native_archive_metadata(path))
        self.agent.extracted_from.pop(str(self.agent.work))
        outside = self.root / 'outside.xp3'
        outside.write_bytes(self.archive.read_bytes())
        self.agent.extracted_from[str(self.output)] = str(outside)
        self.agent.extraction_stamps[str(self.output)] = self.agent.file_fingerprint(outside)
        self.assertIsNone(self.agent.native_archive_metadata(path))

    def test_mapped_name_uses_whole_raw_bytes_not_filename_size_or_user_metadata(self):
        raw = b'Unique opaque member bytes'
        path, = self.extract([dict(name='opaque-native-name',raw=raw)],['readable.scn'])
        observation = self.agent.metadata_input(path)['native_archive']
        self.assertEqual(observation['adler32'],zlib.adler32(raw)&0xffffffff)
        path.write_bytes(raw[:-1]+b'!')
        self.assertNotIn('native_archive',self.agent.metadata_input(path))


if __name__ == '__main__':
    unittest.main()
