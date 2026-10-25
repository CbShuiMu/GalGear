"""Synthetic save-join diagnostics and inspection metadata parity; no game data."""
import copy
import importlib.util
import json
import pathlib
import struct
import tempfile
import unittest
from unittest import mock


MODULE_DIR = pathlib.Path(__file__).parent
spec = importlib.util.spec_from_file_location('save_feedback_host', MODULE_DIR / 'agent_host.py')
host = importlib.util.module_from_spec(spec)
spec.loader.exec_module(host)


class SaveNodeFeedbackTests(unittest.TestCase):
    def fixture(self):
        native_source = 'fixture_chapter'
        caption = 'PRIVATE_SYNTHETIC_CAPTION'
        canonical = 'unpacked/translated/fixture_chapter.lng'
        position = 9137
        source_bytes, anchor_bytes = native_source.encode('ascii'), caption.encode('ascii')
        anchor_offset = len(source_bytes) + 1
        position_offset = anchor_offset + len(anchor_bytes)
        data = source_bytes + b'\0' + anchor_bytes + struct.pack('<I', position)
        facts = dict(source=canonical, line=position, anchors=[caption], warnings=[],
            evidence=dict(source=dict(offset=0, length=len(source_bytes), encoding='ascii'),
                          anchor=dict(offset=anchor_offset, length=len(anchor_bytes), encoding='ascii'),
                          position=dict(offset=position_offset, width=4, endian='little')))
        rows = [dict(source=canonical, position=position, text=caption, speaker=None)]
        return data, facts, rows, native_source

    def diagnostic(self, data, facts, rows):
        with self.assertRaises(ValueError) as caught:
            host.EVIDENCE['save_facts'](data, copy.deepcopy(facts), rows)
        message = str(caught.exception)
        prefix = 'Current save node is ambiguous or disagrees with independent byte evidence; '
        self.assertTrue(message.startswith(prefix))
        value = json.loads(message[len(prefix):])
        self.assertEqual(value['category'], 'save_node_join')
        self.assertLess(len(message.encode('utf-8')), 2048)
        for row in rows:
            self.assertNotIn(row['source'], message)
            self.assertNotIn(row['text'], message)
            self.assertNotIn(str(row['position']), message)
        return value

    def test_native_source_alias_proof_still_accepts_exact_verified_identity(self):
        data, facts, rows, native_source = self.fixture()
        self.assertNotEqual(native_source, facts['source'])
        verified = host.EVIDENCE['save_facts'](data, copy.deepcopy(facts), rows)
        self.assertEqual(verified['source'], facts['source'])
        self.assertEqual(verified['line'], facts['line'])
        self.assertEqual(verified['anchors'], facts['anchors'])
        self.assertEqual(verified['history'], [])

    def test_bare_alias_rejection_reports_source_field_and_counts_without_choosing(self):
        data, facts, rows, native_source = self.fixture()
        facts['source'] = native_source
        value = self.diagnostic(data, facts, rows)
        self.assertEqual(value['reason'], 'source_identity_absent')
        self.assertEqual(value['fields'], ['facts.source'])
        self.assertEqual(value['counts'], dict(exact=0, source=0, position=1, anchor=1,
            source_position=0, source_anchor=0, anchor_position=1, native_source_alias=1))
        self.assertFalse(value['facts_source_present'])
        self.assertTrue(value['native_source_alias_present'])
        self.assertIn('verbatim', value['instruction'])
        self.assertIn('does not select a candidate', value['instruction'])

    def test_duplicate_exact_nodes_remain_rejected_as_ambiguous(self):
        data, facts, rows, _ = self.fixture()
        rows += copy.deepcopy(rows)
        value = self.diagnostic(data, facts, rows)
        self.assertEqual(value['reason'], 'ambiguous_exact_node')
        self.assertEqual(value['counts']['exact'], 2)
        self.assertTrue(value['facts_source_present'])

    def test_proven_position_missing_from_verified_source_reports_pair_mismatch(self):
        data, facts, rows, _ = self.fixture()
        rows[0]['position'] += 1
        value = self.diagnostic(data, facts, rows)
        self.assertEqual(value['reason'], 'source_position_absent')
        self.assertEqual(value['fields'], ['facts.source', 'facts.line'])
        self.assertEqual(value['counts']['source'], 1)
        self.assertEqual(value['counts']['position'], 0)
        self.assertEqual(value['counts']['source_position'], 0)
        self.assertEqual(value['counts']['anchor'], 1)

    def test_proven_anchor_different_from_verified_source_position_remains_rejected(self):
        data, facts, rows, _ = self.fixture()
        rows[0]['text'] = 'ANOTHER_PRIVATE_FIXTURE_CAPTION'
        value = self.diagnostic(data, facts, rows)
        self.assertEqual(value['reason'], 'anchor_mismatch_for_source_position')
        self.assertEqual(value['fields'], ['facts.anchors[0]'])
        self.assertEqual(value['counts']['source_position'], 1)
        self.assertEqual(value['counts']['anchor'], 0)
        self.assertEqual(value['counts']['exact'], 0)

    def test_native_position_proof_disagreement_still_fails_before_dialogue_join(self):
        data, facts, rows, _ = self.fixture()
        facts['line'] += 1
        with self.assertRaisesRegex(ValueError, 'position byte evidence disagrees') as caught:
            host.EVIDENCE['save_facts'](data, facts, rows)
        self.assertNotIn('save_node_join', str(caught.exception))


class SaveInspectionMetadataTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = pathlib.Path(temporary.name)
        (self.root / 'fixture.exe').write_bytes(b'fixture')
        self.save = self.root / 'saves'
        self.save.mkdir()
        (self.save / 'current.sav').write_bytes(b'synthetic save bytes')
        (self.root / 'chapter.lng').write_bytes(b'synthetic script bytes')
        self.agent = host.Agent(dict(game=str(self.root), save=str(self.save), skill=str(self.root),
            work=str(self.root / 'work'), provider='offline-fixture', model='offline-fixture', language='en-US'))
        self.rows = [dict(source='canonical/chapter.lng', position=9137, text='Fixture caption', speaker=None)]
        self.agent.parts[str(self.root / 'chapter.lng')] = copy.deepcopy(self.rows)
        self.agent.script_resources = dict(names=dict(value=1), scoped=dict(value=2))
        self.agent.speaker_resources = dict(names={'entry':dict(id=1,text='Fixture name')},
                                           scoped={'entry':dict(id=2,text=None)})
        self.agent.speaker_resource_targets['scoped'] = str(self.root / 'chapter.lng')
        self.agent.programs['save'] = 'def extract(data, metadata): return {}'

    def test_save_inspection_receives_the_same_verified_dialogue_as_save_test(self):
        captured = []
        class MetadataCaptured(RuntimeError):
            pass
        def run(source, data, metadata, verify_save=False):
            captured.append(dict(metadata=copy.deepcopy(metadata), verify_save=verify_save))
            if verify_save:
                raise MetadataCaptured('Synthetic metadata capture only')
            return dict(row_count=len(metadata.get('dialogue', [])))
        before = copy.deepcopy(self.agent.parts)
        # Isolate this metadata boundary from script-policy migration, which is
        # exercised separately with real parser/helper verification.
        with mock.patch.object(self.agent, 'dialogue', return_value=self.rows), \
                mock.patch.object(host, 'run_parser', side_effect=run):
            result = self.agent._dispatch('inspect_data', dict(root='save',path='current.sav',
                source='def extract(data, metadata): return {}'))
            self.assertEqual(self.agent.parts, before)
            with self.assertRaises(MetadataCaptured):
                self.agent._dispatch('test_program', dict(kind='save',root='save',path='current.sav'))
        self.assertEqual(result, dict(row_count=1))
        self.assertEqual([entry['verify_save'] for entry in captured], [False, True])
        self.assertEqual(captured[0]['metadata']['dialogue'], self.rows)
        self.assertEqual(captured[0]['metadata'], captured[1]['metadata'])
        self.assertEqual(self.agent.tests, {})

    def test_nonsave_inspection_keeps_existing_helper_scope_without_dialogue(self):
        args = dict(root='game',path='chapter.lng',source='def extract(data, metadata): return {}',
                    target_script=dict(root='game',path='chapter.lng'))
        expected = self.agent.metadata_input(self.root/'chapter.lng',self.root/'chapter.lng')
        with mock.patch.object(host,'run_parser',return_value=dict(ok=True)) as run:
            self.agent._dispatch('inspect_data',args)
        actual = run.call_args.args[2]
        self.assertEqual(actual,expected)
        self.assertNotIn('dialogue',actual)
        self.assertEqual(set(actual['speaker_resources']),{'names','scoped'})

    def test_tool_description_explains_save_only_parity_without_new_arguments(self):
        tool = next(entry['function'] for entry in host.TOOLS if entry['function']['name']=='inspect_data')
        self.assertIn('For root="save", metadata["dialogue"]',tool['description'])
        self.assertEqual(set(tool['parameters']['properties']),{'root','path','source','target_script'})
        self.assertEqual(tool['parameters']['required'],['root','path','source'])


if __name__ == '__main__':
    unittest.main()
