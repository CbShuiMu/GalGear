"""Adversarial regression tests for format-neutral verification, without model calls."""
import copy
import importlib.util
import json
import pathlib
import struct
import tempfile
import unittest
from unittest import mock
from types import SimpleNamespace
import zlib

spec = importlib.util.spec_from_file_location('agent_evidence_tests', pathlib.Path(__file__).with_name('agent_host.py'))
host = importlib.util.module_from_spec(spec)
spec.loader.exec_module(host)
checks = host.EVIDENCE


def span(offset, text):
    return dict(offset=offset, length=len(text.encode()), encoding='utf-8')


class EvidenceTests(unittest.TestCase):
    def indexed_fixture(self):
        # Empty and unreferenced entries are real ordinals, not removable padding.
        texts = ['', 'Before', 'Current', 'After']
        code = struct.pack('<I', 2)
        pool = struct.pack('<I', len(texts))
        for i, text in enumerate(texts):
            pool += struct.pack('<I', len(text)) + text.encode()
            refs = [0] if i == 2 else []
            pool += struct.pack('<I', len(refs)) + b''.join(struct.pack('<I', ref) for ref in refs)
        schema = dict(offset=len(code), length=len(pool), fields=[dict(kind='text', encoding='utf-8'), dict(kind='skip', unit=4)])
        row = dict(position=0, text='Current', speaker=None,
                   evidence=dict(table='strings', index=2, reference=dict(base=0, operand_offset=0)))
        return code + pool, dict(rows=[row], tables=dict(strings=schema))

    def test_correct_ordinal_reference_with_empty_records_is_accepted(self):
        data, output = self.indexed_fixture()
        self.assertEqual(checks['script_rows'](data, output)[0]['text'], 'Current')

    def test_next_string_text_cannot_be_attached_to_previous_instruction(self):
        data, output = self.indexed_fixture()
        output['rows'][0]['text'] = 'After'
        with self.assertRaisesRegex(ValueError, 'actual string index'):
            checks['script_rows'](data, output)
        output['rows'][0]['evidence']['index'] = 3
        with self.assertRaisesRegex(ValueError, 'ordinal'):
            checks['script_rows'](data, output)

    def test_message_index_coordinates_bind_to_verified_table_ordinals(self):
        data, output = self.indexed_fixture()
        output['rows'][0]['position'] = 2
        output['rows'][0]['evidence']['reference'] = dict(mode='ordinal')
        self.assertEqual(checks['script_rows'](data, output)[0]['position'], 2)
        output['rows'][0]['position'] = 1
        with self.assertRaisesRegex(ValueError, 'verified table ordinal'):
            checks['script_rows'](data, output)

    def test_reversed_fields_and_partial_tables_do_not_self_validate(self):
        data, output = self.indexed_fixture()
        for mutation in ('reverse', 'truncate'):
            bad = copy.deepcopy(output)
            if mutation == 'reverse':
                bad['tables']['strings']['fields'].reverse()
            else:
                bad['tables']['strings']['length'] -= 1
            with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                checks['script_rows'](data, bad)

    def test_direct_text_position_and_speaker_require_actual_bytes(self):
        data = b'Alice|Hello'
        row = dict(position=6, text='Hello', speaker='Alice', evidence=dict(span(6, 'Hello'), speaker=span(0, 'Alice')))
        self.assertEqual(checks['script_rows'](data, [row]), [row])
        row['speaker'] = 'Beth'
        with self.assertRaises(ValueError):
            checks['script_rows'](data, [row])
        row['speaker'], row['position'] = None, 5
        with self.assertRaisesRegex(ValueError, 'byte offset'):
            checks['script_rows'](data, [row])

    def test_declared_standard_transforms_are_recomputed(self):
        data = b'Hello'
        packed = b'HEAD' + zlib.compress(bytes(b ^ 7 for b in data))
        steps = [dict(op='slice', offset=4, length=len(packed) - 4), dict(op='zlib'), dict(op='xor', key='07')]
        row = dict(position=0, text='Hello', speaker=None, evidence=span(0, 'Hello'))
        self.assertEqual(checks['script_rows'](packed, dict(rows=[row], view=steps))[0]['text'], 'Hello')
        with self.assertRaises(ValueError):
            checks['view'](packed[4:] + b'trailing', [dict(op='zlib')])

    def test_external_speaker_requires_name_id_bytes_and_native_script_reference(self):
        metadata = dict(values={'7': 'Alice'}, speakers={'alice': dict(id=7, text='Alice',
            evidence=span(2, 'Alice'), id_evidence=dict(offset=0, width=2))})
        values, names = checks['speaker_metadata'](b'\x07\0Alice', metadata)
        self.assertEqual(values, {'7': 'Alice'})
        data = b'\x07\0Hello'
        row = dict(position=2, text='Hello', speaker='Alice', evidence=dict(span(2, 'Hello'),
            speaker=dict(resource='names', entry='alice', reference=dict(offset=0, width=2))))
        self.assertEqual(checks['script_rows'](data, [row], speaker_resources={'names': names}), [row])
        for mutation in ('id', 'name', 'null', 'unverified'):
            bad, resources = copy.deepcopy(row), {'names': names}
            raw = data
            if mutation == 'id': raw = b'\x08\0Hello'
            if mutation == 'name': bad['speaker'] = 'Guessed identity'
            if mutation == 'null': bad['speaker'] = None
            if mutation == 'unverified': resources = {'names': {'alice': 'Alice'}}
            with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                checks['script_rows'](raw, [bad], speaker_resources=resources)
        metadata['speakers']['alice']['id'] = 8
        with self.assertRaisesRegex(ValueError, 'native bytes'):
            checks['speaker_metadata'](b'\x07\0Alice', metadata)

    def test_original_script_native_message_id_binds_translated_table_speaker(self):
        name_file = b'Yuka|Localized name'
        _, names = checks['speaker_metadata'](name_file, dict(values={}, speakers={'yuka': dict(
            id='Yuka', text='Localized name', id_evidence=span(0, 'Yuka'), evidence=span(5, 'Localized name'))}))
        original = b'Yuka' + struct.pack('<I', 2)
        _, messages = checks['speaker_metadata'](original, dict(values={}, speakers={'message-2': dict(
            id=2, text='Localized name', id_evidence=dict(offset=4), evidence=dict(resource='names', entry='yuka', reference=span(0, 'Yuka')))}), {'names': names})
        messages['message-2']['_source'] = 'chapter.lng'
        translated, output = self.indexed_fixture()
        output['rows'][0]['speaker'] = 'Localized name'
        output['rows'][0]['evidence']['speaker'] = dict(resource='original', entry='message-2', reference=dict(mode='table_index'))
        self.assertEqual(checks['script_rows'](translated, output, speaker_resources={'original': messages}, source='chapter.lng')[0]['speaker'], 'Localized name')
        messages['message-2']['id'] = 1
        with self.assertRaisesRegex(ValueError, 'companion ID|Missing speaker/narration proof'):
            checks['script_rows'](translated, output, speaker_resources={'original': messages}, source='chapter.lng')

    def test_offset_table_keeps_duplicate_text_at_distinct_24bit_graph_references(self):
        data = b'\0\0\0\x01\0\0\x02' + b'\0' * 6 + b'Hello\0'
        table = dict(kind='offset_array', offset=7, length=6, count=dict(offset=6, width=1),
                     offset_width=3, base=13, pool_length=6, encoding='utf-8')
        rows = [dict(position=position, text='Hello', speaker=None,
                     evidence=dict(table='pool', index=index, reference=dict(operand_offset=0, width=3)))
                for index, position in enumerate((0, 3))]
        self.assertEqual(checks['script_rows'](data, dict(rows=rows, tables={'pool': table})), rows)
        bad = bytearray(data)
        bad[10:13] = b'\xff\xff\xff'
        with self.assertRaisesRegex(ValueError, 'string pool'):
            checks['script_rows'](bytes(bad), dict(rows=rows, tables={'pool': table}))
        with self.assertRaisesRegex(ValueError, 'NUL terminator'):
            checks['script_rows'](data[:-1] + b'!', dict(rows=rows, tables={'pool': table}))

    def test_separate_length_array_recomputes_xor_pool_and_preserves_empty_ordinals(self):
        texts = ['', 'Hello', 'Current']
        pool = b''.join((text+'\0').encode('utf-16-le') for text in texts)
        key = bytes.fromhex('3412')
        packed = bytes(value ^ key[index % 2] for index, value in enumerate(pool))
        data = struct.pack('<I3H', 3, 2, 12, 16) + packed
        table = dict(kind='length_array', offset=4, length=6, count=dict(offset=0), length_width=2,
                     base=10, pool_length=len(packed), encoding='utf-16-le', terminator_width=2,
                     pool_view=[dict(op='xor', key='3412')])
        row = dict(position=2, text='Current', speaker=None, evidence=dict(table='translated', index=2, reference=dict(mode='ordinal')))
        self.assertEqual(checks['script_rows'](data, dict(rows=[row], tables={'translated': table})), [row])
        bad = copy.deepcopy(table)
        bad['pool_view'][0]['key'] = '3512'
        with self.assertRaisesRegex(ValueError, 'NUL terminator'):
            checks['script_rows'](data, dict(rows=[row], tables={'translated': bad}))
        with self.assertRaisesRegex(ValueError, 'original structure'):
            checks['script_rows'](data, dict(rows=[row], tables={'translated': table}),
                                  required_table_regions=[dict(offset=4, length=6)])

    def test_repeated_captions_need_native_position_evidence(self):
        rows = [dict(source='a', position=10, text='Current'), dict(source='a', position=20, text='Current')]
        data = b'Current' + struct.pack('<I', 20) + b'a'
        facts = dict(source='a', line=10, anchors=['Current'], evidence=dict(anchor=span(0, 'Current')))
        with self.assertRaises(ValueError):
            checks['save_facts'](data, copy.deepcopy(facts), rows)
        facts['evidence']['position'] = dict(offset=7)
        facts['evidence']['source'] = span(11, 'a')
        with self.assertRaises(ValueError):
            checks['save_facts'](data, copy.deepcopy(facts), rows)
        facts['line'] = 20
        self.assertEqual(checks['save_facts'](data, facts, rows)['line'], 20)

    def test_save_history_is_built_from_bytes_not_claimed_by_the_parser(self):
        rows = [dict(source='route-b', position=90, text='Before'), dict(source='route-a', position=2, text='Current')]
        data = b'Before|Current|Chapter'
        source_offset = len(data) + 1
        position_offset = source_offset + len(b'route-a')
        data += b'|route-a' + struct.pack('<I', 2)
        facts = dict(source='route-a', line=2, scene='Chapter', anchors=['Current'],
                     history=[dict(source='fake', position=999)],
                     evidence=dict(anchor=span(7, 'Current'), scene=span(15, 'Chapter'),
                                   source=span(source_offset, 'route-a'), position=dict(offset=position_offset, width=4, endian='little'),
                                   history=[span(0, 'Before'), span(7, 'Current')]))
        result = checks['save_facts'](data, copy.deepcopy(facts), rows)
        self.assertEqual(result['history'], [dict(source='route-b', position=90), dict(source='route-a', position=2)])
        facts['evidence'].pop('history')
        self.assertEqual(checks['save_facts'](data, copy.deepcopy(facts), rows)['history'], [])
        facts['evidence']['history'] = [span(7, 'Current'), span(0, 'Before')]
        with self.assertRaisesRegex(ValueError, 'record order'):
            checks['save_facts'](data, facts, rows)


class SharedPipelineTests(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.root = pathlib.Path(tmp.name)
        (self.root / 'game.exe').write_bytes(b'fixture')
        self.work = self.root / 'work'
        self.agent = host.Agent(dict(game=tmp.name, save=tmp.name, skill=tmp.name, work=str(self.work), language='en-US'))
        self.agent.dispatch('verification_contract', {})

    def test_contract_read_is_enforced_before_program_writes(self):
        self.agent.read_contract = False
        with self.assertRaisesRegex(ValueError, 'Read verification_contract'):
            self.agent.dispatch('write_program', dict(kind='script', source='def extract(data, metadata): return []'))

    def test_windows_checkpoint_replacement_survives_transient_file_locks(self):
        target = self.work / 'lock-test.json'
        target.write_text('{"old":true}', encoding='utf-8')
        replace = pathlib.Path.replace
        calls = []
        def temporarily_locked(path, destination):
            calls.append(1)
            if len(calls) < 3:
                raise PermissionError('Transient scanner lock')
            return replace(path, destination)
        with mock.patch.object(pathlib.Path, 'replace', new=temporarily_locked), mock.patch.object(host, 'os', SimpleNamespace(name='nt')), mock.patch.object(host.time, 'sleep'):
            host.atomic_json(target, {'new': True})
        self.assertEqual(json.loads(target.read_text('utf-8')), {'new': True})
        self.assertEqual(len(calls), 3)

    def test_failed_progress_sample_survives_rewrites_and_cannot_be_relabelled(self):
        (self.root / 'latest.sav').write_bytes(b'Current')
        self.agent.programs['save'] = 'def extract(data, metadata): return {"anchors":[]}'
        with self.assertRaisesRegex(ValueError, 'current.*node|node.*current'):
            self.agent.dispatch('test_program', dict(kind='save', root='save', path='latest.sav'))
        self.agent.dispatch('write_program', dict(kind='save', source='def extract(data, metadata): return {"anchors":[]}'))
        self.assertEqual(self.agent.tests, {})
        self.assertEqual(self.agent.samples[str(self.root / 'latest.sav')], 'progress')
        with self.assertRaisesRegex(ValueError, 'hidden as a control'):
            self.agent.dispatch('test_program', dict(kind='save', root='save', path='latest.sav', sample_type='control'))
        self.agent.checkpoint([])
        saved = json.loads((self.work / 'checkpoint.json').read_text('utf-8'))
        self.assertEqual(saved['samples'][str(self.root / 'latest.sav')], 'progress')

    def test_required_latest_save_cannot_be_replaced_by_two_easier_slots(self):
        from test_agent_host import PreparationLifecycleTests
        fixture = PreparationLifecycleTests()
        fixture.setUp()
        self.addCleanup(fixture.doCleanups)
        fixture.validate_save('one.sav')
        fixture.validate_save('two.sav')
        self.assertTrue(fixture.agent.preparation_status()['ready_for_finish'])
        fixture.agent.samples[str(fixture.saves / 'latest.sav')] = 'progress'
        fixture.agent.required.add(str(fixture.saves / 'latest.sav'))
        self.assertFalse(fixture.agent.preparation_status()['ready_for_finish'])

    def test_duplicate_archive_member_requires_verified_configuration_priority(self):
        for name in ('base.arc', 'patch.arc'):
            (self.root / name).write_bytes(b'fixture')
        (self.root / 'load.xml').write_bytes(b'<archive path="patch.arc"/><archive path="base.arc"/>')
        self.agent.config['inventory'] = [dict(path=str(self.root / n)) for n in ('base.arc', 'patch.arc')]
        listing = dict(exit_code=0, stdout=' [00000000] 10 scene.bin\n')
        with mock.patch.object(self.agent, 'dispatch', return_value=listing):
            args = dict(script_name='scene.bin', selected='base.arc', configuration='load.xml', priority='first')
            with self.assertRaisesRegex(ValueError, 'configuration priority'):
                self.agent.verify_resource_selection(args)
            args['selected'] = 'patch.arc'
            result = self.agent.verify_resource_selection(args)
        self.assertEqual(result['selected'], str(self.root / 'patch.arc'))
        self.assertEqual(len(result['candidates']), 2)

    def test_archive_without_matching_script_is_a_verified_negative(self):
        for name in ('images.arc', 'story.arc'):
            (self.root / name).write_bytes(b'fixture')
        self.agent.config['inventory'] = [dict(path=str(self.root / n)) for n in ('images.arc', 'story.arc')]
        replies = [dict(exit_code=1, stdout='', stderr='Error: No files match the given filter\r\n'),
                   dict(exit_code=0, stdout=' [00000000] 10 scene.bin\n', stderr='')]
        with mock.patch.object(self.agent, 'dispatch', side_effect=replies):
            result = self.agent.verify_resource_selection(dict(script_name='scene.bin', selected='story.arc'))
        self.assertEqual(result['candidates'], [str(self.root / 'story.arc')])

    def test_scene_only_slot_is_visible_but_cannot_satisfy_the_target(self):
        from test_agent_host import PreparationLifecycleTests
        fixture = PreparationLifecycleTests()
        fixture.setUp()
        self.addCleanup(fixture.doCleanups)
        fixture.validate_save('one.sav')
        fixture.validate_save('two.sav')
        slot = fixture.saves / 'scene-only.sav'
        slot.write_bytes(b'Chapter')
        fixture.agent.programs['save'] = 'def extract(data, metadata): return {"anchors":[],"scene":"Chapter","warnings":["No current caption recorded"],"evidence":{"scene":{"offset":0,"length":7,"encoding":"utf-8"}}}'
        fixture.agent.dispatch('test_program', dict(kind='save', root='save', path=slot.name))
        self.assertEqual(fixture.agent.tests[str(slot)]['coverage'], 'scene_only')
        self.assertTrue(fixture.agent.preparation_status()['ready_for_finish'])
        fixture.agent.required.add(str(slot))
        self.assertFalse(fixture.agent.preparation_status()['ready_for_finish'])
        self.assertIn(str(slot), fixture.agent.preparation_status()['pending_targets'])

    def test_companion_file_list_cannot_claim_external_speaker_verification(self):
        from test_agent_host import PreparationLifecycleTests
        fixture = PreparationLifecycleTests()
        fixture.setUp()
        self.addCleanup(fixture.doCleanups)
        fixture.agent.parts[str(fixture.scene)][0]['speaker'] = 'Verified inline name'
        fixture.agent.script_resources = {'listing_only': {'entries': ['name-image.eri']}}
        fixture.validate_save('one.sav')
        fixture.validate_save('two.sav')
        result = fixture.agent.dispatch('finish', dict(fixture.finish, speaker_resolution='mixed'))
        self.assertEqual(result['speaker_resolution'], 'inline')
        self.assertEqual(result['speaker_verification']['kind'], 'inline_bytes')


if __name__ == '__main__':
    unittest.main()
