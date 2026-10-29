"""Offline tests for declarative TLV discovery; no game records are decoded."""
import copy
import importlib.util
import json
import pathlib
import tempfile
import unittest


spec = importlib.util.spec_from_file_location('agent_workflow', pathlib.Path(__file__).with_name('agent_workflow.py'))
workflow = importlib.util.module_from_spec(spec)
spec.loader.exec_module(workflow)


class ContainerPolicyTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = pathlib.Path(temporary.name)
        self.skill = self.root / 'skill'
        self.reference = self.skill / 'reference'
        self.reference.mkdir(parents=True)
        self.prefix = b'HEADMAGIC!!!'
        self.policy = dict(header=dict(offset=4, ascii='MAGIC'),
                           chunks=dict(offset=len(self.prefix), tag_width=4, length_width=2,
                                       endian='little', tag_ascii='TABL', max_chunks=3))
        self.catalog()

    def catalog(self, entries=None):
        if entries is None:
            entries = [dict(id='fixture', verification_policy=self.policy)]
        (self.reference / 'format-index.json').write_text(json.dumps(entries), encoding='utf-8-sig')

    def chunk(self, tag=b'TABL', body=b'opaque', endian='little'):
        return tag + len(body).to_bytes(2, endian) + body

    def regions(self, data):
        return workflow.required_table_regions(self.skill, data)

    def test_discovers_exact_body_without_decoding_opaque_record_bytes(self):
        body = b'\xff\x00\x80\xfeopaque'
        self.assertEqual(self.regions(self.prefix + self.chunk(body=body)),
                         [dict(offset=len(self.prefix) + 6, length=len(body))])

    def test_skips_unrelated_chunks_and_collects_all_nonempty_target_regions(self):
        unknown = self.chunk(tag=b'CODE', body=b'first')
        first = self.chunk(body=b'one')
        second = self.chunk(body=b'two two')
        self.assertEqual(self.regions(self.prefix + unknown + first + second),
                         [dict(offset=len(self.prefix) + len(unknown) + 6, length=3),
                          dict(offset=len(self.prefix) + len(unknown) + len(first) + 6, length=7)])

    def test_empty_target_and_supported_header_without_target_require_no_region(self):
        self.assertEqual(self.regions(self.prefix + self.chunk(body=b'')), [])
        self.assertEqual(self.regions(self.prefix + self.chunk(tag=b'CODE')), [])
        self.assertEqual(self.regions(self.prefix), [])

    def test_unmatched_header_never_attempts_to_parse_invalid_tlv(self):
        self.assertEqual(self.regions(b'not a supported header\xff'), [])
        self.assertEqual(self.regions(b''), [])
        self.assertEqual(self.regions(b'HEADOTHER!!!TABL\xff\xff'), [])

    def test_malformed_unmatched_identification_entries_are_skipped(self):
        self.catalog([None, 'not an entry', {}, dict(verification_policy=[]),
                      dict(verification_policy=dict(header=dict(offset=True, ascii='MAGIC'))),
                      dict(verification_policy=dict(header=dict(offset=4, ascii=7))),
                      dict(verification_policy=dict(header=dict(offset=4, ascii='\u975eASCII'))),
                      dict(verification_policy=dict(header=dict(offset=400, ascii='MAGIC'))),
                      dict(verification_policy=dict(header=dict(offset=0, ascii='OTHER'), chunks=False)),
                      dict(verification_policy=self.policy)])
        self.assertEqual(self.regions(self.prefix + self.chunk()),
                         [dict(offset=len(self.prefix) + 6, length=6)])

    def test_matching_header_requires_valid_chunk_policy(self):
        for chunks in (None, False, [], 'chunk settings'):
            with self.subTest(chunks=chunks):
                self.catalog([dict(verification_policy=dict(header=self.policy['header'], chunks=chunks))])
                with self.assertRaisesRegex(ValueError, 'Matched container verification policy'):
                    self.regions(self.prefix)

    def test_strict_integer_ranges_widths_endian_and_target_tag(self):
        invalid = [('offset', True), ('offset', -1), ('offset', len(self.prefix) + 1),
                   ('offset', '12'), ('tag_width', 0), ('tag_width', 65), ('tag_width', 4.0),
                   ('length_width', 0), ('length_width', 3), ('length_width', 9),
                   ('length_width', True), ('max_chunks', 0), ('max_chunks', 4097),
                   ('max_chunks', 3.0), ('endian', 'native'), ('endian', []),
                   ('tag_ascii', ''), ('tag_ascii', 'TOO-LONG'), ('tag_ascii', '\u8868\u683c'),
                   ('tag_ascii', '\x00' * 4), ('tag_ascii', None)]
        for name, value in invalid:
            with self.subTest(name=name, value=value):
                policy = copy.deepcopy(self.policy)
                policy['chunks'][name] = value
                self.catalog([dict(verification_policy=policy)])
                with self.assertRaisesRegex(ValueError, 'Matched container verification policy'):
                    self.regions(self.prefix)

    def test_truncated_header_and_body_are_rejected_after_identification(self):
        for tail in (b'T', b'TABL\x01', b'TABL\xff\xff', b'TABL\x04\x00abc'):
            with self.subTest(tail=tail), self.assertRaisesRegex(ValueError, 'Matched container verification policy'):
                self.regions(self.prefix + tail)

    def test_full_width_unsigned_length_cannot_wrap_or_escape_bounds(self):
        self.policy['chunks']['length_width'] = 8
        self.catalog()
        with self.assertRaisesRegex(ValueError, 'exceeds input boundary'):
            self.regions(self.prefix + b'TABL' + b'\xff' * 8)

    def test_chunk_count_limit_is_enforced_even_for_empty_unknown_chunks(self):
        self.policy['chunks']['max_chunks'] = 2
        self.catalog()
        empty = self.chunk(tag=b'CODE', body=b'')
        self.assertEqual(self.regions(self.prefix + empty * 2), [])
        with self.assertRaisesRegex(ValueError, 'exceeds max_chunks'):
            self.regions(self.prefix + empty * 3)

    def test_complete_zero_tag_header_terminates_before_trailing_junk(self):
        actual = self.chunk(body=b'table')
        zero_header = b'\x00' * 4 + b'\xff\xff'
        self.assertEqual(self.regions(self.prefix + actual + zero_header + b'unparsed trailer'),
                         [dict(offset=len(self.prefix) + 6, length=5)])

    def test_declarative_big_endian_policy_is_not_engine_specific(self):
        self.policy['chunks']['endian'] = 'big'
        self.catalog()
        body = b'opaque' * 50
        self.assertEqual(self.regions(self.prefix + self.chunk(body=body, endian='big')),
                         [dict(offset=len(self.prefix) + 6, length=len(body))])

    def test_wrapped_catalog_and_duplicate_policies_do_not_duplicate_regions(self):
        self.catalog(dict(formats=[dict(verification_policy=self.policy)] * 2))
        self.assertEqual(self.regions(self.prefix + self.chunk()),
                         [dict(offset=len(self.prefix) + 6, length=6)])

    def test_missing_catalog_and_entries_without_policy_return_no_regions(self):
        self.catalog([dict(id='format', extensions=['.bin'])])
        self.assertEqual(self.regions(self.prefix + self.chunk()), [])
        (self.reference / 'format-index.json').unlink()
        self.assertEqual(self.regions(self.prefix + self.chunk()), [])

    def test_invalid_catalog_is_reported_instead_of_disabling_validation_silently(self):
        (self.reference / 'format-index.json').write_text('{broken', encoding='utf-8')
        with self.assertRaisesRegex(ValueError, 'Invalid container verification catalog'):
            self.regions(self.prefix)

    def test_catalog_symlink_cannot_load_policy_outside_skill_root(self):
        outside = self.root / 'outside.json'
        outside.write_text(json.dumps([dict(verification_policy=self.policy)]), encoding='utf-8')
        catalog = self.reference / 'format-index.json'
        catalog.unlink()
        try:
            catalog.symlink_to(outside)
        except (OSError, NotImplementedError) as error:
            self.skipTest('File symlinks unavailable: ' + str(error))
        with self.assertRaisesRegex(ValueError, 'outside the skill root'):
            self.regions(self.prefix)

    def test_shipped_catalog_routes_and_declared_container_policies_remain_compatible(self):
        repository = pathlib.Path(__file__).resolve().parents[4]
        expected = dict(header=dict(offset=16, ascii='Cotopha Image file'),
                        chunks=dict(offset=64, tag_width=8, length_width=8, endian='little',
                                    tag_ascii='conststr', max_chunks=64))
        prefix = bytearray(64)
        prefix[16:34] = b'Cotopha Image file'
        opaque = b'\x00\xff\x80opaque'
        target = b'conststr' + len(opaque).to_bytes(8, 'little') + opaque
        image = b'image   ' + (0).to_bytes(8, 'little')
        shipped = ['Galgear-GalgameReverse-Skill/GalgameResourceExtract']
        if (repository / 'Galgear-GalgameReverse-Skill/EN/GalgameResourceExtract').is_dir():
            shipped.append('Galgear-GalgameReverse-Skill/EN/GalgameResourceExtract')
        for relative in shipped:
            with self.subTest(skill=relative):
                skill = repository / relative
                catalog_path = workflow.format_catalog_path(skill)
                self.assertIsNotNone(catalog_path)
                catalog = json.loads(catalog_path.read_text('utf-8-sig'))
                entry = next(item for item in catalog if item['id'] == 'entis-cotopha'
                             or 'entis-cotopha' in item.get('aliases', []))
                hints = workflow.reference_hints(skill, [dict(path='fixture.csx')])
                self.assertIn(entry['reference'], [item['path'] for item in hints])
                if 'verification_policy' in entry:
                    self.assertEqual(entry['verification_policy'], expected)
                    self.assertEqual(workflow.required_table_regions(skill, bytes(prefix) + target),
                                     [dict(offset=80, length=len(opaque))])
                else:
                    # The standalone latest skill supplies reference routes,
                    # not host-specific policies; do not silently import an old catalog.
                    self.assertEqual(catalog_path.relative_to(skill).as_posix(), 'catalog/formats.json')
                    self.assertEqual(workflow.required_table_regions(skill, bytes(prefix) + target), [])
                self.assertEqual(workflow.required_table_regions(skill, bytes(prefix) + image), [])


if __name__ == '__main__':
    unittest.main()
