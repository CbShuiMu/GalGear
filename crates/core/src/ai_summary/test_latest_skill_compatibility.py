"""Offline installed-skill compatibility; no game data, parser hints or model calls."""
import importlib.util
import json
import pathlib
import shutil
import tempfile
import unittest
from unittest import mock


spec = importlib.util.spec_from_file_location('latest_skill_host', pathlib.Path(__file__).with_name('agent_host.py'))
host = importlib.util.module_from_spec(spec)
spec.loader.exec_module(host)
workflow = host.WORKFLOW


class LatestSkillCompatibilityTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = pathlib.Path(temporary.name)
        self.skill = self.root / 'skill'
        self.skill.mkdir()
        self.game = self.root / 'game'
        self.game.mkdir()
        (self.game / 'game.exe').write_bytes(b'fixture')
        (self.game / 'opaque.arc').write_bytes(b'opaque archive fixture')
        (self.game / 'chapter.bin').write_bytes(b'Current')
        self.agent = host.Agent(dict(game=str(self.game), save=str(self.game), skill=str(self.skill),
            work=str(self.game / '.work'), language='en-US'))
        self.agent.read_skill = self.agent.read_contract = True

    def write(self, relative, text):
        path = self.skill / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding='utf-8')
        return path

    def catalog(self, relative, reference, **extra):
        self.write(reference, 'General version-sensitive format reference.')
        return self.write(relative, json.dumps([dict(id='fixture', extensions=['.bin'], reference=reference, **extra)]))

    def test_new_catalog_and_engine_paths_take_precedence_over_legacy_files(self):
        self.catalog('reference/format-index.json', 'reference/old.md')
        current = self.catalog('catalog/formats.json', 'engine/current.md')
        self.assertEqual(workflow['format_catalog_path'](self.skill), current)
        hints = workflow['reference_hints'](self.skill, [dict(path='scripts/CHAPTER.BIN')])
        self.assertEqual([item['path'] for item in hints], ['engine/current.md'])
        current.write_text('{broken', encoding='utf-8')
        self.assertEqual(workflow['reference_hints'](self.skill, [dict(path='chapter.bin')]), [])

    def test_new_catalog_participates_in_checkpoint_and_test_cache_identity(self):
        legacy = self.catalog('reference/format-index.json', 'reference/old.md')
        current = self.catalog('catalog/formats.json', 'engine/current.md')
        self.agent.programs['script'] = 'def extract(data, metadata): return []'
        args = dict(kind='script', root='game', path='chapter.bin')
        initial_context = self.agent.context()
        initial_signature = self.agent.test_signature(args)
        legacy.write_text('[]', encoding='utf-8')
        self.assertEqual(self.agent.context(), initial_context)
        self.assertEqual(self.agent.test_signature(args), initial_signature)
        current.write_text('[]', encoding='utf-8')
        self.assertNotEqual(self.agent.context(), initial_context)
        self.assertNotEqual(self.agent.test_signature(args), initial_signature)

    def test_new_and_legacy_catalog_policies_follow_the_same_selected_path(self):
        policy = dict(header=dict(offset=0, ascii='MAGC'), chunks=dict(offset=4,
            tag_width=4, length_width=2, endian='little', tag_ascii='TABL', max_chunks=2))
        self.catalog('reference/format-index.json', 'reference/old.md', verification_policy=policy)
        data = b'MAGCTABL\x03\x00raw'
        self.assertEqual(workflow['required_table_regions'](self.skill, data), [dict(offset=10, length=3)])
        self.catalog('catalog/formats.json', 'engine/current.md')
        self.assertEqual(workflow['required_table_regions'](self.skill, data), [])
        self.catalog('catalog/formats.json', 'engine/current.md', verification_policy=policy)
        self.assertEqual(workflow['required_table_regions'](self.skill, data), [dict(offset=10, length=3)])

    def test_garbro_requires_complete_current_cli_document_even_for_a_cached_listing(self):
        document = self.write('commands/garbro.md', 'General CLI reference.\n' * 600)
        self.write('reference/GARbro-CLI.md', 'Obsolete CLI reference.')
        args = dict(command='l', archive='opaque.arc')
        completed = mock.Mock(returncode=0, stdout=b'[00000000] 7 chapter.bin', stderr=b'')
        with mock.patch.object(host.subprocess, 'run', return_value=completed) as process:
            with self.assertRaisesRegex(ValueError, 'commands/garbro.md'):
                self.agent.dispatch('run_skill', args)
            self.agent.dispatch('read_text', dict(root='skill', path='reference/GARbro-CLI.md'))
            with self.assertRaisesRegex(ValueError, 'complete current'):
                self.agent.dispatch('run_skill', args)
            page = self.agent.dispatch('read_text', dict(root='skill', path='commands/garbro.md'))
            self.assertIsNotNone(page['next_offset'])
            with self.assertRaisesRegex(ValueError, 'follow next_offset'):
                self.agent.dispatch('run_skill', args)
            process.assert_not_called()
            self.agent.dispatch('read_text', dict(root='skill', path='commands/garbro.md', offset=page['next_offset']))
            self.assertEqual(self.agent.dispatch('run_skill', args)['exit_code'], 0)
            self.assertTrue(self.agent.dispatch('run_skill', args)['reused'])
            document.write_text('Updated CLI reference.', encoding='utf-8')
            with self.assertRaisesRegex(ValueError, 'complete current'):
                self.agent.dispatch('run_skill', args)
            self.agent.dispatch('read_text', dict(root='skill', path='commands/garbro.md'))
            self.assertTrue(self.agent.dispatch('run_skill', args)['reused'])
            process.assert_called_once()

    def test_legacy_cli_document_remains_supported(self):
        self.write('reference/GARbro-CLI.md', 'General legacy CLI reference.')
        self.assertEqual(workflow['skill_document_paths'](self.skill)['garbro'], 'reference/GARbro-CLI.md')
        self.agent.dispatch('read_text', dict(root='skill', path='reference/GARbro-CLI.md'))
        with mock.patch.object(host.subprocess, 'run', return_value=mock.Mock(returncode=0, stdout=b'', stderr=b'')):
            self.assertEqual(self.agent.dispatch('run_skill', dict(command='i', archive='opaque.arc'))['exit_code'], 0)

    def test_installed_memory_helper_supports_all_host_actions_in_work_local_state(self):
        repository = pathlib.Path(__file__).resolve().parents[4]
        helper = repository / 'skills/GalgameResourceExtract/scripts/memory.py'
        self.assertTrue(helper.is_file())
        installed = self.skill / 'scripts/memory.py'
        installed.parent.mkdir()
        shutil.copyfile(helper, installed)
        self.assertEqual(self.agent.dispatch('project_memory', dict(action='configure'))['exit_code'], 0)
        self.assertEqual(self.agent.dispatch('project_memory', dict(action='observe', sources=['chapter.bin']))['exit_code'], 0)
        self.agent.dispatch('project_memory', dict(action='update', patch=dict(notes=['Observed fixture only.'])))
        artifact = self.agent.work / 'fixture.json'
        artifact.write_text('{}', encoding='utf-8')
        self.agent.dispatch('project_memory', dict(action='checkpoint', stage='fixture', status='completed',
            notes='Temporary fixture checkpoint.', artifacts=['fixture.json']))
        read = self.agent.dispatch('project_memory', dict(action='read'))
        self.assertEqual(read['result']['notes'], ['Observed fixture only.'])
        self.assertEqual(read['result']['checkpoints']['fixture']['status'], 'completed')
        self.assertTrue(self.agent.dispatch('project_memory', dict(action='check'))['result']['passed'])
        artifact.write_text('{"changed":true}', encoding='utf-8')
        changed = self.agent.dispatch('project_memory', dict(action='check'))
        self.assertEqual(changed['exit_code'], 1)
        self.assertFalse(changed['result']['passed'])
        self.assertTrue((self.agent.work / 'project-memory.json').is_file())
        self.assertFalse((self.skill / 'memory.json').exists())


if __name__ == '__main__':
    unittest.main()
