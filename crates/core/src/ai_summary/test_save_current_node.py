"""Synthetic current-caption and counted-record proofs; no game/model code."""
import copy
import pathlib
import runpy
import unittest


checks = runpy.run_path(str(pathlib.Path(__file__).with_name('evidence.py')))


def u32(value):
    return value.to_bytes(4, 'little')


def fixture(caption, records):
    encoded = caption.encode('utf-16-le')
    data = b'NXSC' + u32(7) + b'\0' * 12 + u32(len(encoded) // 2) + encoded
    proofs = []
    dialogue = []
    for source, position, text in records:
        name, raw = source.encode('utf-16-le'), text.encode('utf-16-le')
        start = len(data)
        data += u32(len(name) // 2) + name + u32(position) + u32(len(raw) // 2) + raw
        position_offset = start + 4 + len(name)
        proofs.append(dict(source=dict(offset=start + 4, length=len(name), encoding='utf-16-le'),
                           position=dict(offset=position_offset, width=4, endian='little'),
                           anchor=dict(offset=position_offset + 8, length=len(raw), encoding='utf-16-le')))
        dialogue.append(dict(source='translated/' + source + '.lng', position=position, text=text))
    return data, dialogue, proofs


class SaveCurrentNodeTests(unittest.TestCase):
    def validate(self, data, dialogue, proofs, chosen=0, **extra):
        row = dialogue[chosen]
        facts = dict(source=row['source'], line=row['position'], anchors=[row['text']],
                     evidence=copy.deepcopy(proofs[chosen]), **extra)
        return checks['save_facts'](data, facts, dialogue)

    def test_current_record_can_precede_a_unique_old_record_at_the_file_tail(self):
        data, rows, proofs = fixture('Current', [('route_7', 12, 'Current'), ('route_7', 9, 'Older')])
        self.assertEqual(self.validate(data, rows, proofs)['line'], 12)
        with self.assertRaisesRegex(ValueError, 'native current-caption header'):
            self.validate(data, rows, proofs, 1)

    def test_reordering_history_does_not_change_the_current_caption_node(self):
        for records, chosen in (([('route_7', 9, 'Older'), ('route_7', 12, 'Current')], 1),
                                ([('route_7', 12, 'Current'), ('route_7', 9, 'Older')], 0)):
            with self.subTest(chosen=chosen):
                data, rows, proofs = fixture('Current', records)
                self.assertEqual(self.validate(data, rows, proofs, chosen)['line'], 12)

    def test_ascii_native_position_bytes_do_not_change_the_counted_source_boundary(self):
        for position in (31, 32, 48, 65, 126, 127, 135):
            with self.subTest(position=position):
                data, rows, proofs = fixture('Current', [('route_7', position, 'Current')])
                self.assertEqual(self.validate(data, rows, proofs)['line'], position)
        data, rows, proofs = fixture('Current', [('route_7', 65, 'Current')])
        proofs[0]['source']['length'] += 2
        with self.assertRaises(ValueError):
            self.validate(data, rows, proofs)

    def test_renderer_projection_is_supported_without_changing_raw_anchor(self):
        text = ' First%K\\nSecond%P '
        data, rows, proofs = fixture('First\nSecond', [('route_7', 4, text)])
        self.assertEqual(self.validate(data, rows, proofs)['anchors'], [text])

    def test_current_caption_ambiguity_is_not_resolved_by_native_or_physical_order(self):
        data, rows, proofs = fixture('Current', [('route_7', 4, 'Current'), ('other_2', 9, 'Current%P')])
        with self.assertRaisesRegex(ValueError, 'multiple or conflicting'):
            self.validate(data, rows, proofs)

    def test_same_visible_caption_at_multiple_positions_is_ambiguous(self):
        data, rows, proofs = fixture('Current', [('route_7', 4, 'Current'), ('route_7', 9, ' Current%K ')])
        with self.assertRaisesRegex(ValueError, 'multiple or conflicting'):
            self.validate(data, rows, proofs)

    def test_unique_text_still_requires_both_native_source_and_position_proofs(self):
        raw = b'Current|route|' + u32(12)
        row = dict(source='route.lng', position=12, text='Current')
        proof = dict(anchor=dict(offset=0, length=7, encoding='ascii'),
                     source=dict(offset=8, length=5, encoding='ascii'), position=dict(offset=14, width=4))
        for missing in ('source', 'position'):
            with self.subTest(missing=missing):
                bad = copy.deepcopy(proof)
                bad.pop(missing)
                with self.assertRaises(ValueError):
                    checks['save_facts'](raw, dict(source='route.lng', line=12, anchors=['Current'], evidence=bad), [row])
        self.assertEqual(checks['save_facts'](raw, dict(source='route.lng', line=12, anchors=['Current'], evidence=proof), [row])['line'], 12)

    def test_wrong_source_or_position_is_rejected_even_for_unique_anchor(self):
        data, rows, proofs = fixture('Current', [('route_7', 12, 'Current')])
        for patch in (dict(source='other.lng'), dict(line=9)):
            facts = dict(source=rows[0]['source'], line=12, anchors=['Current'], evidence=copy.deepcopy(proofs[0]))
            facts.update(patch)
            with self.subTest(patch=patch), self.assertRaises(ValueError):
                checks['save_facts'](data, facts, rows)

    def test_splicing_source_position_or_anchor_from_separate_records_is_rejected(self):
        data, rows, proofs = fixture('Current', [('route_7', 12, 'Current'), ('route_7', 12, 'Current')])
        # Duplicate physical copies may describe the same node, but field proofs
        # must still belong to one complete counted record.
        for field in ('position', 'anchor'):
            bad = copy.deepcopy(proofs)
            bad[0][field] = bad[1][field]
            with self.subTest(field=field), self.assertRaisesRegex(ValueError, 'same|immediately'):
                self.validate(data, rows[:1], bad)

    def test_fake_current_annotation_does_not_override_a_conflicting_native_header(self):
        data, rows, proofs = fixture('Current', [('route_7', 12, 'Current'), ('route_7', 9, 'Older')])
        proofs[1]['current'] = dict(role='current', is_current=True)
        with self.assertRaisesRegex(ValueError, 'native current-caption header'):
            self.validate(data, rows, proofs, 1)

    def test_malformed_or_unsupported_header_cannot_downgrade_to_empty_anchors(self):
        valid, _, _ = fixture('Current', [('route_7', 12, 'Current')])
        samples = [b'NXSC', valid[:20] + u32(100000) + b'\0\0',
                   valid[:8] + b'\1' + valid[9:], valid[:20] + u32(1) + b'\x00\xd8']
        for data in samples:
            with self.subTest(bytes=len(data)), self.assertRaisesRegex(ValueError, 'NXSC'):
                checks['save_facts'](data, dict(anchors=[], warnings=['No node']), [])

    def test_valid_current_caption_cannot_downgrade_to_byte_proved_scene_only(self):
        data, rows, _ = fixture('Current', [('route_7', 12, 'Current')])
        scene = b'Route overview'
        proof = dict(scene=dict(offset=len(data), length=len(scene), encoding='ascii'))
        data += scene
        self.assertEqual(checks['span'](data, proof['scene']), scene.decode('ascii'))
        for extra in ({}, dict(scene=scene.decode('ascii'),
                               warnings=['Scene identified; current node unavailable'], evidence=proof)):
            with self.subTest(scene_proved=bool(extra)), self.assertRaisesRegex(ValueError, 'requires a verified current node'):
                checks['save_facts'](data, dict(anchors=[], **extra), rows)

    def test_unsupported_controls_cannot_be_stripped_to_make_an_old_node_match(self):
        for text in ('Current%Q', 'Current\\x', 'Current\0', '%K%P'):
            with self.subTest(text_hash=checks['hashlib'].sha256(text.encode()).hexdigest()):
                data, rows, proofs = fixture(text, [('route_7', 12, text)])
                with self.assertRaisesRegex(ValueError, 'NXSC'):
                    self.validate(data, rows, proofs)
        data, rows, proofs = fixture('Current', [('route_7', 12, 'Current%Q')])
        with self.assertRaisesRegex(ValueError, 'unsupported renderer'):
            self.validate(data, rows, proofs)

    def test_transformed_or_unbounded_field_proofs_do_not_bypass_raw_policy(self):
        data, rows, proofs = fixture('Current', [('route_7', 12, 'Current')])
        for field in ('source', 'position', 'anchor'):
            bad = copy.deepcopy(proofs)
            bad[0][field]['view'] = [dict(op='slice', offset=0, length=len(data))]
            with self.subTest(field=field), self.assertRaisesRegex(ValueError, 'raw save record'):
                self.validate(data, rows, bad)
        self.assertEqual(checks['SAVE_VERIFICATION_VERSION'], 3)


if __name__ == '__main__':
    unittest.main()
