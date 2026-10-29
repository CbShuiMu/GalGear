"""Cold-run isolation checks using synthetic files, without model requests."""
import importlib.util
import json
import os
from pathlib import Path
import runpy
import struct
import tempfile
import unittest
from unittest import mock
import zlib


spec = importlib.util.spec_from_file_location('cold_runner', Path(__file__).with_name('validate-ai-cold.py'))
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)
REAL_REPO = runner.REPO


def native_fixture(path, data, member='chapter.bin'):
    """A complete independent uncompressed synthetic XP3 member."""
    chunk = lambda tag, value: tag + struct.pack('<Q', len(value)) + value
    encoded = member.encode('utf-16-le')
    info = struct.pack('<IQQH', 0, len(data), len(data), len(encoded)//2) + encoded
    segment = struct.pack('<IQQQ', 0, 19, len(data), len(data))
    record = chunk(b'File', chunk(b'info', info) + chunk(b'segm', segment)
                   + chunk(b'adlr', struct.pack('<I', zlib.adler32(data) & 0xffffffff)))
    path.write_bytes(b'XP3\r\n \n\x1a\x8bg\x01' + struct.pack('<Q', 19 + len(data))
                     + data + b'\0' + struct.pack('<Q', len(record)) + record)


class ColdIsolationTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.game, self.saves = self.root / 'game', self.root / 'saves'
        self.game.mkdir()
        self.saves.mkdir()
        (self.game / 'chapter.ks').write_text('*fixture\nSynthetic dialogue', encoding='utf-8')
        self.slot = self.saves / 'slot.ksd'
        self.slot.write_bytes(b'synthetic native slot')
        host = self.root / 'crates/core/src/ai_summary/agent_host.py'
        host.parent.mkdir(parents=True)
        host.write_text('# synthetic host', encoding='utf-8')
        skill = self.root / 'Galgear-GalgameReverse-Skill/GalgameResourceExtract'
        skill.mkdir(parents=True)
        (skill / 'SKILL.md').write_text('Synthetic portable skill', encoding='utf-8')
        self.settings = self.root / 'settings.json'
        runner.write_json(self.settings, dict(language='en-US', ai_models={'deepseek': 'configured-model'}))
        self.inputs = self.root / 'inputs.json'
        runner.write_json(self.inputs, dict(game=str(self.game), title='Synthetic game', language='en-US',
            save_inputs=dict(save=str(self.saves), required_saves=[str(self.slot)])))
        patcher = mock.patch.object(runner, 'REPO', self.root)
        patcher.start()
        self.addCleanup(patcher.stop)

    def plan(self, output='run'):
        return runner.prepare_plan(self.inputs, self.settings, self.root / output, 2700, 150)

    def test_each_plan_has_a_new_empty_work_identity(self):
        _, first = self.plan('first')
        _, second = self.plan('second')
        self.assertNotEqual(first['config']['work'], second['config']['work'])
        self.assertFalse(Path(first['config']['work']).exists())
        self.assertTrue(first['config']['cold_start'])

    def test_existing_run_is_rejected_instead_of_resumed(self):
        self.plan()
        with self.assertRaisesRegex(ValueError, 'every test requires a new'):
            self.plan()

    def test_required_target_is_used_exactly_as_selected_by_production(self):
        _, plan = self.plan()
        self.assertEqual(plan['config']['required_saves'], [str(self.slot)])

    def test_unknown_candidates_remain_unforced_for_model_classification(self):
        inputs = runner.read_json(self.inputs)
        inputs['save_inputs']['required_saves'] = []
        inputs['save_inputs']['candidates'] = [dict(path=str(self.slot), kind='unknown')]
        runner.write_json(self.inputs, inputs)
        _, plan = self.plan()
        self.assertEqual(plan['config']['required_saves'], [])
        self.assertEqual(plan['config']['save_candidates'], inputs['save_inputs']['candidates'])

    def test_outside_required_path_is_rejected(self):
        inputs = runner.read_json(self.inputs)
        inputs['save_inputs']['required_saves'] = [str(self.game / 'chapter.ks')]
        runner.write_json(self.inputs, inputs)
        with self.assertRaisesRegex(ValueError, 'outside the selected'):
            self.plan()

    def test_prior_preparation_is_excluded_from_model_inventory(self):
        previous = self.game / '.galgear-ai-summary/previous'
        previous.mkdir(parents=True)
        (previous / 'save_extractor.py').write_text('prior model answer', encoding='utf-8')
        (previous / 'dialogue.jsonl').write_text('prior transcript', encoding='utf-8')
        _, plan = self.plan()
        self.assertEqual([Path(row['path']).name for row in plan['config']['inventory']], ['chapter.ks'])
        self.assertFalse(plan['seed_artifacts_copied'])

    def test_modified_input_is_detected_even_when_length_is_unchanged(self):
        _, plan = self.plan()
        self.slot.write_bytes(b'Synthetic native slot')
        self.assertFalse(runner.unchanged(plan['original_inputs'])['unchanged'])

    def test_planning_does_not_read_or_persist_credentials(self):
        with mock.patch.object(runner, 'credential', side_effect=AssertionError('vault read')):
            output, plan = self.plan()
        self.assertNotIn('token', plan['config'])
        self.assertNotIn('token', json.loads((output / 'plan.json').read_text('utf-8')))

    def test_independent_audit_distinguishes_denied_and_successful_prior_cache_reads(self):
        _, plan = self.plan()
        event = dict(tool='read_text', arguments=dict(root='game', path='.galgear-ai-summary/old/parser.py'))
        self.assertFalse(runner.cache_read_audit([event], plan['config'])['passed'])
        event['error'] = 'Cold preparation cannot read another preparation cache'
        self.assertTrue(runner.cache_read_audit([event], plan['config'])['passed'])

    def test_reading_current_run_outputs_is_allowed(self):
        _, plan = self.plan()
        event = dict(tool='inspect_structured', arguments=dict(root='work', path='fresh.psb'))
        self.assertTrue(runner.cache_read_audit([event], plan['config'])['passed'])

    def test_new_runtime_module_invalidates_a_frozen_run(self):
        _, plan = self.plan()
        self.assertTrue(runner.bindings(plan)['unchanged'])
        (Path(plan['host']).parent / 'new_verifier.py').write_text('# new production module', encoding='utf-8')
        self.assertFalse(runner.bindings(plan)['runtime']['membership_matches'])

    def test_new_skill_document_invalidates_a_frozen_run(self):
        _, plan = self.plan()
        (Path(plan['config']['skill']) / 'new-document.md').write_text('new protocol', encoding='utf-8')
        self.assertFalse(runner.bindings(plan)['skill']['membership_matches'])

    def test_vault_identity_preserves_logical_settings_path_under_file_virtualization(self):
        original = Path.resolve
        physical = self.root / 'package-localcache/settings.json'
        def redirected(path, *args, **kwargs):
            return physical if path == self.settings else original(path, *args, **kwargs)
        with mock.patch.object(Path, 'resolve', redirected):
            _, plan = self.plan()
        self.assertEqual(plan['settings_path'], os.path.abspath(self.settings))
        self.assertNotEqual(plan['settings_path'], str(physical))

    def native_context(self):
        archives = runpy.run_path(str(REAL_REPO / 'crates/core/src/ai_summary/archive_evidence.py'))
        work = self.root / 'fresh-work'
        folder = work / 'unpacked'
        folder.mkdir(parents=True)
        data = b'Independent synthetic native payload'
        archive = self.game / 'resources.xp3'
        native_fixture(archive, data)
        path = folder / 'chapter.bin'
        path.write_bytes(data)
        checkpoint = dict(extracted_from={str(folder): str(archive)},
                          native_archive=dict(adler32=0, scope='forged'))
        return dict(ARCHIVES=archives), checkpoint, work, path, data

    def test_audit_native_metadata_comes_from_complete_archive_not_checkpoint_claims(self):
        host, checkpoint, work, path, data = self.native_context()
        native = runner.native_archive_metadata(host, checkpoint, work, self.game, path, data)
        self.assertEqual(native['adler32'], zlib.adler32(data) & 0xffffffff)
        self.assertEqual(native['size'], len(data))
        self.assertEqual(native['scope'], 'native_declaration_only')
        self.assertNotIn('verified', native)

    def test_audit_omits_native_metadata_when_complete_raw_identity_does_not_bind(self):
        host, checkpoint, work, path, data = self.native_context()
        self.assertIsNone(runner.native_archive_metadata(
            host, checkpoint, work, self.game, path, data[:-1] + b'!'))

    def test_audit_rejects_native_metadata_from_another_game(self):
        host, checkpoint, work, path, data = self.native_context()
        other = self.root / 'other.xp3'
        native_fixture(other, data)
        checkpoint['extracted_from'] = {str(path.parent): str(other)}
        with self.assertRaisesRegex(ValueError, 'escaped this game/run'):
            runner.native_archive_metadata(host, checkpoint, work, self.game, path, data)

    def document_context(self):
        host, checkpoint, work, path, _ = self.native_context()
        codecs = runpy.run_path(str(REAL_REPO / 'crates/core/src/ai_summary/bytecodecs.py'))
        host['EVIDENCE'] = dict(view=codecs['view'])
        inner = b'PSB\0Synthetic complete outer-boundary test payload'
        data = b'mdf\0' + len(inner).to_bytes(4, 'little') + zlib.compress(inner)
        path.write_bytes(data)
        archive = Path(next(iter(checkpoint['extracted_from'].values())))
        native_fixture(archive, data)
        output = dict(rows=[dict(evidence=dict(kind='typed_field',
            document=dict(syntax='psb', frame=dict(codec='mdf')),
            path=['text'], type='string'))])
        return host, checkpoint, work, path, data, archive, inner, output

    def test_audit_checks_outer_cipher_before_internal_document_not_inner_plaintext(self):
        host, checkpoint, work, path, data, _, inner, output = self.document_context()
        self.assertNotEqual(zlib.adler32(data), zlib.adler32(inner))
        self.assertEqual(runner.replay_member_boundary(
            host, checkpoint, work, self.game, path, data, output), data)

    def test_audit_document_description_cannot_skip_wrong_outer_checksum(self):
        host, checkpoint, work, path, data, archive, inner, output = self.document_context()
        # Only the native adlr is changed; complete raw identity and sizes still bind.
        archive.write_bytes(archive.read_bytes()[:-4] + struct.pack('<I', zlib.adler32(inner)))
        with self.assertRaisesRegex(ValueError, 'whole restored member'):
            runner.replay_member_boundary(host, checkpoint, work, self.game, path, data, output)

    def test_audit_top_level_rows_keep_the_same_outer_member_boundary(self):
        host, checkpoint, work, path, data, _, _, output = self.document_context()
        self.assertEqual(runner.replay_member_boundary(
            host, checkpoint, work, self.game, path, data, output['rows']), data)

    def test_audit_top_level_rows_cannot_skip_wrong_outer_checksum(self):
        host, checkpoint, work, path, data, archive, inner, output = self.document_context()
        archive.write_bytes(archive.read_bytes()[:-4] + struct.pack('<I', zlib.adler32(inner)))
        with self.assertRaisesRegex(ValueError, 'whole restored member'):
            runner.replay_member_boundary(host, checkpoint, work, self.game, path, data, output['rows'])

    def test_audit_loose_document_replays_complete_native_frame(self):
        host, _, work, _, data, _, _, output = self.document_context()
        path = self.game / 'loose.mdf'
        path.write_bytes(data)
        self.assertEqual(runner.replay_member_boundary(
            host, {}, work, self.game, path, data, output), data)
        damaged = data + b'trailing'
        path.write_bytes(damaged)
        with self.assertRaises(ValueError):
            runner.replay_member_boundary(host, {}, work, self.game, path, damaged, output)


if __name__ == '__main__':
    unittest.main()
