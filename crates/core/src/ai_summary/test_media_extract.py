"""独立媒体任务回归；只使用临时资源、模拟模型及随附离线转换器。"""
import hashlib
import json
import os
from pathlib import Path
import runpy
import subprocess
import tempfile
import unittest
from unittest.mock import patch

BASE = Path(__file__).parent
API = runpy.run_path(str(BASE / 'agent_host.py'))
MEDIA = runpy.run_path(str(BASE / 'media_agent.py'))
FILES = MEDIA['FILES']


class MediaTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.game, self.skill = self.root / 'game', self.root / 'skill'
        self.game.mkdir()
        self.skill.mkdir()
        (self.game/'game.exe').write_bytes(b'MZ fixture only')
        (self.skill/'SKILL.md').write_text('Installed extraction skill fixture', encoding='utf-8')
        self.config = dict(game=str(self.game), save=str(self.game), skill=str(self.skill),
            work=str(self.game/'.galgear-ai-summary'/'media-fixture'), kind='audio',
            provider='deepseek', model='fixture', token='never-sent', title='fixture', language='zh-CN')
        self.agent = MEDIA['MediaAgent'](self.config, API)

    def finish(self):
        return self.agent.dispatch('finish_media', dict(scope='Fixture audio selection', warnings=[]))

    def test_audio_preserves_paths_sources_and_repeat_exports(self):
        for folder, content in [('voice', b'OggS voice'), ('music', b'OggS music')]:
            directory = self.game/folder/'nested'
            directory.mkdir(parents=True)
            (directory/'same.ogg').write_bytes(content)
            self.agent.dispatch('export_media', dict(root='game', path=folder, pattern='**/*.ogg'))
        self.agent.dispatch('export_media', dict(root='game', path='voice', pattern='**/*.ogg'))
        self.assertEqual(self.finish()['files'], 2)
        manifest = json.loads((self.agent.delivery/'manifest.json').read_text('utf-8'))
        self.assertEqual({(self.agent.delivery/e['path']).read_bytes() for e in manifest['files']}, {b'OggS voice', b'OggS music'})
        self.assertTrue(all(e['path'].endswith('nested/same.ogg') for e in manifest['files']))
        self.assertEqual((self.game/'voice/nested/same.ogg').read_bytes(), b'OggS voice')

    def test_empty_wrong_kind_and_non_media_do_not_succeed(self):
        with self.assertRaisesRegex(ValueError, 'No media'):
            self.finish()
        (self.game/'cg.png').write_bytes(b'image')
        for path in ['game.exe', 'cg.png']:
            with self.assertRaisesRegex(ValueError, 'unrecognized media'):
                self.agent.dispatch('export_media', dict(root='game', path=path))
        self.assertFalse((self.agent.delivery/'manifest.json').exists())

    def test_changed_source_prevents_completion(self):
        path = self.game/'voice.ogg'
        path.write_bytes(b'OggS original')
        self.agent.dispatch('export_media', dict(root='game', path=path.name))
        path.write_bytes(b'changed')
        with self.assertRaisesRegex(ValueError, 'changed'):
            self.finish()

    def test_absolute_traversal_and_other_task_inputs_are_rejected(self):
        for path in ['../secret.ogg', str(self.root/'secret.ogg'), 'C:/secret.ogg', '.galgear-ai-summary/media-fixture/delivery']:
            with self.subTest(path=path), self.assertRaises(ValueError):
                self.agent.dispatch('export_media', dict(root='game', path=path))

    @unittest.skipUnless(os.name == 'nt', 'Windows junction boundary')
    def test_junction_cannot_export_outside_game(self):
        outside = self.root/'outside'
        outside.mkdir()
        (outside/'voice.ogg').write_bytes(b'private')
        junction = self.game/'linked'
        subprocess.run(['cmd', '/c', 'mklink', '/J', str(junction), str(outside)], check=True, capture_output=True)
        self.addCleanup(lambda: junction.rmdir())
        with self.assertRaisesRegex(ValueError, 'reparse'):
            self.agent.dispatch('export_media', dict(root='game', path='linked', pattern='*.ogg'))

    def test_model_loop_uses_installed_skill_and_only_actual_output_can_finish(self):
        for index in range(8):
            (self.game/('voice'+str(index)+'.ogg')).write_bytes(b'OggS'+bytes([index])*32)
        responses = [
            [('test_extraction', dict(mode='files',path='.',pattern='*.ogg'))],
            [('start_batch', dict(tests=['sample-1'],scope='本地音频',warnings=[]))],
        ]
        def complete(config, messages, available, **kwargs):
            self.assertIn('Installed extraction skill fixture', messages[1]['content'])
            self.assertNotIn('never-sent', json.dumps(messages))
            return dict(message=dict(role='assistant', content=None, tool_calls=[dict(id='c'+str(i),type='function',function=dict(name=name,arguments=json.dumps(args))) for i,(name,args) in enumerate(responses.pop(0))]))
        with patch.dict(API['TRANSPORT'], complete=complete):
            result = self.agent.run()
        self.assertEqual(result['tested_samples'], 3)
        self.assertEqual(len(self.agent.files), 3)
        self.assertFalse((self.agent.work/'delivery').exists())
        self.assertEqual(json.loads((self.agent.work/'session.json').read_text())['status'], 'completed')
        self.assertNotIn('never-sent', (self.agent.work/'conversation.json').read_text('utf-8'))
        batch=runpy.run_path(str(BASE/'media_batch.py'))
        config={key:value for key,value in self.config.items() if key not in ('token','provider','model')}
        with patch.dict(API['TRANSPORT'],complete=lambda *args,**kwargs: self.fail('Batch script must never call AI')):
            result=batch['run'](config,API)
        self.assertEqual(result['files'],8)
        progress=json.loads((self.agent.work/'progress.json').read_text())
        self.assertEqual(progress['batch'],dict(completed=1,total=1,files=8))
        self.assertEqual(json.loads((self.agent.work/'ai-session.json').read_text())['requests'],2)

    def test_untested_recipe_cannot_start_batch_and_batch_rejects_credentials(self):
        with self.assertRaisesRegex(ValueError,'successful test'):
            self.agent.dispatch('start_batch',dict(tests=['sample-1'],scope='test',warnings=[]))
        batch=runpy.run_path(str(BASE/'media_batch.py'))
        with self.assertRaisesRegex(ValueError,'credentials'):
            batch['run'](self.config,API)

    def test_converter_partial_failures_are_retained_even_if_model_omits_them(self):
        self.agent.kind = 'cg'
        (self.game/'cg.native').write_bytes(b'native fixture')
        executable = self.skill/'tools/GARbro-cli/GalImageExport.exe'
        executable.parent.mkdir(parents=True)
        executable.write_bytes(b'fixture')
        guide = self.skill/'commands/garbro-images.md'
        guide.parent.mkdir()
        guide.write_text('fixture guide')
        self.agent.dispatch('read_text', dict(root='skill',path='commands/garbro-images.md'))
        def convert(command, **kwargs):
            job = json.loads(Path(command[1]).read_text())
            output = Path(job['output'])
            output.mkdir()
            native = output/'cg.native'
            native.write_bytes(b'native fixture')
            entries = [dict(kind='raw',path=str(native),sha256=hashlib.sha256(native.read_bytes()).hexdigest()), dict(kind='decode_error',name='cg.native',error='unsupported fixture')]
            (output/'manifest.jsonl').write_text('\n'.join(json.dumps(e) for e in entries))
            return subprocess.CompletedProcess(command, 0, b'', b'')
        with patch.object(FILES['subprocess'], 'run', side_effect=convert):
            result = self.agent.dispatch('convert_images', dict(root='game',path='cg.native',mode='files'))
        self.assertEqual(result['failed'], 1)
        self.finish()
        self.assertEqual(len(json.loads((self.agent.delivery/'manifest.json').read_text())['warnings']), 1)

    @unittest.skipUnless(os.name == 'nt', 'Bundled Windows converter')
    def test_bundled_image_converter_exports_real_png_without_model_or_network(self):
        skill = BASE.parents[3]/'Galgear-GalgameReverse-Skill/GalgameResourceExtract'
        if not (skill/'tools/GARbro-cli/GalImageExport.exe').is_file():
            self.skipTest('Bundled image converter not installed')
        self.agent.kind = 'cg'
        self.agent.host.roots['skill'] = skill.resolve()
        path = self.game/'sample.png'
        # Valid 1x1 RGBA PNG; all files are temporary and unrelated to a real game.
        import struct
        import zlib
        def chunk(name, data):
            return struct.pack('>I',len(data))+name+data+struct.pack('>I',zlib.crc32(name+data))
        png = b'\x89PNG\r\n\x1a\n'+chunk(b'IHDR',struct.pack('>IIBBBBB',1,1,8,6,0,0,0))+chunk(b'IDAT',zlib.compress(b'\0\xff\0\0\xff'))+chunk(b'IEND',b'')
        path.write_bytes(png)
        self.agent.dispatch('read_text',dict(root='skill',path='commands/garbro-images.md'))
        result = self.agent.dispatch('convert_images',dict(root='game',path=path.name,mode='files'))
        self.assertEqual(result['failed'], 0)
        self.assertGreaterEqual(result['files'], 2)
        self.finish()
        self.assertEqual(path.read_bytes(), png)


if __name__ == '__main__':
    unittest.main()
