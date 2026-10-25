"""Retrospective scope requirements must also guard cached accepted chapters."""
import copy
import json
import pathlib
import struct
import tempfile
import unittest
from unittest import mock

from test_speaker_scopes import host, HELPER, PARSER, companion_bytes


class MissingSpeakerScopeTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = pathlib.Path(temporary.name)
        (self.root / 'game.exe').write_bytes(b'fixture')
        self.config = dict(game=str(self.root), save=str(self.root), skill=str(self.root),
                           work=str(self.root / 'work'), language='en-US')
        self.agent = host.Agent(self.config)
        self.agent.read_skill = self.agent.read_contract = self.agent.used_skill = True
        self.agent.dispatch('write_program', dict(kind='script', source=PARSER))
        for name, speaker in [('first', 'Alice'), ('second', 'Yuka'), ('narration', '')]:
            (self.root / (name + '.ws2')).write_bytes(companion_bytes(0, speaker))
            text = ('Message ' + name).encode('ascii')
            (self.root / (name + '.lng')).write_bytes(struct.pack('<II', 1, len(text)) + text)
        batch = self.agent.dispatch('test_program_batch', dict(root='game', path='.', filter='*.lng'))
        self.assertTrue(all('error' not in row for row in batch['files']))
        self.assertEqual(len(self.agent.parts), 3)
        self.assertTrue(all(row['speaker'] is None for row in self.agent.dialogue()))

    def add_first_scope(self):
        self.agent.dispatch('load_script_metadata', dict(root='game', path='first.ws2', key='first-helper',
            source=HELPER, target_script=dict(root='game', path='first.lng')))
        return self.agent.dispatch('test_program', dict(kind='script', root='game', path='first.lng'))

    def test_old_null_chapters_remain_pending_after_another_chapter_gains_names(self):
        self.add_first_scope()
        status = self.agent.preparation_status()
        self.assertEqual(status['speaker_named_rows'], 1)
        self.assertEqual(status['speaker_verification']['unresolved_rows'], 2)
        self.assertTrue(status['speaker_review_required'])
        self.assertEqual(status['pending_speaker_inputs'],
                         sorted(str(self.root / name) for name in ('narration.lng', 'second.lng')))
        self.assertEqual(status['speaker_verification']['scope_version'], 0)
        self.assertFalse(status['ready_for_finish'])
        with self.assertRaisesRegex(ValueError, 'before finishing'):
            self.agent.dispatch('finish', dict(speaker_resolution='external', limitations=[]))

    def test_unchanged_old_cache_cannot_bypass_new_own_scope_requirement(self):
        self.add_first_scope()
        key = ('script', str(self.root / 'second.lng'))
        self.assertIn(key, self.agent.program_test_cache)
        with mock.patch.object(host, 'run_parser', side_effect=AssertionError('Must reject before parsing')) as parser:
            with self.assertRaisesRegex(ValueError, 'matching original-script'):
                self.agent.dispatch('test_program', dict(kind='script', root='game', path='second.lng'))
        parser.assert_not_called()
        failure = self.agent.last_script_failure
        self.assertEqual(failure['category'], 'speaker_metadata')
        self.assertEqual(failure['path'], 'second.lng')

    def test_missing_scope_routes_to_real_companion_and_saved_working_helper(self):
        self.add_first_scope()
        guidance = self.agent.work_guidance()
        self.assertEqual(guidance['stage'], 'speaker_review')
        self.assertIn('repair_missing_scope', guidance['goal'])
        progress = self.agent.speaker_metadata_progress()
        self.assertEqual(progress['next_step'], 'repair_missing_scope')
        self.assertEqual(progress['sample']['path'], 'narration.lng')
        repair = progress['missing_scope_repair']
        self.assertEqual(repair['reusable_helpers'][0]['helper_key'], 'first-helper')
        self.assertEqual(repair['companion_candidates'][0]['path'], 'narration.ws2')
        self.assertEqual(repair['companion_candidates'][0]['root'], 'game')
        source = next(iter(self.agent.helper_sources().values()))
        self.assertEqual(source['source'], HELPER)
        self.assertIn('first-helper', source['helper_keys'])

    def test_unobserved_same_stem_path_is_not_invented_as_a_repair_candidate(self):
        self.add_first_scope()
        (self.root / 'narration.ws2').unlink()
        progress = self.agent.speaker_metadata_progress()
        self.assertEqual(progress['next_step'], 'repair_missing_scope')
        self.assertEqual(progress['missing_scope_repair']['companion_candidates'], [])
        self.assertTrue(progress['missing_scope_repair']['reusable_helpers'])

    def test_changed_helper_or_companion_is_never_suggested_as_verified_reuse(self):
        self.add_first_scope()
        helper = self.agent.metadata_helpers['first-helper']
        path = self.agent.path('work', helper['path'])
        original = path.read_bytes()
        path.write_bytes(original + b'\n# changed\n')
        target = self.root / 'second.lng'
        self.assertEqual(self.agent.missing_scope_repair(target),
                         dict(reusable_helpers=[], companion_candidates=[]))
        path.write_bytes(original)
        (self.root / 'first.ws2').write_bytes(companion_bytes(0, 'Changed'))
        self.assertEqual(self.agent.missing_scope_repair(target),
                         dict(reusable_helpers=[], companion_candidates=[]))

    def test_restore_keeps_missing_targets_and_recovers_without_rewriting_successful_parser(self):
        self.add_first_scope()
        self.agent.checkpoint([])
        saved = json.loads((self.agent.work / 'checkpoint.json').read_text('utf-8'))
        restored = host.Agent(self.config)
        restored.programs['script'] = PARSER
        self.assertTrue(restored.restore_checkpoint(saved))
        self.assertEqual(restored.missing_speaker_scope_targets(), self.agent.missing_speaker_scope_targets())
        # The per-process result cache is intentionally rebuilt after restart.
        restored.dispatch('test_program', dict(kind='script', root='game', path='first.lng'))
        accepted = copy.deepcopy(restored.parts[str(self.root / 'first.lng')])
        result = restored.dispatch('load_script_metadata_batch', dict(root='game', path='.', filter='*.ws2',
            helper_key='first-helper', target_root='game', target_path='.', target_extension='.lng'))
        self.assertEqual(result['loaded'], 3)
        self.assertEqual(restored.parts[str(self.root / 'first.lng')], accepted)
        self.assertTrue(restored.dispatch('test_program', dict(kind='script', root='game', path='first.lng'))['reused'])
        batch = restored.dispatch('test_program_batch', dict(root='game', path='.', filter='*.lng'))
        self.assertTrue(all('error' not in item for item in batch['files']))
        status = restored.preparation_status()
        self.assertFalse(status['speaker_review_required'])
        self.assertEqual(status['pending_speaker_inputs'], [])
        self.assertEqual(status['speaker_verification']['scope_version'], 2)
        self.assertEqual({pathlib.Path(path).stem: rows[0]['speaker'] for path, rows in restored.parts.items()},
                         dict(first='Alice', second='Yuka', narration=None))
        self.assertEqual(restored.programs['script'], PARSER)

    def test_direct_span_parser_cannot_avoid_the_scope_guard(self):
        self.add_first_scope()
        source = """def extract(data, metadata):
    return [{'position':8,'text':data[8:].decode('ascii'),'speaker':None,
             'evidence':{'offset':8,'length':len(data)-8,'encoding':'ascii'}}]
"""
        self.agent.dispatch('write_program', dict(kind='script', source=source))
        with self.assertRaisesRegex(ValueError, 'matching original-script'):
            self.agent.dispatch('test_program', dict(kind='script', root='game', path='second.lng'))

    def test_scope_requirement_does_not_expand_to_unrelated_file_suffixes(self):
        other = self.root / 'other.txt'
        other.write_bytes((self.root / 'second.lng').read_bytes())
        self.agent.dispatch('test_program', dict(kind='script', root='game', path='other.txt'))
        self.add_first_scope()
        self.assertNotIn(str(other), self.agent.missing_speaker_scope_targets())
        self.assertTrue(self.agent.dispatch('test_program', dict(kind='script', root='game', path='other.txt'))['reused'])


if __name__ == '__main__':
    unittest.main()
